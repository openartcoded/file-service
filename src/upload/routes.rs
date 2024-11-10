use std::collections::HashMap;
use std::env::var;
use std::path::{Path, PathBuf};

use axum::extract::multipart::Field;
use axum::extract::{Multipart, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{header, StatusCode};
use axum::response::{AppendHeaders, IntoResponse};
use axum::Json;
use mime_guess::mime::APPLICATION_OCTET_STREAM;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::common::constant::{FILE_SERVICE_COLLECTION_NAME, PUBLIC_TENANT, SHARE_DRIVE_PATH};
use crate::common::user_header::ExtractUserInfo;
use crate::common::util::StoreCollection;
use crate::store::{Repository, StoreClient, StoreRepository};
use crate::upload::service::FileService;

use super::domain::{
    DownloadFileRequestUriParams, FileRouterState, FileUpload, ShareDrive,
    UploadFileRequestUriParams,
};

pub fn make_state(client: StoreClient) -> FileRouterState {
    let share_drive_path: String = std::env::var(SHARE_DRIVE_PATH).unwrap();
    let collection_name: String =
        var(FILE_SERVICE_COLLECTION_NAME).unwrap_or_else(|_| String::from("upload"));
    FileRouterState {
        client,
        share_drive: ShareDrive(share_drive_path),
        collection: StoreCollection(collection_name),
    }
}

pub async fn metadata(
    State(FileRouterState {
        client, collection, ..
    }): State<FileRouterState>,
    x_user_info: Option<ExtractUserInfo>,
    Query(DownloadFileRequestUriParams { id }): Query<DownloadFileRequestUriParams>,
) -> impl IntoResponse {
    tracing::debug!("Metadata route entered!");

    match get_file_upload(&id, &x_user_info, &client, &collection).await {
        Some((_, upl)) => Json(upl).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into_response(),
    }
}
pub async fn download(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,
    x_user_info: Option<ExtractUserInfo>,
    Query(DownloadFileRequestUriParams { id }): Query<DownloadFileRequestUriParams>,
) -> impl IntoResponse {
    tracing::debug!("Download route entered!");

    tracing::debug!("trying to fetch document with id {id}");

    match get_file_upload(&id, &x_user_info, &client, &collection).await {
        Some((repo, file)) => {
            let file_service = FileService {
                share_drive_path: &share_drive,
                store: &repo,
            };
            let file_handle = file_service.download(&file).await.unwrap();
            let stream = ReaderStream::new(file_handle);
            let body = axum::body::Body::from_stream(stream);

            let content_header = if file.is_image() {
                (header::CONTENT_LENGTH, format!("{}", &file.size))
            } else {
                (
                    header::CONTENT_DISPOSITION,
                    format!(r#"attachment; filename="{}""#, &file.original_filename),
                )
            };

            let ct = file
                .content_type
                .unwrap_or_else(|| APPLICATION_OCTET_STREAM.to_string());

            let content_type = (CONTENT_TYPE, ct);

            let headers = AppendHeaders([content_type, content_header]);

            (headers, body).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into_response(),
    }
}
pub async fn upload(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    Query(mut query): Query<UploadFileRequestUriParams>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    tracing::debug!("Upload route entered!");

    let mut uploads = HashMap::new();

    while let Some(mut field) = multipart.next_field().await.unwrap() {
        let file_name = field.file_name().unwrap().to_string();

        let mut file_upload = FileUpload {
            content_type: field.content_type().map(|ct| ct.into()).or_else(|| {
                mime_guess::from_path(&file_name)
                    .first_raw()
                    .map(|ct| ct.into())
            }),
            correlation_id: query.correlation_id.take(),
            extension: Path::new(&file_name)
                .extension()
                .map(|s| s.to_string_lossy().to_string()),
            original_filename: file_name.to_string(),
            ..Default::default()
        };
        let (temp_file_path, len) =
            write_field_to_temp_file(&mut field, &share_drive, &file_name).await;

        file_upload.size = len;

        tracing::debug!("Length of `{}` is {} bytes", file_name, len);

        uploads.insert(file_name, (file_upload, temp_file_path));
    }

    if uploads.len() == 1 {
        let Some((_, (mut upl, temp_file_path))) = uploads.into_iter().last() else {
            unreachable!("should never happen")
        };

        if let Some(id) = query.id.take() {
            upl.id = id;
        }

        upl.public_resource = query.is_public.unwrap_or(false);

        let tenant = if upl.public_resource {
            PUBLIC_TENANT.into()
        } else {
            x_user_info.tenant.unwrap()
        };

        let repository: StoreRepository<FileUpload> =
            StoreRepository::get_repository(client, &collection.0, &tenant).await;
        let file_service = FileService {
            share_drive_path: &share_drive,
            store: &repository,
        };
        let upl = file_service
            .upload(upl, Some(&temp_file_path))
            .await
            .unwrap();

        (StatusCode::OK, Json(upl)).into_response()
    } else {
        let mut uploads_resp = Vec::with_capacity(uploads.len());
        let tenant = &x_user_info.tenant.unwrap();
        let repository: StoreRepository<FileUpload> =
            StoreRepository::get_repository(client, &collection.0, tenant).await;
        let file_service = FileService {
            share_drive_path: &share_drive,
            store: &repository,
        };
        for (_, (upl, temp_file_path)) in uploads {
            let upl = file_service
                .upload(upl, Some(&temp_file_path))
                .await
                .unwrap();
            uploads_resp.push(upl);
        }
        (StatusCode::OK, Json(uploads_resp)).into_response()
    }
}
async fn write_field_to_temp_file<'a>(
    field: &mut Field<'a>,
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
async fn get_file_upload(
    id: &str,
    x_user_info: &Option<ExtractUserInfo>,
    client: &StoreClient,
    collection: &StoreCollection,
) -> Option<(StoreRepository<FileUpload>, FileUpload)> {
    async fn get_upload(repository: &StoreRepository<FileUpload>, id: &str) -> Option<FileUpload> {
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
    } else if let Some(tenant) = x_user_info
        .as_ref()
        .map(|u| &u.user_info)
        .and_then(|u| u.tenant.clone())
    {
        let private_repository: StoreRepository<FileUpload> =
            StoreRepository::get_repository(client.clone(), &collection.0, &tenant).await;
        get_upload(&private_repository, id)
            .await
            .map(|fu| (private_repository, fu))
    } else {
        None
    }
}
