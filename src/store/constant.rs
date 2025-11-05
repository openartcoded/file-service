use std::{env, sync::LazyLock, time::Duration};

pub static MONGO_HOST: LazyLock<String> =
    LazyLock::new(|| env::var("MONGO_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()));
pub static MONGO_PORT: LazyLock<String> =
    LazyLock::new(|| env::var("MONGO_PORT").unwrap_or_else(|_| "27017".to_string()));
pub static MONGO_USERNAME: LazyLock<String> =
    LazyLock::new(|| env::var("MONGO_USERNAME").unwrap_or_else(|_| "root".to_string()));
pub static MONGO_PASSWORD: LazyLock<String> =
    LazyLock::new(|| env::var("MONGO_PASSWORD").unwrap_or_else(|_| "root".to_string()));
pub static MONGO_ADMIN_DATABASE: LazyLock<String> =
    LazyLock::new(|| env::var("MONGO_ADMIN_DATABASE").unwrap_or_else(|_| "admin".to_string()));

pub static MONGO_CONN_TIMEOUT: LazyLock<Option<Duration>> = LazyLock::new(|| {
    env::var("MONGO_CONN_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
});
