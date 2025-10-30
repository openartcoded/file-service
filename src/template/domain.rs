use std::ops::{Deref, DerefMut};

use chrono::{DateTime, Local, Utc};
use std::fmt::Display;
use utoipa::{IntoParams, ToSchema};

use ::serde::{Deserialize, Serialize};

use crate::{
    common::util::{IdGenerator, StoreCollection},
    store::{Identifiable, StoreClient},
};

#[derive(Debug, PartialEq, PartialOrd, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemplateV2 {
    #[serde(rename = "_id")]
    pub id: String,
    pub creation_date: DateTime<Utc>,
    pub updated_date: Option<DateTime<Utc>>,
    pub file_id: String,
    pub template_type: TemplateType,
    pub template_context: Context,
    pub title: String,
    pub description: Option<String>,
}

impl Identifiable for TemplateV2 {
    fn get_id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, PartialEq, PartialOrd, Serialize, Deserialize, Copy, Clone, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TemplateType {
    Html,
    Xml,
}

#[derive(Debug, PartialEq, PartialOrd, Serialize, Deserialize, Copy, Clone, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Context {
    Invoice,
    Peppol,
    CV,
}
impl Display for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Context::Invoice => write!(f, "INVOICE"),
            Context::CV => write!(f, "CV"),
            Context::Peppol => write!(f, "PEPPOL"),
        }
    }
}
impl Display for TemplateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateType::Html => write!(f, "HTML"),
            TemplateType::Xml => write!(f, "XML"),
        }
    }
}
#[derive(Deserialize, ToSchema, IntoParams)]
pub struct ContextQuery {
    pub context: Context,
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct TemplateTypeQuery {
    pub template_type: TemplateType,
}
#[derive(Serialize, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderRequest {
    pub template_id: String,
    pub context: serde_json::Value,
    pub file_name: String,
    pub template_context: Context,
}
pub struct TemplateWrapper(pub TemplateV2);

impl Deref for TemplateWrapper {
    type Target = TemplateV2;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for TemplateWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, PartialEq, PartialOrd, Serialize, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemplateUpsert {
    #[serde(rename = "_id")]
    pub id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub template_type: TemplateType,
    pub template_context: Context,
}

impl Default for TemplateWrapper {
    fn default() -> Self {
        Self(TemplateV2 {
            id: IdGenerator.get(),
            creation_date: Local::now().to_utc(),
            updated_date: Default::default(),
            file_id: Default::default(),
            template_type: TemplateType::Html,
            template_context: Context::Invoice,
            title: Default::default(),
            description: Default::default(),
        })
    }
}
#[derive(Clone)]
pub struct TemplRouterState {
    pub client: StoreClient,
    pub collection: StoreCollection,
}
