#![allow(unused)]
use std::{env::var, net::SocketAddr, str::FromStr};

use axum::{
    extract::FromRef,
    http::StatusCode,
    routing::{get, post},
    Router,
};
use common::{
    constant::{BODY_SIZE_LIMIT, SERVICE_APPLICATION_NAME, SERVICE_HOST, SERVICE_PORT},
    util::setup_tracing,
};
use store::StoreClient;
use tower_http::{
    limit::RequestBodyLimitLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use upload::domain::FileRouterState;

mod common;
mod store;
mod upload;
async fn fallback() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not Found")
}

#[derive(Clone)]
struct AppState {
    file_state: FileRouterState,
}
impl FromRef<AppState> for FileRouterState {
    fn from_ref(app_state: &AppState) -> FileRouterState {
        app_state.file_state.clone()
    }
}

fn get_file_router() -> Router<AppState> {
    let body_size_limit = (var("BODY_SIZE_LIMIT").unwrap_or_else(|_| "1024".into()))
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("could not extract {}", BODY_SIZE_LIMIT));
    Router::new()
        .route("/upload", post(upload::routes::upload))
        .route("/download", get(upload::routes::download))
        .route("/metadata", get(upload::routes::metadata))
        .layer(RequestBodyLimitLayer::new(body_size_limit))
}

#[tokio::main]
async fn main() {
    setup_tracing();
    let host = var(SERVICE_HOST).unwrap_or_else(|_| String::from("127.0.0.1"));
    let port = var(SERVICE_PORT).unwrap_or_else(|_| String::from("80"));
    let app_name = var(SERVICE_APPLICATION_NAME).unwrap_or_else(|_| String::from("köfte-service"));
    let addr = SocketAddr::from_str(&format!("{host}:{port}")).unwrap();
    let client = StoreClient::new(app_name).await.unwrap();
    tracing::info!("listening on {:?}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    let app = Router::new()
        .nest("/api/v1/upload", get_file_router())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(AppState {
            file_state: upload::routes::make_state(client),
        })
        .fallback(fallback);

    tracing::info!("listening on {:?}", listener);
    axum::serve(listener, app).await.unwrap();
}
