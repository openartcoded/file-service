use std::{env::var, fmt::Display, io::Cursor};

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{AppendHeaders, IntoResponse},
};
use axum_extra::headers::ContentType;
use mime_guess::mime::{APPLICATION_PDF, TEXT_XML};

use mongodb::{bson::doc, options::FindOneAndReplaceOptions};
use serde_json::json;
use tokio_util::io::ReaderStream;

use crate::{
    common::{
        constant::{DEFAULT_TENANT, TEMPL_SERVICE_COLLECTION_NAME},
        domain::ServiceError,
        util::{OpenApiBinaryResponse, OpenApiDocUploadForm, QueryIds, StoreCollection},
    },
    store::{Repository, StoreClient, StoreRepository},
    template::domain::{TemplateType, TemplateV2, TemplateWrapper},
    upload::{
        domain::{FileRouterState, FileUploadV2},
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
    // security(("bearerAuth" = []))
)]
pub async fn render(
    State(file_router_state): State<FileRouterState>,
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    Json(req): Json<RenderRequest>,
) -> axum::response::Result<axum::response::Response> {
    tracing::debug!("Template render route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
    match repository.find_by_id(&req.template_id).await {
        Ok(Some(tpl)) => {
            if tpl.template_context != req.template_context {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json! ({"error": "invalid template context"})),
                )
                    .into_response());
            }
            let (extension, ct) = match tpl.template_type {
                TemplateType::Html => (".pdf", APPLICATION_PDF),
                TemplateType::Xml => (".xml", TEXT_XML),
            };
            let result = super::render::render(
                &tpl,
                &req.context,
                &file_router_state,
                Some(DEFAULT_TENANT.to_string()),
            )
            .await?;
            let cursor = Cursor::new(result);
            let stream = ReaderStream::new(cursor);
            let body = axum::body::Body::from_stream(stream);
            let content_header = (
                header::CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{}{extension}""#, &req.file_name),
            );

            let content_type = (header::CONTENT_TYPE, ct.to_string());

            let headers = AppendHeaders([content_type, content_header]);

            Ok((StatusCode::OK, headers, body).into_response())
        }
        Ok(None) => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "template not found"})),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into()),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/template/find-by-type",
    params(TemplateTypeQuery),
    responses(
        (status = 200, description = "Find templates by type", body=Vec<TemplateV2>)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_by_type(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    Query(templ_type): Query<TemplateTypeQuery>,
) -> impl IntoResponse {
    tracing::debug!("Template find by context route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Find templates by context", body=Vec<TemplateV2>)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_by_context(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    Query(context): Query<ContextQuery>,
) -> impl IntoResponse {
    tracing::debug!("Template find by context route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Find By ids", body=Vec<TemplateV2>)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_by_ids(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,

    axum_extra::extract::Query(QueryIds { ids: query_ids }): axum_extra::extract::Query<QueryIds>,
) -> impl IntoResponse {
    tracing::debug!("Template list by ids route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Upsert a template", body=TemplateV2)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn upsert(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    State(file_router_state): State<FileRouterState>,
    Query(query): Query<TemplateUpsert>,
    mut form: Multipart,
) -> axum::response::Result<axum::response::Response> {
    tracing::debug!("Upsert template route entered!");

    fn handle_err<T: std::error::Error + Display>(e: T) -> axum::response::ErrorResponse {
        tracing::error!("could not proceed upsert invoice. err: {e:?}");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into()
    }

    let client = client.get_raw_client(); // todo, maybe make a SessionStoreRepository or something
    let mut session = client.start_session().await.map_err(handle_err)?;

    session.start_transaction().await.map_err(handle_err)?;

    let template_collection = session
        .client()
        .database(&DEFAULT_TENANT)
        .collection::<TemplateV2>(&collection.0);

    let maybe_template = {
        if let Some(id) = query.id {
            let i = template_collection.find_one(doc! {"_id": id}).await;
            match i.map_err(handle_err)? {
                Some(mut i) => {
                    i.updated_date = Some(bson::DateTime::now());
                    TemplateWrapper(i)
                }
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

    let mut template = TemplateV2 {
        title,
        description,
        template_context,
        ..maybe_template
    };
    let options = FindOneAndReplaceOptions::builder()
        .upsert(Some(true))
        .build();

    match form
        .next_field()
        .await
        .map_err(|e| ServiceError(e.to_string()))?
    {
        Some(mut field) => {
            let Some(file_name) = field.file_name().map(|s| s.to_string()) else {
                return Err(ServiceError("missing filename".into()).into());
            };

            let (temp_path, len) =
                write_field_to_temp_file(&mut field, &file_router_state.share_drive.0, &file_name)
                    .await;

            match &template_type {
                TemplateType::Html => {
                    if let Some(ct) = mime_guess::from_path(&temp_path).first()
                        && ContentType::from(ct) != ContentType::html()
                    {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": "File content type doesn't match template type (should be html)"})),
                        )
                            .into());
                    }
                }
                TemplateType::Xml => {
                    if let Some(ct) = mime_guess::from_path(&temp_path).first()
                        && ContentType::from(ct) != ContentType::xml()
                    {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": "File content type doesn't match template type (should be xml)"})),
                        )
                            .into());
                    }
                }
            }
            template.template_type = template_type;
            let repository: StoreRepository<FileUploadV2> = StoreRepository::get_repository(
                &file_router_state.client,
                &file_router_state.collection.0,
                &DEFAULT_TENANT,
            );
            let file_service = FileService {
                share_drive_path: &file_router_state.share_drive.0,
                store: &repository,
            };
            let fu = FileUploadV2::new(
                &temp_path.display().to_string(),
                &file_name,
                Some(template.id.clone()),
                false,
                len,
            )?;

            let upl = file_service.upload(fu, Some(&temp_path), false).await?;

            template.file_id = upl.id;
        }
        _ => {
            if template.file_id.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "you cannot save a template that doesn't have a file attached to it",
                )
                    .into());
            }
        }
    }
    template_collection
        .find_one_and_replace(doc! {"_id": &template.id}, &template)
        .with_options(options)
        .await
        .map_err(handle_err)?;

    session.commit_transaction().await.map_err(handle_err)?;
    Ok((StatusCode::OK, Json(template)).into_response())
}

#[utoipa::path(
    delete,
    path = "/api/v1/template/{id}",
    responses(
        (status = 200, description = "Delete a template by id")
    ),
    // security(("bearerAuth" = []))
)]
pub async fn delete_templ_by_id(
    State(fs): State<FileRouterState>,
    State(TemplRouterState { client, collection }): State<TemplRouterState>,
    Path(templ_id): Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Template delete one route entered!");

    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);

    let fs_repository: StoreRepository<FileUploadV2> =
        StoreRepository::get_repository(&fs.client, &fs.collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Find all templates", body=Vec<TemplateV2>)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_all(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
) -> impl IntoResponse {
    tracing::debug!("Template list route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);
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
        (status = 200, description = "Find a template by id", body=TemplateV2)
    ),
    // security(("bearerAuth" = []))
)]
pub async fn find_one(
    State(TemplRouterState { collection, client }): State<TemplRouterState>,
    Path(templ_id): Path<String>,
) -> impl IntoResponse {
    tracing::debug!("Template find one route entered!");
    let repository: StoreRepository<TemplateV2> =
        StoreRepository::get_repository(&client, &collection.0, &DEFAULT_TENANT);

    match repository.find_by_id(&templ_id).await {
        Ok(templ) => (StatusCode::OK, Json(templ)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
