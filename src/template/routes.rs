use core::panic;
use std::{
    env::{temp_dir, var},
    error::Error,
    io::Cursor,
};

use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{AppendHeaders, IntoResponse},
    routing::{delete, get, post},
};
use axum_extra::headers::ContentType;
use chrono::Local;
use mime_guess::mime::APPLICATION_PDF;

use mongodb::{bson::doc, options::FindOneAndReplaceOptions};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::{
    common::{
        constant::{PUBLIC_TENANT, TEMPL_SERVICE_COLLECTION_NAME},
        user_header::ExtractUserInfo,
        util::{
            IdGenerator, OpenApiBinaryResponse, OpenApiDocUploadForm, QueryIds, StoreCollection,
        },
    },
    store::{Repository, StoreClient, StoreRepository},
    template::domain::{Template, TemplateType, TemplateWrapper},
    upload::{
        domain::{FileRouterState, FileUpload},
        service::{FileService, write_field_to_temp_file},
    },
};

use super::domain::{
    ContextQuery, RenderRequest, TemplRouterState, TemplateTypeQuery, TemplateUpsert,
};

pub fn make_state(client: StoreClient) -> TemplRouterState {
    let collection_name: String =
        var(TEMPL_SERVICE_COLLECTION_NAME).unwrap_or_else(|_| String::from("template"));
    TemplRouterState {
        client,
        collection: StoreCollection(collection_name),
    }
}
#[utoipa::path(
    post,
    path = "/api/v1/template/render",
    responses(
        (status = 200, description = "Render template", content_type = "*/*",body=inline(OpenApiBinaryResponse))
    ),
    security(("bearerAuth" = []))
)]
pub async fn render(
    State(file_router_state): State<FileRouterState>,
    x_user_info: ExtractUserInfo,
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    Json(req): Json<RenderRequest>,
) -> impl IntoResponse {
    tracing::debug!("Template render route entered!");
    let tenant = x_user_info
        .user_info
        .group
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PUBLIC_TENANT.to_string());
    let repository: StoreRepository<Template> =
        StoreRepository::get_repository(client, &collection.0, &tenant).await;
    match repository.find_by_id(&req.template_id).await {
        Ok(Some(tpl)) => {
            if tpl.template_context != req.template_context {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json! ({"error": "invalid template context"})),
                )
                    .into_response();
            }
            let pdf = super::render::render(&tpl, &req.context, &file_router_state, Some(tenant))
                .await
                .unwrap();
            let cursor = Cursor::new(pdf);
            let stream = ReaderStream::new(cursor);
            let body = axum::body::Body::from_stream(stream);
            let content_header = (
                header::CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{}""#, &req.file_name),
            );

            let content_type = (header::CONTENT_TYPE, APPLICATION_PDF.to_string());

            let headers = AppendHeaders([content_type, content_header]);

            (StatusCode::OK, headers, body).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "template not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/template/find-by-type",
    params(TemplateTypeQuery),
    responses(
        (status = 200, description = "Find templates by type", body=Vec<Template>)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_by_type(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    Query(templ_type): Query<TemplateTypeQuery>,
) -> impl IntoResponse {
    tracing::debug!("Template find by context route entered!");
    let repository: StoreRepository<Template> = StoreRepository::get_repository(
        client,
        &collection.0,
        &x_user_info.group.unwrap_or_else(|| PUBLIC_TENANT.into()),
    )
    .await;
    let templ_type = templ_type.template_type.to_string();
    let query = doc! {"templateType": templ_type};
    match repository.find_by_query(query, None).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
#[utoipa::path(
    get,
    path = "/api/v1/template/find-by-context",
    params(ContextQuery),
    responses(
        (status = 200, description = "Find templates by context", body=Vec<Template>)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_by_context(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    Query(context): Query<ContextQuery>,
) -> impl IntoResponse {
    tracing::debug!("Template find by context route entered!");
    let repository: StoreRepository<Template> = StoreRepository::get_repository(
        client,
        &collection.0,
        &x_user_info.group.unwrap_or_else(|| PUBLIC_TENANT.into()),
    )
    .await;
    let context = context.context.to_string();
    let query = doc! {"templateContext": context};
    match repository.find_by_query(query, None).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
#[utoipa::path(
    get,
    path = "/api/v1/template/find-by-ids",
    params(QueryIds),
    responses(
        (status = 200, description = "Find By ids", body=Vec<Template>)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_by_ids(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    axum_extra::extract::Query(QueryIds { ids: query_ids }): axum_extra::extract::Query<QueryIds>,
) -> impl IntoResponse {
    tracing::debug!("Template list by ids route entered!");
    let repository: StoreRepository<Template> = StoreRepository::get_repository(
        client,
        &collection.0,
        &x_user_info.group.unwrap_or_else(|| PUBLIC_TENANT.into()),
    )
    .await;
    match repository.find_by_ids(query_ids).await {
        Ok(templs) => (StatusCode::OK, Json(templs)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/template",
    params(TemplateUpsert),
    request_body(content = inline(OpenApiDocUploadForm), content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Upsert a template", body=Template)
    ),
    security(("bearerAuth" = []))
)]
pub async fn upsert(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    State(file_router_state): State<FileRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        header: x_user_info_header,
    }: ExtractUserInfo,
    Query(query): Query<TemplateUpsert>,
    mut form: Multipart,
) -> impl IntoResponse {
    tracing::debug!("Upsert template route entered!");

    let handle_err = |e: mongodb::error::Error| {
        tracing::error!("could not proceed upsert invoice. err: {e:?}");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
    };
    let Some(tenant) = x_user_info.group else {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "result": "tenant is missing"
            })),
        )
            .into_response();
    };
    let client = client.get_raw_client(); // todo, maybe make a SessionStoreRepository or something
    let mut session = match client.start_session().await {
        Ok(session) => session,
        Err(e) => return handle_err(e),
    };

    if let Err(e) = session.start_transaction().await {
        return handle_err(e);
    }

    let template_collection = session
        .client()
        .database(&tenant)
        .collection::<Template>(&collection.0);

    let maybe_template = {
        if let Some(id) = query.id {
            let i = template_collection.find_one(doc! {"_id": id}).await;
            match i {
                Ok(Some(mut i)) => {
                    i.updated_date = Some(Local::now().to_utc());
                    TemplateWrapper(i)
                }
                Err(e) => return handle_err(e),
                _ => Default::default(),
            }
        } else {
            Default::default()
        }
    };
    let maybe_template = maybe_template.0;

    let TemplateUpsert {
        id: _,
        title,
        description,
        template_type,
        template_context,
    } = query;

    let mut template = Template {
        title,
        description,
        template_context,
        ..maybe_template
    };
    let options = FindOneAndReplaceOptions::builder()
        .upsert(Some(true))
        .build();

    match form.next_field().await.unwrap() {
        Some(mut field) => {
            let file_name = field.file_name().unwrap().to_string();

            let (temp_path, len) =
                write_field_to_temp_file(&mut field, &file_router_state.share_drive.0, &file_name)
                    .await;

            match &template_type {
                TemplateType::Html => {
                    if let Some(ct) = mime_guess::from_path(&temp_path).first() {
                        if ContentType::from(ct) != ContentType::html() {
                            return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": "File content type doesn't match template type"})),
                        )
                            .into_response();
                        }
                    }
                }
            }
            template.template_type = template_type;
            let repository: StoreRepository<FileUpload> = StoreRepository::get_repository(
                file_router_state.client,
                &file_router_state.collection.0,
                &tenant,
            )
            .await;
            let file_service = FileService {
                share_drive_path: &file_router_state.share_drive.0,
                store: &repository,
            };
            let fu = FileUpload::new(
                &temp_path.display().to_string(),
                &file_name,
                Some(template.id.clone()),
                false,
                len,
            )
            .unwrap();

            let upl = file_service.upload(fu, Some(&temp_path)).await.unwrap();

            template.file_id = upl.id;
        }
        _ => {
            if template.file_id.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    "you cannot save a template that doesn't have a file attached to it",
                )
                    .into_response();
            }
        }
    }
    if let Err(e) = template_collection
        .find_one_and_replace(doc! {"_id": &template.id}, &template)
        .with_options(options)
        .await
    {
        return handle_err(e);
    }

    if let Err(e) = session.commit_transaction().await {
        return handle_err(e);
    }

    (StatusCode::OK, Json(template)).into_response()
}

#[utoipa::path(
    delete,
    path = "/api/v1/template/{id}",
    responses(
        (status = 200, description = "Delete a template by id")
    ),
    security(("bearerAuth" = []))
)]
pub async fn delete_templ_by_id(
    State(fs): State<FileRouterState>,
    State(TemplRouterState { client, collection }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    Path(templ_id): Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Template delete one route entered!");
    let Some(tenant) = x_user_info.group else {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "result": "tenant is missing"
            })),
        );
    };
    let repository: StoreRepository<Template> =
        StoreRepository::get_repository(client, &collection.0, &tenant).await;

    let fs_repository: StoreRepository<FileUpload> =
        StoreRepository::get_repository(fs.client, &fs.collection.0, &tenant).await;
    let file_service = FileService {
        share_drive_path: &fs.share_drive.0,
        store: &fs_repository,
    };
    match repository.delete_by_id(&templ_id).await {
        Ok(Some(templ)) => {
            if let Err(e) = file_service.delete_by_correlation_id(&templ_id).await {
                tracing::error!("could not delete files linked to templ {templ:?} => {e}")
            };
            (
                StatusCode::OK,
                Json(json!({
                    "result": format!("templ with id {} deleted", &templ.id)
                })),
            )
        }
        Ok(None) => (
            StatusCode::NO_CONTENT,
            Json(json!({
                "result": format!("templ with id {} not found", &templ_id)
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/template/find-all",
    responses(
        (status = 200, description = "Find all templates", body=Vec<Template>)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_all(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
) -> impl IntoResponse {
    tracing::debug!("Template list route entered!");
    let repository: StoreRepository<Template> = StoreRepository::get_repository(
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
    get,
    path = "/api/v1/template/find-one/{templ_id}",
    responses(
        (status = 200, description = "Find a template by id", body=Template)
    ),
    security(("bearerAuth" = []))
)]
pub async fn find_one(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    ExtractUserInfo {
        user_info: x_user_info,
        ..
    }: ExtractUserInfo,
    Path(templ_id): Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Template find one route entered!");
    let repository: StoreRepository<Template> = StoreRepository::get_repository(
        client,
        &collection.0,
        &x_user_info.group.unwrap_or_else(|| PUBLIC_TENANT.into()),
    )
    .await;

    match repository.find_by_id(&templ_id).await {
        Ok(templ) => (StatusCode::OK, Json(templ)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
