use mongodb::{Client, Database, bson::doc, options::ClientOptions};
use tracing::info;

use crate::{
    common::constant::SERVICE_APPLICATION_NAME,
    store::constant::{
        MONGO_ADMIN_DATABASE, MONGO_CONN_TIMEOUT, MONGO_HOST, MONGO_PASSWORD, MONGO_PORT,
        MONGO_USERNAME,
    },
};

use super::StoreError;

#[derive(Debug, Clone)]
pub struct StoreClient {
    client: Client,
}

impl StoreClient {
    pub async fn new() -> Result<StoreClient, StoreError> {
        let client = StoreClient::create_client().await?;

        Ok(StoreClient { client })
    }

    pub fn get_raw_client(&self) -> Client {
        self.client.clone()
    }

    pub fn get_db(&self, database_name: &str) -> Database {
        let client = self.get_raw_client();
        client.database(database_name)
    }

    #[tracing::instrument]
    async fn create_client() -> Result<Client, StoreError> {
        let mut client_options = ClientOptions::parse(format!(
            "mongodb://{}:{}@{}:{}",
            *MONGO_USERNAME, *MONGO_PASSWORD, *MONGO_HOST, *MONGO_PORT
        ))
        .await
        .map_err(|e| StoreError { msg: e.to_string() })?;
        client_options.app_name = Some(SERVICE_APPLICATION_NAME.to_string());

        client_options.server_selection_timeout = *MONGO_CONN_TIMEOUT;
        client_options.connect_timeout = *MONGO_CONN_TIMEOUT;

        tracing::info!("connecting to mongodb with options {client_options:?}");

        let client =
            Client::with_options(client_options).map_err(|e| StoreError { msg: e.to_string() })?;

        let _ = client
            .database(&MONGO_ADMIN_DATABASE)
            .run_command(doc! {"ping": 1})
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;

        info!("Successfully connected");
        Ok(client)
    }
}
