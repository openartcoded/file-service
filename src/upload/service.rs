use std::{
    error::Error,
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
};

use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
use axum::extract::multipart::Field;
use futures::TryStreamExt;
use image::{EncodableLayout, ImageFormat};
use mime_guess::mime::IMAGE_PNG;
use mongodb::bson::{Document, doc};
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
};
use tokio_util::io::StreamReader;
use tracing::debug;

use crate::upload::ConvertType;
use crate::{
    common::{
        constant::{SHARE_DRIVE_PATH_BUF, THUMB_H, THUMB_W, TMP_FS_PATH},
        domain::ServiceError,
        util::{IdGenerator, StoreCollection},
    },
    store::{Repository, StoreClient, StoreRepository, get_document_filter_by_maybe_object_id},
};

use super::domain::FileUploadV2;

#[derive(Clone)]
pub struct FileService {
    pub store: StoreRepository<FileUploadV2>,
}

impl FileService {
    pub async fn get_file_upload(
        id: &str,
        tenant: Option<String>,
        client: &StoreClient,
        collection: &StoreCollection,
    ) -> Option<(StoreRepository<FileUploadV2>, FileUploadV2)> {
        async fn get_upload(
            repository: &StoreRepository<FileUploadV2>,
            id: &str,
        ) -> Option<FileUploadV2> {
            match repository.find_by_id(id).await {
                Ok(Some(response)) => Some(response),
                Ok(None) => {
                    tracing::debug!("could not find file with id {id}");
                    None
                }
                Err(e) => {
                    tracing::error!("db error {e}");
                    None
                }
            }
        }

        if let Some(tenant) = tenant {
            let private_repository: StoreRepository<FileUploadV2> =
                StoreRepository::get_repository(client, &collection.0, &tenant);
            get_upload(&private_repository, id)
                .await
                .map(|fu| (private_repository, fu))
        } else {
            None
        }
    }
    pub fn get_physical_path(&self, internal_name: &str) -> PathBuf {
        SHARE_DRIVE_PATH_BUF.join(internal_name)
    }

    pub async fn make_thumbnail(
        &self,
        upl: &FileUploadV2,
        internal_name: &str,
        temp_file_path: &PathBuf,
    ) -> Result<Option<String>, ServiceError> {
        let (extension, thumb) = {
            let (ct, image) = if upl.is_supported_image() {
                let bytes = tokio::fs::read(temp_file_path)
                    .await
                    .map_err(ServiceError::new)?;

                image::load_from_memory(&bytes)
                    .map_err(ServiceError::new)
                    .map(|im| (upl.content_type.clone(), im))
            } else {
                match super::imagemagick::convert_to(temp_file_path, ConvertType::Png)
                    .await
                    .map_err(ServiceError::new)
                    .map(|bytes| {
                        image::load_from_memory(&bytes)
                            .map_err(ServiceError::new)
                            .map(|im| (Some(IMAGE_PNG.to_string()), im))
                    }) {
                    Ok(img) => img,
                    Err(e) => {
                        tracing::error!(
                            "error converting file {}: {}, trying with soffice...",
                            internal_name,
                            e
                        );
                        match super::soffice::convert_to(temp_file_path, ConvertType::Png)
                            .await
                            .map_err(ServiceError::new)
                            .map(|bytes| {
                                image::load_from_memory(&bytes)
                                    .map_err(ServiceError::new)
                                    .map(|im| (Some(IMAGE_PNG.to_string()), im))
                            }) {
                            Ok(img) => img,
                            Err(e) => {
                                tracing::error!(
                                    "error converting file {}: {}, giving up...",
                                    internal_name,
                                    e
                                );
                                return Ok(None);
                            }
                        }
                    }
                }
            }?;

            let thumb = image.thumbnail(*THUMB_W, *THUMB_H);

            let Some(ct) = ct else {
                return Err(ServiceError::new("No Content type! Should not happen"));
            };

            let Some(image_format) = ImageFormat::from_mime_type(ct) else {
                return Err(ServiceError::new(
                    "Format cannot be transformed to thumbnail",
                ));
            };

            tracing::debug!("generate thumbnail...");

            let mut cursor = Cursor::new(Vec::new());

            thumb
                .write_to(&mut cursor, image_format)
                .map_err(ServiceError::new)?;
            cursor.seek(SeekFrom::Start(0)).map_err(ServiceError::new)?;

            let mut thumb = Vec::new();

            cursor.read_to_end(&mut thumb).map_err(ServiceError::new)?;
            (image_format.extensions_str().join("."), thumb)
        };

        let thumb_filename = if Some(&extension) != upl.extension.as_ref() {
            format!("thumb-{internal_name}.{extension}")
        } else {
            format!("thumb-{internal_name}")
        };
        let thumbnail = FileUploadV2 {
            content_type: mime_guess::from_ext(&extension)
                .first_raw()
                .map(|m| m.into()),
            thumbnail_id: None,
            original_filename: thumb_filename.clone(),
            bookmarked: Some(false),
            thumb: Some(true),
            name: Some(thumb_filename),
            extension: Some(extension),
            size: thumb.len() as u64,
            public_resource: upl.public_resource,
            correlation_id: Some(upl.id.clone()),
            ..Default::default()
        };

        let path_buf = SHARE_DRIVE_PATH_BUF.join(self.get_filename_on_disk(&thumbnail));
        tracing::debug!("save thumbnail... {path_buf:?}");

        tokio::fs::write(path_buf, thumb.as_bytes())
            .await
            .map_err(ServiceError::new)?;

        self.store
            .upsert(&thumbnail.id, &thumbnail)
            .await
            .map_err(ServiceError::new)?;
        Ok(Some(thumbnail.id))
    }

    pub async fn upload(
        &self,
        mut upl: FileUploadV2,
        temp_file_path: Option<&PathBuf>,
        without_thumbnail: bool,
    ) -> Result<FileUploadV2, ServiceError> {
        let final_file = if let Some(temp_file_path) = temp_file_path {
            let upload = self
                .store
                .find_by_id(&upl.id)
                .await
                .map_err(ServiceError::new)?;
            let (old_internal_name, old_thumbnail_id) = if let Some(upload) = upload {
                (
                    Some(self.get_filename_on_disk(&upload)),
                    upload.thumbnail_id,
                )
            } else {
                (None, None)
            };
            let internal_name = self.get_filename_on_disk(&upl);
            if let Some(old_internal_name) = old_internal_name {
                upl.updated_date = Some(bson::DateTime::now());
                // override file
                tracing::info!("removing old file {}", old_internal_name);
                if let Err(e) =
                    tokio::fs::remove_file(SHARE_DRIVE_PATH_BUF.join(&old_internal_name)).await
                {
                    tracing::error!("could not remove old file: {e}");
                }
                if let Some(old_thumbnail_id) = old_thumbnail_id {
                    self.store
                        .delete_by_id(&old_thumbnail_id)
                        .await
                        .map_err(ServiceError::new)?;

                    tracing::info!("removing old thumbnail {}", old_thumbnail_id);
                    if let Err(e) = tokio::fs::remove_file(
                        SHARE_DRIVE_PATH_BUF.join(format!("thumb-{old_internal_name}")),
                    )
                    .await
                    {
                        tracing::error!("could not remove old thumbnail: {e}");
                    }
                }
            }
            let final_file_path = SHARE_DRIVE_PATH_BUF.join(&internal_name);
            match tokio::fs::rename(&temp_file_path, &final_file_path)
                .await
                .map_err(ServiceError::new)
            {
                Ok(_) => (),
                Err(e) => {
                    tracing::error!(
                        "we couldn't simply rename files, could be related to os error 18: {e}"
                    );
                    tracing::info!("let's try with just copy and delete then");
                    tracing::info!("copy...");
                    tokio::fs::copy(&temp_file_path, &final_file_path)
                        .await
                        .map_err(ServiceError::new)?;
                    tracing::info!("delete...");
                    tokio::fs::remove_file(&temp_file_path)
                        .await
                        .map_err(ServiceError::new)?;
                }
            }
            upl.name = Some(internal_name.clone());
            Some((internal_name, final_file_path))
        } else {
            None
        };

        self.store
            .upsert(&upl.id, &upl)
            .await
            .map_err(ServiceError::new)?;

        tracing::info!(
            "we now try to make a thumb,without_thumbnail:{without_thumbnail}, final_file: {final_file:?}, checking condition..."
        );
        if let Some((internal_name, final_file_path)) = final_file
            && !without_thumbnail
        {
            tracing::info!("condition matches! making a thumb");
            let mut upl = upl.clone();
            let that = self.clone();
            // make thumb generation asynchronous
            tokio::spawn(async move {
                match that
                    .make_thumbnail(&upl, &internal_name, &final_file_path)
                    .await
                {
                    Err(err) => {
                        tracing::error!("could not generate thumbnail for upl {upl:?}, {err}")
                    }
                    Ok(o) => {
                        tracing::info!("new thumb: {o:?}");
                        upl.thumbnail_id = o;
                        if let Err(err) = that
                            .store
                            .upsert(&upl.id, &upl)
                            .await
                            .map_err(ServiceError::new)
                        {
                            tracing::error!(
                                "could not save {upl:?} after generating thumbnail {err}"
                            );
                        }
                    }
                }
            });
        }

        Ok(upl)
    }
    pub async fn delete_by(&self, query: Document) -> Result<(), ServiceError> {
        let upls = self
            .store
            .find_by_query(query, None)
            .await
            .map_err(ServiceError::new)?;
        for upl in upls {
            self.store
                .delete_by_id(&upl.id)
                .await
                .map_err(ServiceError::new)?;
            if let Err(e) =
                tokio::fs::remove_file(self.get_physical_path(&self.get_filename_on_disk(&upl)))
                    .await
            {
                tracing::error!("could not delete file {upl:?} => {e}");
            };
            if let Some(thumb_id) = &upl.thumbnail_id
                && let Ok(Some(thumb)) = self.store.find_by_id(thumb_id).await
            {
                self.store
                    .delete_by_id(&thumb.id)
                    .await
                    .map_err(ServiceError::new)?;
                if let Err(e) = tokio::fs::remove_file(
                    self.get_physical_path(&self.get_filename_on_disk(&thumb)),
                )
                .await
                {
                    tracing::error!("could not delete thumb file {upl:?} => {e}");
                };
            }
        }
        Ok(())
    }
    pub fn get_filename_on_disk(&self, file_upload: &FileUploadV2) -> String {
        format!(
            "{}.{}",
            file_upload.id,
            file_upload.extension.clone().unwrap_or_else(|| "".into())
        )
    }
    pub async fn delete_by_correlation_id(&self, id: &str) -> Result<(), ServiceError> {
        self.delete_by(doc! {"correlationId": id}).await
    }
    pub async fn delete_by_id(&self, id: &str) -> Result<(), ServiceError> {
        self.delete_by(get_document_filter_by_maybe_object_id(id))
            .await
    }

    pub async fn download(&self, upl: &FileUploadV2) -> Result<File, ServiceError> {
        let path = self.get_physical_path(&self.get_filename_on_disk(upl));
        tracing::info!("downloading {path:?}");
        tokio::fs::File::open(path).await.map_err(ServiceError::new)
    }
    pub async fn download_bulk(
        &self,
        upls: &[FileUploadV2],
    ) -> Result<(File, PathBuf), ServiceError> {
        use tokio::io::AsyncReadExt;
        let zip_path = TMP_FS_PATH.join(format!("{}.zip", IdGenerator.get()));
        let mut zip = tokio::fs::File::create(&zip_path)
            .await
            .map_err(ServiceError::new)?;
        let mut writer = ZipFileWriter::with_tokio(&mut zip);
        for upl in upls {
            let mut file = self.download(upl).await?;
            let mut data = Vec::new();
            file.read_to_end(&mut data)
                .await
                .map_err(ServiceError::new)?;
            let builder = ZipEntryBuilder::new(
                upl.original_filename.to_string().into(),
                Compression::Deflate,
            );
            writer
                .write_entry_whole(builder, &data)
                .await
                .map_err(ServiceError::new)?;
        }
        writer.close().await.map_err(ServiceError::new)?;
        let file = tokio::fs::File::open(&zip_path)
            .await
            .map_err(ServiceError::new)?;

        Ok((file, zip_path))
    }

    pub async fn download_bytes(&self, upl: &FileUploadV2) -> Result<Vec<u8>, ServiceError> {
        use io::AsyncReadExt;
        let mut download = self.download(upl).await?;
        let mut bytes = Vec::with_capacity(1024);
        download
            .read_to_end(&mut bytes)
            .await
            .map_err(ServiceError::new)?;
        Ok(bytes)
    }
}
pub async fn write_field_to_temp_file(
    field: &mut Field<'_>,
    volume: impl Into<PathBuf>,
    file_name: &str,
) -> Result<(PathBuf, u64), Box<dyn Error>> {
    let temp_volume = volume.into();
    tracing::debug!("temp_volume: - {temp_volume:?}");
    if !temp_volume.exists() {
        tokio::fs::create_dir(&temp_volume).await?;
    }
    let temp_file_path = temp_volume.join(file_name);

    let mut temp_file = {
        let mut o = tokio::fs::OpenOptions::new();
        o.write(true)
            .truncate(true)
            .create(true)
            .open(&temp_file_path)
            .await
    }?;

    debug!("writing to temp file...");
    let mut reader = StreamReader::new(field.map_err(std::io::Error::other));
    let bytes_written = tokio::io::copy(&mut reader, &mut temp_file).await?;
    temp_file.flush().await?;

    debug!("file written");
    Ok((temp_file_path, bytes_written))
}
