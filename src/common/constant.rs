use std::{env::var, path::PathBuf, str::FromStr, sync::LazyLock};

pub const FILE_SERVICE_COLLECTION_NAME: &str = "FILE_SERVICE_COLLECTION_NAME";
pub const TEMPL_SERVICE_COLLECTION_NAME: &str = "TEMPL_SERVICE_COLLECTION_NAME";
pub const TZ: &str = "TZ";

pub static THUMB_W: LazyLock<u32> = LazyLock::new(|| {
    var("THUMB_WIDTH")
        .ok()
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(300)
});

pub static THUMB_H: LazyLock<u32> = LazyLock::new(|| {
    var("THUMB_HEIGHT")
        .ok()
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(300)
});

pub static TMP_FS_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let temp_fs_folder = var("TMP_FS_PATH")
        .map(PathBuf::from)
        .expect("missing TMP_FS_PATH variable");
    if !temp_fs_folder.exists() {
        std::fs::create_dir_all(&temp_fs_folder).expect("could not create tmpfs folder!");
    }
    temp_fs_folder
});
pub static DEFAULT_TENANT: LazyLock<String> =
    LazyLock::new(|| std::env::var("DEFAULT_TENANT").unwrap_or_else(|_| "artcoded_test".into()));

pub static SERVICE_APPLICATION_NAME: LazyLock<String> = LazyLock::new(|| {
    std::env::var("SERVICE_APPLICATION_NAME").unwrap_or_else(|_| "file-service".into())
});

pub static SHARE_DRIVE_PATH_BUF: LazyLock<PathBuf> = LazyLock::new(|| {
    let share_drive_path: String = std::env::var("SHARE_DRIVE_PATH_BUF").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(SERVICE_APPLICATION_NAME.to_string())
            .display()
            .to_string()
    });
    tracing::info!("share path: {}", share_drive_path);
    let p = PathBuf::from_str(&share_drive_path)
        .expect("could not create path buf from share drive path");
    if !p.exists() {
        std::fs::create_dir(&share_drive_path).expect("could not create share_drive_path");
    }
    p
});

pub static SERVICE_HOST: LazyLock<String> =
    LazyLock::new(|| var("SERVICE_HOST").unwrap_or_else(|_| String::from("127.0.0.1")));
pub static SERVICE_PORT: LazyLock<String> =
    LazyLock::new(|| var("SERVICE_PORT").unwrap_or_else(|_| String::from("80")));

pub static BODY_SIZE_LIMIT: LazyLock<usize> = LazyLock::new(|| {
    (var("BODY_SIZE_LIMIT").unwrap_or_else(|_| format!("{}", 1024 * 1024 * 50)))
        .parse::<usize>()
        .expect("could not extract BODY_SIZE_LIMIT")
});

/*pub static CACHED_REDIS_CONNECTION_STRING: LazyLock<String> = LazyLock::new(|| {
    std::env::var("CACHED_REDIS_CONNECTION_STRING")
        .expect("CACHED_REDIS_CONNECTION_STRING must be set")
});*/