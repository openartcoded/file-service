use std::{error::Error, fmt::Display};

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde_json::json;

#[derive(Debug)]
pub struct ServiceError(String);

impl Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ServiceError {}

impl ServiceError {
    pub fn new(msg: impl Display) -> Self {
        ServiceError(msg.to_string())
    }
}
impl IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": self.to_string()})),
        )
            .into_response()
    }
}
