use std::collections::HashMap;
use std::env::{temp_dir, var};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use axum::Json;
use axum::extract::multipart::Field;
use axum::extract::{Multipart, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse};
use mime_guess::mime::APPLICATION_OCTET_STREAM;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::common::constant::{FILE_SERVICE_COLLECTION_NAME, PUBLIC_TENANT, SHARE_DRIVE_PATH};
use crate::common::user_header::ExtractUserInfo;
use crate::common::util::{OpenApiBinaryResponse, OpenApiDocUploadForm, StoreCollection};
use crate::store::{Repository, StoreClient, StoreRepository};
use crate::upload::service::{FileService, write_field_to_temp_file};

use super::domain::{
    DownloadFileRequestUriParams, FileRouterState, FileUpload, ShareDrive,
    UploadFileRequestUriParams,
};

pub async fn make_state(client: StoreClient) -> FileRouterState {
    let share_drive_path: String = std::env::var(SHARE_DRIVE_PATH).unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(client.get_application_name())
            .display()
            .to_string()
    });
    tracing::info!("share path: {}", share_drive_path);
    if !PathBuf::from_str(&share_drive_path).unwrap().exists() {
        tokio::fs::create_dir(&share_drive_path).await.unwrap();
    }
    let collection_name: String =
        var(FILE_SERVICE_COLLECTION_NAME).unwrap_or_else(|_| String::from("fileUpload"));
    FileRouterState {
        client,
        share_drive: ShareDrive(share_drive_path),
        collection: StoreCollection(collection_name),
    }
}
#[utoipa::path(
    get,
    path = "/api/v1/upload/metadata",
    params(DownloadFileRequestUriParams),
    responses(
        (status = 200, description = "Get upload metadata", body=FileUpload)
    ),
    security(("bearerAuth" = []))
)]
pub async fn metadata(
    State(FileRouterState {
        client, collection, ..
    }): State<FileRouterState>,
    x_user_info: Option<ExtractUserInfo>,
    Query(DownloadFileRequestUriParams { id }): Query<DownloadFileRequestUriParams>,
) -> impl IntoResponse {
    tracing::debug!("Metadata route entered!");

    let tenant = x_user_info
        .as_ref()
        .map(|u| &u.user_info)
        .and_then(|u| u.group.clone());
    match FileService::get_file_upload(&id, tenant, &client, &collection).await {
        Some((_, upl)) => Json(upl).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/upload/download",
    params(DownloadFileRequestUriParams),
    responses(
        (status = 200, description = "Download file",content_type = "*/*",body=inline(OpenApiBinaryResponse))
    ),
    security(("bearerAuth" = []))
)]
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
    let tenant = x_user_info
        .as_ref()
        .map(|u| &u.user_info)
        .and_then(|u| u.group.clone());
    match FileService::get_file_upload(&id, tenant, &client, &collection).await {
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

#[utoipa::path(
    get,
    path = "/api/v1/upload/find-all",
    responses(
        (status = 200, description = "Find all upload", body=Vec<FileUpload>)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_all_uploads(
    State(FileRouterState {
        client, collection, ..
    }): State<FileRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
) -> impl IntoResponse {
    tracing::debug!("Template list route entered!");
    let repository: StoreRepository<FileUpload> = StoreRepository::get_repository(
        client,
        &collection.0,
        &x_user_info.group.unwrap_or_else(|| PUBLIC_TENANT.into()),
    )
    .await;
    match repository.find_all().await {
        Ok(templ) => (StatusCode::OK, Json(templ)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
#[utoipa::path(
    delete,
    path = "/api/v1/upload/{id}",
    responses(
        (status = 200, description = "Delete a file by id")
    ),
    security(("bearerAuth" = []))
)]
pub async fn delete_by_id(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    axum::extract::Path(upl_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Delete upload route entered!");
    let Some(tenant) = x_user_info.group else {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "result": "tenant is missing"
            })),
        );
    };
    let fs_repository: StoreRepository<FileUpload> =
        StoreRepository::get_repository(client, &collection.0, &tenant).await;
    let file_service = FileService {
        share_drive_path: &share_drive,
        store: &fs_repository,
    };
    if let Err(e) = file_service.delete_by_id(&upl_id).await {
        tracing::error!("could not delete files {e:?}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "result": format!("upl with id {} could not be deleted, check logs", &upl_id)
            })),
        )
    } else {
        (
            StatusCode::OK,
            Json(json!({
                "result": format!("upl with id {} deleted", &upl_id)
            })),
        )
    }
}
#[utoipa::path(
    post,
    path = "/api/v1/upload",
    params(UploadFileRequestUriParams),
    request_body(content = inline(OpenApiDocUploadForm), content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Upload a file", body=FileUpload)
    ),
    security(("bearerAuth" = []))
)]
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
            x_user_info.group.unwrap()
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
        let tenant = &x_user_info.group.unwrap();
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
