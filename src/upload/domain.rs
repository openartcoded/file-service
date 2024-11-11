use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    common::{
        domain::ServiceError,
        util::{IdGenerator, StoreCollection},
    },
    store::StoreClient,
};

#[derive(Debug, Eq, PartialEq, PartialOrd, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileUpload {
    #[serde(rename = "_id")]
    pub id: String,
    pub creation_date: NaiveDateTime,
    pub updated_date: Option<NaiveDateTime>,
    pub content_type: Option<String>,
    pub thumbnail_id: Option<String>,
    pub original_filename: String,
    pub internal_name: String,
    pub extension: Option<String>,
    pub size: u64,
    pub public_resource: bool,
    pub correlation_id: Option<String>,
}

impl FileUpload {
    pub fn new(
        path: &str,
        file_name: &str,
        correlation_id: Option<String>,
        public_resource: bool,
        size: u64,
    ) -> Result<FileUpload, ServiceError> {
        let path = PathBuf::from_str(path).map_err(|e| ServiceError(format!("{e}")))?;

        let mut f = FileUpload {
            content_type: mime_guess::from_path(path.as_path())
                .first_raw()
                .map(|ct| ct.into()),
            correlation_id,
            original_filename: file_name.to_string(),
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

impl Default for FileUpload {
    fn default() -> Self {
        FileUpload {
            id: IdGenerator.get(),
            content_type: Default::default(),
            original_filename: Default::default(),
            internal_name: Default::default(),
            extension: Default::default(),
            creation_date: Local::now().naive_local(),
            updated_date: Default::default(),
            thumbnail_id: Default::default(),
            size: Default::default(),
            public_resource: Default::default(),
            correlation_id: Default::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct UploadFileRequestUriParams {
    pub correlation_id: Option<String>,
    pub id: Option<String>,
    pub is_public: Option<bool>,
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
