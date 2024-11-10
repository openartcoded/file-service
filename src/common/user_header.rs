use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{constant::X_USER_INFO_HEADER, domain::ServiceError};

pub struct ExtractUserInfo {
    pub user_info: UserInfo,
    pub header: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserInfo {
    pub id: String,
    pub full_name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub tenant: Option<String>,
}

impl<'a> TryFrom<&'a str> for ExtractUserInfo {
    type Error = ServiceError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let r = base64::engine::general_purpose::STANDARD
            .decode(value)
            .map(|b| (value.to_string(), b))
            .map(|(e, d)| {
                serde_json::from_slice::<UserInfo>(&d)
                    .map(|des| (e, des))
                    .ok()
            })
            .ok()
            .flatten()
            .map(|(header, user_info)| ExtractUserInfo { user_info, header });
        r.ok_or(ServiceError(format!("could not extract token")))
    }
}
#[async_trait]
impl<B> FromRequestParts<B> for ExtractUserInfo
where
    B: Send + Sync,
{
    type Rejection = (StatusCode, axum::Json<serde_json::Value>);

    async fn from_request_parts(req: &mut Parts, _state: &B) -> Result<Self, Self::Rejection> {
        if let Some(user_info) = req.headers.get(X_USER_INFO_HEADER) {
            match user_info
                .to_str()
                .ok()
                .and_then(|token| ExtractUserInfo::try_from(token).ok())
            {
                Some(v) => Ok(v),
                _ => Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"X-USER-INFO is invalid"})),
                )),
            }
        } else {
            Err((
                StatusCode::FORBIDDEN,
                Json(json!({"error":"X-USER-INFO is missing"})),
            ))
        }
    }
}
