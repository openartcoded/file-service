use std::{
    env::{args, var},
    net::SocketAddr,
    str::FromStr,
};

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    http::StatusCode,
    routing::{delete, get, post},
};
use common::{
    constant::{BODY_SIZE_LIMIT, SERVICE_APPLICATION_NAME, SERVICE_HOST, SERVICE_PORT},
    util::{OpenApiBinaryResponse, setup_tracing},
};
use store::StoreClient;
use template::domain::{Context, TemplRouterState, TemplateType, TemplateUpsert, TemplateV2};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::info;
use upload::domain::{
    DownloadFileRequestUriParams, FileRouterState, FileUploadV2, FindAllQueryParams,
    UploadFileRequestUriParams,
};
use utoipa::{Modify, OpenApi};
use utoipa_swagger_ui::SwaggerUi;

use crate::template::render;

mod common;
mod store;
mod template;
mod upload;

async fn fallback() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not Found")
}

#[derive(Clone)]
struct AppState {
    file_state: FileRouterState,
    templ_state: TemplRouterState,
}
impl FromRef<AppState> for FileRouterState {
    fn from_ref(app_state: &AppState) -> FileRouterState {
        app_state.file_state.clone()
    }
}

impl FromRef<AppState> for TemplRouterState {
    fn from_ref(app_state: &AppState) -> TemplRouterState {
        app_state.templ_state.clone()
    }
}
#[derive(OpenApi)]
#[openapi(
    info(description = "File Api V1", title="FileApi",version="0.2", license(identifier="MIT")),
    components(schemas(TemplateType,OpenApiBinaryResponse, FileUploadV2,FindAllQueryParams, TemplateV2,UploadFileRequestUriParams, DownloadFileRequestUriParams, Context,TemplateUpsert)),
    paths(
        upload::routes::metadata,
        upload::routes::find_all_uploads,
        upload::routes::download,
        upload::routes::upload,
        upload::routes::delete_by_id,
        template::routes::find_all,
        template::routes::find_by_ids,
        template::routes::find_by_context,
        template::routes::find_by_type,
        template::routes::find_one,
        template::routes::delete_templ_by_id,
        template::routes::render,
        template::routes::upsert
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(_components) = openapi.components.as_mut() {
            // disabled
            // components.add_security_scheme(
            //     "bearerAuth",
            //     SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            // )
        }
    }
}

fn get_templ_router() -> Router<AppState> {
    Router::new()
        .route("/find-all", get(template::routes::find_all))
        .route("/find-by-ids", get(template::routes::find_by_ids))
        .route("/find-by-context", get(template::routes::find_by_context))
        .route("/find-by-type", get(template::routes::find_by_type))
        .route("/find-one/{templ_id}", get(template::routes::find_one))
        .route("/{templ_id}", delete(template::routes::delete_templ_by_id))
        .route("/render", post(template::routes::render))
        .route("/", post(template::routes::upsert))
}

fn get_file_router() -> Router<AppState> {
    let body_size_limit = (var("BODY_SIZE_LIMIT")
        .unwrap_or_else(|_| format!("{}", 1024 * 1024 * 50)))
    .parse::<usize>()
    .unwrap_or_else(|_| panic!("could not extract {}", BODY_SIZE_LIMIT));

    info!("body_size_limit set to {body_size_limit}mb");

    Router::new()
        .route("/find-all", get(upload::routes::find_all_uploads))
        .route("/{upl_id}", delete(upload::routes::delete_by_id))
        .route("/download", get(upload::routes::download))
        .route("/metadata", get(upload::routes::metadata))
        .route("/", post(upload::routes::upload))
        .layer(DefaultBodyLimit::max(body_size_limit))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_tracing()?;
    let api_doc = ApiDoc::openapi();
    if args().skip(1).take(1).any(|s| &s == "--generate-openapi") {
        tracing::info!("generate openapi spec...");
        tokio::fs::remove_file("openapi.json").await?;
        tokio::fs::write("openapi.json", api_doc.to_pretty_json()?).await?;
        tracing::info!("done.");
        return Ok(());
    }
    let host = var(SERVICE_HOST).unwrap_or_else(|_| String::from("127.0.0.1"));
    let port = var(SERVICE_PORT).unwrap_or_else(|_| String::from("80"));
    let app_name = var(SERVICE_APPLICATION_NAME).unwrap_or_else(|_| String::from("file-service"));
    let addr = SocketAddr::from_str(&format!("{host}:{port}"))?;
    let client = StoreClient::new(app_name).await?;
    tracing::info!("listening on {:?}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let app = Router::new()
        .nest("/api/v1/upload", get_file_router())
        .nest("/api/v1/template", get_templ_router())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(AppState {
            file_state: upload::routes::make_state(client.clone()).await,
            templ_state: template::routes::make_state(client),
        })
        .merge(SwaggerUi::new("/openapi").url("/api-docs/openapi.json", api_doc))
        .fallback(fallback);

    render::init()?;
    tracing::info!("listening on {:?}", listener);
    axum::serve(listener, app).await?;
    Ok(())
}
