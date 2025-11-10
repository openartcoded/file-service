use std::error::Error;

use chrono::Local;
use serde::Deserialize;
use time::{UtcOffset, macros::format_description};
use tracing::Level;
use tracing_subscriber::{EnvFilter, FmtSubscriber, fmt::time::OffsetTime};
use utoipa::{IntoParams, ToSchema};

#[derive(utoipa::ToSchema)]
#[schema(format = Binary,content_media_type = "*/*")]
#[allow(unused)]
pub struct OpenApiBinaryResponse(String);

#[derive(utoipa::ToSchema)]
#[allow(unused)]
pub struct OpenApiDocUploadForm {
    #[schema(content_media_type = "application/octet-stream", format = "binary",  value_type = Vec<String>)]
    pub files: Vec<String>,
}
#[derive(utoipa::ToSchema)]
#[allow(unused)]
pub struct OpenApiDocUploadFormSimpleFile {
    #[schema(value_type=String, format = Binary)]
    pub file: String,
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

#[derive(Debug, Clone)]
pub struct StoreCollection(pub String);
pub struct IdGenerator;

impl IdGenerator {
    pub fn get(&self) -> String {
        uuid::Uuid::now_v7().to_string()
    }
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct QueryIds {
    pub ids: Vec<String>,
}
