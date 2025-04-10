use std::{
    cell::OnceCell,
    env::var,
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::OnceLock,
};

use axum::extract::multipart::Field;
use chrono::Local;
use image::{EncodableLayout, ImageFormat};
use mime_guess::mime::IMAGE_PNG;
use mongodb::bson::{Document, doc};
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
};

use crate::{
    common::{
        constant::{PUBLIC_TENANT, THUMB_HEIGHT, THUMB_WIDTH},
        domain::ServiceError,
        user_header::ExtractUserInfo,
        util::StoreCollection,
    },
    store::{Repository, StoreClient, StoreRepository},
    upload::soffice::{ConvertType, convert_to},
};

use super::domain::FileUpload;

pub struct FileService<'a> {
    pub share_drive_path: &'a str,
    pub store: &'a StoreRepository<FileUpload>,
}

static THUMB_W: OnceLock<u32> = OnceLock::new();
static THUMB_H: OnceLock<u32> = OnceLock::new();

pub fn get_thumb_width() -> u32 {
    *THUMB_W.get_or_init(|| {
        var(THUMB_WIDTH)
            .ok()
            .and_then(|a| a.parse::<u32>().ok())
            .unwrap_or(300)
    })
}
pub fn get_thumb_height() -> u32 {
    *THUMB_H.get_or_init(|| {
        var(THUMB_HEIGHT)
            .ok()
            .and_then(|a| a.parse::<u32>().ok())
            .unwrap_or(300)
    })
}
impl FileService<'_> {
    pub async fn get_file_upload(
        id: &str,
        tenant: Option<String>,
        client: &StoreClient,
        collection: &StoreCollection,
    ) -> Option<(StoreRepository<FileUpload>, FileUpload)> {
        async fn get_upload(
            repository: &StoreRepository<FileUpload>,
            id: &str,
        ) -> Option<FileUpload> {
            match repository.find_by_id(id).await {
                Ok(Some(response)) => Some(response),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("db error {e}");
                    None
                }
            }
        }

        let public_repository: StoreRepository<FileUpload> =
            StoreRepository::get_repository(client.clone(), &collection.0, PUBLIC_TENANT).await;

        if let Some(fu) = get_upload(&public_repository, id).await {
            Some((public_repository, fu))
        } else if let Some(tenant) = tenant {
            let private_repository: StoreRepository<FileUpload> =
                StoreRepository::get_repository(client.clone(), &collection.0, &tenant).await;
            get_upload(&private_repository, id)
                .await
                .map(|fu| (private_repository, fu))
        } else {
            None
        }
    }
    fn get_physical_path(&self, internal_name: &str) -> PathBuf {
        PathBuf::from(self.share_drive_path).join(internal_name)
    }

    async fn make_thumbnail(
        &self,
        upl: &FileUpload,
        internal_name: &str,
        temp_file_path: &PathBuf,
    ) -> Result<Option<String>, ServiceError> {
        let (extension, thumb) = {
            let (ct, image) = if !upl.is_image() {
                match convert_to(temp_file_path, ConvertType::Png).await {
                    Ok(bytes) => image::load_from_memory(&bytes)
                        .map_err(|e| ServiceError::from(&e))
                        .map(|im| (Some(IMAGE_PNG.to_string()), im)),
                    Err(e) => {
                        tracing::error!("error converting file {}: {} ", upl.original_filename, e);
                        return Ok(None);
                    }
                }
            } else {
                let bytes = tokio::fs::read(temp_file_path)
                    .await
                    .map_err(|e| ServiceError::from(&e))?;

                image::load_from_memory(&bytes)
                    .map_err(|e| ServiceError::from(&e))
                    .map(|im| (upl.content_type.clone(), im))
            }?;
            let thumb = image.thumbnail(get_thumb_width(), get_thumb_height());

            let Some(ct) = ct else {
                return Err(ServiceError("No Content type! Should not happen".into()));
            };

            let Some(image_format) = ImageFormat::from_mime_type(ct) else {
                return Err(ServiceError(
                    "Format cannot be transformed to thumbnail".into(),
                ));
            };

            tracing::debug!("generate thumbnail...");

            let mut cursor = Cursor::new(Vec::new());

            thumb
                .write_to(&mut cursor, image_format)
                .map_err(|e| ServiceError(format!("{e}")))?;
            cursor
                .seek(SeekFrom::Start(0))
                .map_err(|e| ServiceError(format!("{e}")))?;

            let mut thumb = Vec::new();

            cursor
                .read_to_end(&mut thumb)
                .map_err(|e| ServiceError(format!("{e}")))?;
            (image_format.extensions_str().join("."), thumb)
        };

        let thumb_filename = if Some(&extension) != upl.extension.as_ref() {
            format!("thumb-{internal_name}.{extension}")
        } else {
            format!("thumb-{internal_name}")
        };
        let thumbnail = FileUpload {
            content_type: mime_guess::from_ext(&extension)
                .first_raw()
                .map(|m| m.into()),
            thumbnail_id: None,
            original_filename: thumb_filename.clone(),
            internal_name: thumb_filename,
            extension: Some(extension),
            size: thumb.len() as u64,
            public_resource: upl.public_resource,
            correlation_id: Some(upl.id.clone()),
            ..Default::default()
        };

        let path_buf = PathBuf::from(&self.share_drive_path).join(&thumbnail.internal_name);
        tracing::debug!("save thumbnail... {path_buf:?}");

        tokio::fs::write(path_buf, thumb.as_bytes())
            .await
            .map_err(|e| ServiceError::from(&e))?;

        self.store
            .update(&thumbnail.id, &thumbnail)
            .await
            .map_err(|e| ServiceError::from(&e))?;
        Ok(Some(thumbnail.id))
    }

    pub async fn upload(
        &self,
        mut upl: FileUpload,
        temp_file_path: Option<&PathBuf>,
    ) -> Result<FileUpload, ServiceError> {
        if let Some(temp_file_path) = temp_file_path {
            let upload = self
                .store
                .find_by_id(&upl.id)
                .await
                .map_err(|e| ServiceError::from(&e))?;
            let (old_internal_name, old_thumbnail_id) = if let Some(upload) = upload {
                (Some(upload.internal_name), upload.thumbnail_id)
            } else {
                (None, None)
            };
            let extension = &upl.extension;
            let internal_name = format!(
                "{}.{}",
                upl.id,
                extension.as_ref().cloned().unwrap_or_else(|| "".into())
            );

            if let Some(old_internal_name) = old_internal_name {
                upl.updated_date = Some(Local::now().to_utc());
                // override file
                tracing::info!("removing old file {}", old_internal_name);
                if let Err(e) = tokio::fs::remove_file(
                    PathBuf::from(self.share_drive_path).join(&old_internal_name),
                )
                .await
                {
                    tracing::error!("could not remove old file: {e}");
                }
                if let Some(old_thumbnail_id) = old_thumbnail_id {
                    self.store
                        .delete_by_id(&old_thumbnail_id)
                        .await
                        .map_err(|e| ServiceError::from(&e))?;

                    tracing::info!("removing old thumbnail {}", old_thumbnail_id);
                    if let Err(e) = tokio::fs::remove_file(
                        PathBuf::from(&self.share_drive_path)
                            .join(format!("thumb-{old_internal_name}")),
                    )
                    .await
                    {
                        tracing::error!("could not remove old thumbnail: {e}");
                    }
                }
            }

            upl.thumbnail_id = self
                .make_thumbnail(&upl, &internal_name, temp_file_path)
                .await?;

            tokio::fs::rename(
                temp_file_path,
                PathBuf::from(&self.share_drive_path).join(&internal_name),
            )
            .await
            .map_err(|e| ServiceError::from(&e))?;
            upl.internal_name = internal_name;
        }

        self.store
            .update(&upl.id, &upl)
            .await
            .map_err(|e| ServiceError::from(&e))?;
        Ok(upl)
    }
    pub async fn delete_by(&self, query: Document) -> Result<(), ServiceError> {
        let upls = self
            .store
            .find_by_query(query, None)
            .await
            .map_err(|e| ServiceError::from(&e))?;
        for upl in upls {
            self.store
                .delete_by_id(&upl.id)
                .await
                .map_err(|e| ServiceError::from(&e))?;
            if let Err(e) = tokio::fs::remove_file(self.get_physical_path(&upl.internal_name)).await
            {
                tracing::error!("could not delete file {upl:?} => {e}");
            };
            if let Some(thumb_id) = &upl.thumbnail_id {
                if let Ok(Some(thumb)) = self.store.find_by_id(thumb_id).await {
                    self.store
                        .delete_by_id(&thumb.id)
                        .await
                        .map_err(|e| ServiceError::from(&e))?;
                    if let Err(e) =
                        tokio::fs::remove_file(self.get_physical_path(&thumb.internal_name)).await
                    {
                        tracing::error!("could not delete thumb file {upl:?} => {e}");
                    };
                }
            }
        }
        Ok(())
    }
    pub async fn delete_by_correlation_id(&self, id: &str) -> Result<(), ServiceError> {
        self.delete_by(doc! {"correlationId": id}).await
    }
    pub async fn delete_by_id(&self, id: &str) -> Result<(), ServiceError> {
        self.delete_by(doc! {"_id": id}).await
    }
    pub async fn download(&self, upl: &FileUpload) -> Result<File, ServiceError> {
        tokio::fs::File::open(self.get_physical_path(&upl.internal_name))
            .await
            .map_err(|e| ServiceError::from(&e))
    }
    pub async fn download_bytes(&self, upl: &FileUpload) -> Result<Vec<u8>, ServiceError> {
        use io::AsyncReadExt;
        let mut download = self.download(upl).await?;
        let mut bytes = Vec::with_capacity(1024);
        download
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| ServiceError(format!("{e}")))?;
        Ok(bytes)
    }
}
pub async fn write_field_to_temp_file(
    field: &mut Field<'_>,
    volume: impl Into<PathBuf>,
    file_name: &str,
) -> (PathBuf, u64) {
    let volume = volume.into();
    let temp_volume = volume.join("tmp"); // necessary to
    // then move the file in the same volume
    tracing::debug!("temp_volume: - {temp_volume:?}");
    if !temp_volume.exists() {
        tokio::fs::create_dir(&temp_volume).await.unwrap();
    }
    let temp_file_path = temp_volume.join(file_name);
    if temp_file_path.exists() {
        tracing::info!(
            "file {file_name} exists. removing: {:?}",
            tokio::fs::remove_file(&temp_file_path).await
        );
    }

    let mut temp_file = {
        let mut o = tokio::fs::OpenOptions::new();
        o.append(true).create(true).open(&temp_file_path).await
    }
    .unwrap();

    while let Ok(Some(chunk)) = field.chunk().await {
        temp_file.write_all(&chunk).await.unwrap();
    }
    let metadata = temp_file.metadata().await.unwrap();
    (temp_file_path, metadata.len())
}
