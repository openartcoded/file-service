use std::error::Error;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{UtcOffset, macros::format_description};
use tracing::Level;
use tracing_subscriber::{EnvFilter, FmtSubscriber, fmt::time::OffsetTime};
use utoipa::{IntoParams, ToSchema};

#[derive(utoipa::ToSchema)]
#[schema(format = Binary,content_media_type = "*/*")]
pub struct OpenApiBinaryResponse(String);

#[derive(utoipa::ToSchema)]
pub struct OpenApiDocUploadForm {
    #[schema(content_media_type = "application/octet-stream", format = "binary")]
    pub file_bytes: String,
}

pub fn setup_tracing() -> Result<(), Box<dyn Error>> {
    let offset_hours = {
        let now = Local::now();
        let offset_seconds = now.offset().local_minus_utc();
        let hours = offset_seconds / 3600;
        hours as i8
    };
    let offset = UtcOffset::from_hms(offset_hours, 0, 0)?;

    let timer = OffsetTime::new(
        offset,
        format_description!("[day]-[month]-[year] [hour]:[minute]:[second]"),
    );
    let subscriber = FmtSubscriber::builder()
        .with_timer(timer)
        .with_max_level(Level::TRACE)
        .with_env_filter(EnvFilter::from_default_env())
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

#[deprecated]
pub fn to_value<T: Serialize + core::fmt::Debug>(data: T) -> Value {
    match serde_json::to_value(&data) {
        Ok(value) => value,
        Err(e) => {
            tracing::error!("error serializing {:?}, error: {e}", &data);
            json!({})
        }
    }
}
pub fn to_json_string<T: Serialize + core::fmt::Debug>(data: T) -> String {
    match serde_json::to_string(&data) {
        Ok(value) => value,
        Err(e) => {
            tracing::error!("error serialing {:?}, error: {e}", &data);
            "{}".into()
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoreCollection(pub String);

pub struct IdGenerator;

impl IdGenerator {
    pub fn get(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct QueryIds {
    pub ids: Vec<String>,
}
