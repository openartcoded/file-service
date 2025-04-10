use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::FromRequestParts;
use axum::http;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::{Json, extract::OptionalFromRequestParts};
use axum_extra::headers;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::domain::ServiceError;

pub struct ExtractUserInfo {
    pub user_info: UserInfo,
    pub header: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserInfo {
    pub exp: u64,
    pub iat: Option<usize>,
    pub sub: Option<String>,
    pub rol: Vec<String>,
    pub group: Option<String>,
}
impl<S> OptionalFromRequestParts<S> for ExtractUserInfo
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, axum::Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        match <ExtractUserInfo as FromRequestParts<S>>::from_request_parts(parts, state).await {
            Ok(res) => Ok(Some(res)),
            Err(_) => Ok(None),
        }
    }
}
impl<'a> TryFrom<&'a str> for ExtractUserInfo {
    type Error = ServiceError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let r = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(value)
            .inspect_err(|e| tracing::error!("{e}"))
            .map(|b| (value.to_string(), b))
            .map(|(e, d)| {
                serde_json::from_slice::<UserInfo>(&d)
                    .map(|des| (e, des))
                    .ok()
            })
            .ok()
            .flatten()
            .map(|(header, user_info)| ExtractUserInfo { user_info, header });
        r.ok_or(ServiceError("could not extract token".to_string()))
    }
}
fn is_expired(timestamp: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    timestamp < now
}
impl<B> FromRequestParts<B> for ExtractUserInfo
where
    B: Send + Sync,
{
    type Rejection = (StatusCode, axum::Json<serde_json::Value>);

    async fn from_request_parts(req: &mut Parts, _state: &B) -> Result<Self, Self::Rejection> {
        if let Some(user_info) = req.headers.get(http::header::AUTHORIZATION) {
            match user_info
                .to_str()
                .ok()
                .and_then(|s| s.split("Bearer ").last())
                .and_then(|s| s.split(".").skip(1).take(1).last())
                .and_then(|token| ExtractUserInfo::try_from(token).ok())
            {
                Some(v) if is_expired(v.user_info.exp) => Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Unauthorized"})),
                )),
                Some(v) => Ok(v),
                _ => Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"Token is invalid"})),
                )),
            }
        } else {
            Err((
                StatusCode::FORBIDDEN,
                Json(json!({"error":"Token is missing"})),
            ))
        }
    }
}
