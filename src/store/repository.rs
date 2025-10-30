use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::Collection;
use mongodb::bson::{self, Document, doc};
use mongodb::options::ReplaceOneModel;
pub use mongodb::options::{FindOneAndReplaceOptions, FindOptions};
use mongodb::results::{DeleteResult, InsertManyResult, InsertOneResult};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::StoreError;
use super::client::StoreClient;
#[derive(Serialize, Deserialize, Debug)]
#[allow(unused)]
pub struct Pageable {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub filter: Option<Document>,
    pub sort: Option<Document>,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct Page<T: Serialize + DeserializeOwned> {
    pub total_elements: i64,
    pub current_page: i64,
    pub next_page: Option<i64>,
    pub page_size: usize,
    pub content: Vec<T>,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct CursorPage<T: Serialize + DeserializeOwned> {
    pub current: Option<String>,
    pub next: Option<String>,
    pub content: Vec<T>,
}
#[derive(Debug, Clone)]
pub struct StoreRepository<T: Identifiable + Serialize + DeserializeOwned + Unpin + Send + Sync> {
    collection: Collection<T>,
    _db_name: String,
    _collection_name: String,
    _client: StoreClient,
}

#[allow(unused)]
pub trait Identifiable {
    fn get_id(&self) -> &str;
}

impl<T> StoreRepository<T>
where
    T: Identifiable + Serialize + DeserializeOwned + Unpin + Send + Sync,
{
    pub fn new(
        client: &StoreClient,
        collection: Collection<T>,
        collection_name: &str,
        tenant_id: &str,
    ) -> Self {
        StoreRepository {
            collection,
            _db_name: tenant_id.to_string(),
            _collection_name: collection_name.to_string(),
            _client: client.clone(),
        }
    }
}

impl<T> StoreRepository<T>
where
    T: Identifiable + Serialize + DeserializeOwned + Unpin + Send + Sync,
{
    pub fn get_repository(client: &StoreClient, collection_name: &str, tenant_id: &str) -> Self {
        let db = client.get_db(tenant_id);
        let collection = db.collection::<T>(collection_name);
        StoreRepository::new(client, collection, collection_name, tenant_id)
    }
    #[allow(unused)]
    pub fn get_collection_name(&self) -> &str {
        &self._collection_name
    }
    #[allow(unused)]
    pub fn get_db_name(&self) -> &str {
        &self._db_name
    }
}

impl<T> Repository<T> for StoreRepository<T>
where
    T: Identifiable + Serialize + DeserializeOwned + Unpin + Send + Sync + std::fmt::Debug,
{
    fn get_collection(&self) -> &Collection<T> {
        &self.collection
    }
    fn get_client(&self) -> &StoreClient {
        &self._client
    }
}

#[async_trait]
pub trait Repository<
    T: Identifiable + Serialize + DeserializeOwned + Unpin + Send + Sync + std::fmt::Debug,
>: std::fmt::Debug
{
    fn get_collection(&self) -> &Collection<T>;
    fn get_client(&self) -> &StoreClient;

    #[instrument(level = "debug")]
    async fn find_all(&self) -> Result<Vec<T>, StoreError> {
        self.find_all_batched(None).await
    }

    #[instrument(level = "debug")]
    async fn find_all_batched(&self, batch_size: Option<u32>) -> Result<Vec<T>, StoreError> {
        let collection = self.get_collection();
        let mut find = collection.find(doc! {});
        if let Some(batch_size) = batch_size {
            find = find.batch_size(batch_size);
        }
        let cursor = find.await.map_err(|e| StoreError { msg: e.to_string() })?;
        let collection: Vec<T> = cursor
            .try_collect()
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(collection)
    }
    #[instrument(level = "debug")]
    async fn count(&self, query: Option<Document>) -> Result<u64, StoreError> {
        let collection = self.get_collection();
        let count = collection
            .count_documents(query.unwrap_or_else(|| doc! {}))
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(count)
    }

    #[instrument(level = "debug")]
    async fn find_by_ids(&self, ids: Vec<String>) -> Result<Vec<T>, StoreError> {
        self.find_by_query(doc! {"_id": {"$in": ids}}, None).await
    }

    #[instrument(level = "debug")]
    async fn find_by_query(
        &self,
        query: Document,
        options: impl Into<Option<FindOptions>> + Send + std::fmt::Debug,
    ) -> Result<Vec<T>, StoreError> {
        let collection = self.get_collection();
        let cursor = collection
            .find(query)
            .with_options(
                options
                    .into()
                    .unwrap_or_else(|| FindOptions::builder().build()),
            )
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        cursor
            .try_collect()
            .await
            .map_err(|e| StoreError { msg: e.to_string() })
    }
    #[instrument(level = "debug")]
    async fn find_page_large_collection(
        &self,
        main_query: Option<Document>,
        next: Option<String>,
        limit: i64,
    ) -> Result<CursorPage<T>, StoreError> {
        self.find_page_large_collection_batched(main_query, next, limit, None)
            .await
    }

    #[instrument(level = "debug")]
    async fn find_page_large_collection_batched(
        &self,
        main_query: Option<Document>,
        next: Option<String>,
        limit: i64,
        batch_size: Option<u32>,
    ) -> Result<CursorPage<T>, StoreError> {
        let collection = self.get_collection();
        let query = {
            let cursor_query = if let Some(last_element_id) = next {
                doc! {
                    "_id": {
                        "$gt": last_element_id
                    }
                }
            } else {
                doc! {}
            };
            if let Some(main_query) = main_query {
                doc! {
                    "$and": [main_query, cursor_query]
                }
            } else {
                cursor_query
            }
        };

        let options = FindOptions::builder()
            .limit(Some(limit + 1))
            .sort(doc! { "_id": 1 })
            .batch_size(batch_size)
            .build();

        let cursor = collection
            .find(query)
            .with_options(options)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        let mut collection: Vec<T> = cursor
            .try_collect()
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;

        // we are at the end
        if (collection.len() as i64) < limit {
            let current = collection.first().map(|a| a.get_id().to_string());
            return Ok(CursorPage {
                next: None,
                content: collection,
                current,
            });
        }
        let next = collection.pop().map(|a| a.get_id().to_string());
        let current = collection.first().map(|a| a.get_id().to_string());
        Ok(CursorPage {
            next,
            current,
            content: collection,
        })
    }
    #[instrument(level = "debug")]
    async fn find_page(&self, pageable: Pageable) -> Result<Page<T>, StoreError> {
        let (limit, page) = (pageable.limit.unwrap_or(10), pageable.page.unwrap_or(0));
        let collection = self.get_collection();
        let query = if let Some(q) = pageable.filter {
            q
        } else {
            doc! {}
        };
        let count = self.count(Some(query.clone())).await? as i64;
        let skip = limit * page; // start at page 0
        if count <= skip {
            return Ok(Page {
                total_elements: 0,
                current_page: 0,
                next_page: None,
                page_size: 0,
                content: vec![],
            });
        }
        let options = FindOptions::builder()
            .skip(Some(skip as u64))
            .sort(pageable.sort)
            .limit(Some(limit))
            .build();
        let cursor = collection
            .find(query)
            .with_options(options)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        let collection: Vec<T> = cursor
            .try_collect()
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        let next_page = if count > (limit * (page + 1)) {
            Some(page + 1)
        } else {
            None
        };
        let page_size = collection.len();
        let page = Page {
            total_elements: count,
            content: collection,
            current_page: page,
            next_page,
            page_size,
        };
        Ok(page)
    }

    #[instrument(level = "debug")]
    async fn delete_many(&self, query: Option<Document>) -> Result<DeleteResult, StoreError> {
        let query = if let Some(q) = query {
            q
        } else {
            doc! {}
        };
        let res = self
            .get_collection()
            .delete_many(query)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn insert_many(&self, data: &Vec<T>) -> Result<InsertManyResult, StoreError> {
        let res = self
            .get_collection()
            .insert_many(data)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn insert_one(&self, data: &T) -> Result<InsertOneResult, StoreError> {
        let res = self
            .get_collection()
            .insert_one(data)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn find_by_id(&self, id: &str) -> Result<Option<T>, StoreError> {
        let collection = self.get_collection();
        let res = collection
            .find_one(doc! {"_id": id})
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn find_one(&self, query: Option<Document>) -> Result<Option<T>, StoreError> {
        let collection = self.get_collection();
        let res = collection
            .find_one(query.unwrap_or_else(|| doc! {})) // it should always have a document, FIXME
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn delete_by_id(&self, id: &str) -> Result<Option<T>, StoreError> {
        self.delete_one_by_query(doc! {"_id": id}).await
    }

    #[instrument(level = "debug")]
    async fn delete_one_by_query(&self, query: Document) -> Result<Option<T>, StoreError> {
        let collection = self.get_collection();
        let res = collection
            .find_one_and_delete(query)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    /// example:
    /// let filter = doc! { "targetUrl": "http://x.com", "status.type": "success" };
    /// let update = doc! { "$set": { "status.type": "archived" } };
    /// repository.update_many(filter, update).await?;
    #[instrument(level = "debug")]
    async fn update_many(&self, filter: Document, replace: Document) -> Result<u64, StoreError> {
        let collection = self.get_collection();
        let update_result = collection
            .update_many(filter, replace)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(update_result.modified_count)
    }

    #[instrument(level = "debug")]
    async fn upsert(&self, id: &str, entity: &T) -> Result<Option<T>, StoreError> {
        let collection = self.get_collection();

        let options = FindOneAndReplaceOptions::builder()
            .upsert(Some(true))
            .build();
        let res = collection
            .find_one_and_replace(doc! {"_id": id}, entity)
            .with_options(options)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;
        Ok(res)
    }

    #[instrument(level = "debug")]
    async fn upsert_many(&self, entities: &[T]) -> Result<(), StoreError> {
        let client = self.get_client().get_raw_client();

        let bulk_update = entities
            .iter()
            .filter_map(|e| bson::to_document(e).ok())
            .map(|e| {
                ReplaceOneModel::builder()
                    .namespace(self.get_collection().namespace())
                    .filter(doc! {"_id": e.get("_id")})
                    .replacement(e)
                    .upsert(true)
                    .build()
            })
            .collect::<Vec<_>>();
        client
            .bulk_write(bulk_update)
            .ordered(true)
            .await
            .map_err(|e| StoreError { msg: e.to_string() })?;

        Ok(())
    }
}
