use std::{path::PathBuf, str::FromStr};

use crate::{
    common::{
        domain::ServiceError,
        util::{IdGenerator, StoreCollection},
    },
    store::{Identifiable, StoreClient},
};
use bson::DateTime;
use mongodb::bson::Bson;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};

fn object_id_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Bson::deserialize(deserializer)?;
    match value {
        Bson::ObjectId(oid) => Ok(oid.to_string()),
        Bson::String(s) => Ok(s),
        other => Err(serde::de::Error::custom(format!(
            "unexpected _id type: {:?}",
            other
        ))),
    }
}

#[derive(Debug, Eq, Clone, PartialEq, PartialOrd, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadV2 {
    #[serde(rename = "_id", deserialize_with = "object_id_to_string")]
    pub id: String,
    pub creation_date: DateTime,
    pub updated_date: Option<DateTime>,
    pub content_type: Option<String>,
    pub bookmarked: Option<bool>,
    pub bookmarked_date: Option<DateTime>,
    pub name: Option<String>,
    pub thumbnail_id: Option<String>,
    pub thumb: Option<bool>,
    pub original_filename: String,
    pub extension: Option<String>,
    pub size: u64,
    pub public_resource: bool,
    pub correlation_id: Option<String>,
}

impl Identifiable for FileUploadV2 {
    fn get_id(&self) -> &str {
        &self.id
    }
}

impl FileUploadV2 {
    pub fn new(
        path: &str,
        file_name: &str,
        correlation_id: Option<String>,
        public_resource: bool,
        size: u64,
    ) -> Result<FileUploadV2, ServiceError> {
        let path = PathBuf::from_str(path).map_err(|e| ServiceError(format!("{e}")))?;

        let f = FileUploadV2 {
            content_type: mime_guess::from_path(path.as_path())
                .first_raw()
                .map(|ct| ct.into()),
            correlation_id,
            original_filename: file_name.to_string(),
            bookmarked: Some(false),
            name: Some(file_name.to_string()),
            extension: path.extension().map(|s| s.to_string_lossy().to_string()),
            size,
            public_resource,
            ..Default::default()
        };

        Ok(f)
    }
    pub fn is_image(&self) -> bool {
        self.content_type
            .as_ref()
            .filter(|ct| ct.starts_with("image"))
            .is_some()
    }
    pub fn is_supported_image(&self) -> bool {
        self.content_type
            .as_ref()
            .map(|ct| {
                matches!(
                    ct.to_ascii_lowercase().as_str(),
                    "image/png" | "image/jpeg" | "image/jpg" | "image/gif"
                )
            })
            .unwrap_or(false)
    }
    pub fn is_pdf(&self) -> bool {
        self.content_type
            .as_ref()
            .map(|ct| ct.eq_ignore_ascii_case("application/pdf"))
            .unwrap_or(false)
    }
}

impl Default for FileUploadV2 {
    fn default() -> Self {
        FileUploadV2 {
            id: IdGenerator.get(),
            content_type: Default::default(),
            original_filename: Default::default(),
            extension: Default::default(),
            creation_date: DateTime::now(),
            updated_date: Default::default(),
            thumb: Default::default(),
            thumbnail_id: Default::default(),
            size: Default::default(),
            public_resource: Default::default(),
            correlation_id: Default::default(),
            bookmarked: Some(false),
            bookmarked_date: None,
            name: Default::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct UploadFileRequestUriParams {
    pub correlation_id: Option<String>,
    pub id: Option<String>,
    pub is_public: Option<bool>,
    pub without_thumbnail: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct DownloadFileRequestUriParams {
    pub id: String,
}
#[derive(Debug, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct DownloadBulkRequestUriParams {
    pub ids: Vec<String>,
}

#[derive(Clone)]
pub struct FileRouterState {
    pub client: StoreClient,
    pub collection: StoreCollection,
}

#[derive(Deserialize, ToSchema, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct FindAllQueryParams {
    pub correlation_id: Option<String>,
}
