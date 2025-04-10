use mongodb::{Client, Database, bson::doc, options::ClientOptions};
use tracing::info;

use std::{env::var, time::Duration};

use crate::store::constant::{
    MONGO_ADMIN_DATABASE, MONGO_CONN_TIMEOUT, MONGO_HOST, MONGO_PASSWORD, MONGO_PORT,
    MONGO_USERNAME,
};

use super::StoreError;

#[derive(Debug, Clone)]
pub struct StoreClient {
    application_name: String,
    client: Client,
}

impl StoreClient {
    pub async fn new(application_name: String) -> Result<StoreClient, StoreError> {
        let client = StoreClient::create_client(application_name.clone()).await?;

        Ok(StoreClient {
            client,
            application_name,
        })
    }

    pub fn get_raw_client(&self) -> Client {
        self.client.clone()
    }

    pub fn get_application_name(&self) -> &str {
        &self.application_name
    }
    pub fn get_db(&self, database_name: &str) -> Database {
        let client = self.get_raw_client();
        client.database(database_name)
    }

    #[tracing::instrument]
    async fn create_client(application_name: String) -> Result<Client, StoreError> {
        let mongo_host = var(MONGO_HOST).unwrap_or_else(|_| String::from("127.0.0.1"));
        let mongo_port = var(MONGO_PORT).unwrap_or_else(|_| String::from("27017"));
        let mongo_username = var(MONGO_USERNAME).unwrap_or_else(|_| String::from("root"));
        let mongo_password = var(MONGO_PASSWORD).unwrap_or_else(|_| String::from("root"));
        let mongo_admin_db = var(MONGO_ADMIN_DATABASE).unwrap_or_else(|_| String::from("admin"));
        let mut client_options = ClientOptions::parse(format!(
            "mongodb://{mongo_username}:{mongo_password}@{mongo_host}:{mongo_port}"
        ))
        .await
        .map_err(|e| StoreError { msg: e.to_string() })?;
        client_options.app_name = Some(application_name);
        let timeout = var(MONGO_CONN_TIMEOUT)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);
        client_options.server_selection_timeout = timeout;
        client_options.connect_timeout = timeout;

        tracing::info!("connecting to mongodb with options {client_options:?}");

        let client =
            Client::with_options(client_options).map_err(|e| StoreError { msg: e.to_string() })?;

        let _ = client
            .database(&mongo_admin_db)
            .run_command(doc! {"ping": 1})
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;

        info!("Successfully connected");
        Ok(client)
    }
}
