use std::{path::PathBuf, str::FromStr};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    common::{
        domain::ServiceError,
        util::{IdGenerator, StoreCollection},
    },
    store::{Identifiable, StoreClient},
};

#[derive(Debug, Eq, PartialEq, PartialOrd, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadV2 {
    #[serde(rename = "_id")]
    pub id: String,
    pub creation_date: DateTime<Utc>,
    pub updated_date: Option<DateTime<Utc>>,
    pub content_type: Option<String>,
    pub bookmarked: Option<bool>,
    pub bookmarked_date: Option<DateTime<Utc>>,
    pub name: Option<String>,
    pub thumbnail_id: Option<String>,
    pub original_filename: String,
    pub internal_name: String,
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
}

impl Default for FileUploadV2 {
    fn default() -> Self {
        FileUploadV2 {
            id: IdGenerator.get(),
            content_type: Default::default(),
            original_filename: Default::default(),
            internal_name: Default::default(),
            extension: Default::default(),
            creation_date: Local::now().to_utc(),
            updated_date: Default::default(),
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

#[derive(Clone, Debug)]
pub struct ShareDrive(pub String);

#[derive(Clone)]
pub struct FileRouterState {
    pub client: StoreClient,
    pub share_drive: ShareDrive,
    pub collection: StoreCollection,
}

#[derive(Deserialize, ToSchema, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct FindAllQueryParams {
    pub correlation_id: Option<String>,
}
