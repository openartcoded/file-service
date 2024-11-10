mod client;
mod constant;
mod repository;
use std::error::Error;
use std::fmt::Display;

pub use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

pub use client::*;
pub use repository::*;
#[derive(Debug, Serialize, Deserialize)]
pub struct StoreError {
    msg: String,
}

impl Error for StoreError {}

impl Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}
