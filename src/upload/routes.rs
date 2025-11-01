use std::collections::HashMap;
use std::env::var;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use axum::Json;
use axum::extract::{Multipart, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse};
use mime_guess::mime::APPLICATION_OCTET_STREAM;
use mongodb::bson::doc;
use serde_json::json;
use tokio_util::io::ReaderStream;

use crate::common::constant::{DEFAULT_TENANT, FILE_SERVICE_COLLECTION_NAME, SHARE_DRIVE_PATH};
use crate::common::domain::ServiceError;
use crate::common::util::{OpenApiBinaryResponse, OpenApiDocUploadForm, StoreCollection};
use crate::store::{Repository, StoreClient, StoreRepository};
use crate::upload::domain::FindAllQueryParams;
use crate::upload::service::{FileService, write_field_to_temp_file};

use super::domain::{
    DownloadFileRequestUriParams, FileRouterState, FileUploadV2, ShareDrive,
    UploadFileRequestUriParams,
};

pub async fn make_state(client: StoreClient) -> Result<FileRouterState, Box<dyn Error>> {
    let share_drive_path: String = std::env::var(SHARE_DRIVE_PATH).unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(client.get_application_name())
            .display()
            .to_string()
    });
    tracing::info!("share path: {}", share_drive_path);
    if !PathBuf::from_str(&share_drive_path)?.exists() {
        tokio::fs::create_dir(&share_drive_path).await?;
    }
    let collection_name: String =
        var(FILE_SERVICE_COLLECTION_NAME).unwrap_or_else(|_| String::from("fileUpload"));
    Ok(FileRouterState {
        client,
        share_drive: ShareDrive(share_drive_path),
        collection: StoreCollection(collection_name),
    })
}
#[utoipa::path(
    get,
    path = "/api/v1/upload/metadata",
    params(DownloadFileRequestUriParams),
    responses(
        (status = 200, description = "Get upload metadata", body=FileUploadV2)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn metadata(
    State(FileRouterState {
        client, collection, ..
    }): State<FileRouterState>,
    Query(DownloadFileRequestUriParams { id }): Query<DownloadFileRequestUriParams>,
) -> impl IntoResponse {
    tracing::debug!("Metadata route entered!");

    match FileService::get_file_upload(&id, Some(DEFAULT_TENANT.to_string()), &client, &collection)
        .await
    {
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
    // security(("bearerAuth" = []))
)]
pub async fn download(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,
    Query(DownloadFileRequestUriParams { id }): Query<DownloadFileRequestUriParams>,
) -> axum::response::Result<axum::response::Response> {
    tracing::debug!("Download route entered!");

    tracing::debug!("trying to fetch document with id {id}");
    match FileService::get_file_upload(&id, Some(DEFAULT_TENANT.to_string()), &client, &collection)
        .await
    {
        Some((repo, file)) => {
            let file_service = FileService {
                share_drive_path: &share_drive,
                store: &repo,
            };
            let file_handle = file_service.download(&file).await?;
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

            Ok((headers, body).into_response())
        }
        None => Err((StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into()),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/upload/find-all",
    params(FindAllQueryParams),
    responses(
        (status = 200, description = "Find all upload", body=Vec<FileUploadV2>)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_all_uploads(
    State(FileRouterState {
        client, collection, ..
    }): State<FileRouterState>,
    Query(params): Query<FindAllQueryParams>,
) -> impl IntoResponse {
    tracing::debug!("Template list route entered!");
    let repository: StoreRepository<FileUploadV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
    let query = if let Some(correlation_id) = params.correlation_id {
        doc! {"correlationId": correlation_id}
    } else {
        doc! {}
    };
    match repository.find_by_query(query, None).await {
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
    // security(("bearerAuth" = []))
)]
pub async fn delete_by_id(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,

    axum::extract::Path(upl_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Delete upload route entered!");
    let fs_repository: StoreRepository<FileUploadV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Upload a file", body=FileUploadV2)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn upload(
    State(FileRouterState {
        client,
        collection,
        share_drive: ShareDrive(share_drive),
    }): State<FileRouterState>,
    Query(mut query): Query<UploadFileRequestUriParams>,
    mut multipart: Multipart,
) -> axum::response::Result<axum::response::Response> {
    tracing::debug!("Upload route entered!");

    let mut uploads = HashMap::new();

    while let Some(mut field) = multipart.next_field().await? {
        let file_name = field
            .file_name()
            .ok_or(ServiceError("no file name in field".into()))?
            .to_string();

        let mut file_upload = FileUploadV2 {
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
            bookmarked: Some(false),
            name: Some(file_name.to_string()),
            ..Default::default()
        };
        let (temp_file_path, len) = write_field_to_temp_file(&mut field, &share_drive, &file_name)
            .await
            .map_err(|e| ServiceError(e.to_string()))?;

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

        let repository: StoreRepository<FileUploadV2> =
            StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
        let file_service = FileService {
            share_drive_path: &share_drive,
            store: &repository,
        };
        let upl = file_service
            .upload(
                upl,
                Some(&temp_file_path),
                query.without_thumbnail.unwrap_or(false),
            )
            .await?;

        Ok((StatusCode::OK, Json(upl)).into_response())
    } else {
        let mut uploads_resp = Vec::with_capacity(uploads.len());
        let repository: StoreRepository<FileUploadV2> =
            StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
        let file_service = FileService {
            share_drive_path: &share_drive,
            store: &repository,
        };
        for (_, (upl, temp_file_path)) in uploads {
            let upl = file_service
                .upload(
                    upl,
                    Some(&temp_file_path),
                    query.without_thumbnail.unwrap_or(false),
                )
                .await?;
            uploads_resp.push(upl);
        }
        Ok((StatusCode::OK, Json(uploads_resp)).into_response())
    }
}
