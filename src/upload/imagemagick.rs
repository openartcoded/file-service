use std::{error::Error, path::PathBuf, process::Stdio};

use tokio::process::Command;

use crate::{
    common::{constant::TMP_FS_PATH, util::IdGenerator},
    upload::ConvertType,
};

pub async fn convert_to(
    input_path: impl Into<PathBuf>,
    ct: ConvertType,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let extension = ct.to_str();
    let input_path: PathBuf = input_path.into();
    tracing::debug!("convert file {input_path:?}");
    let input_path_str = &input_path.display().to_string();
    let temp_dir = TMP_FS_PATH.join(IdGenerator.get());
    tokio::fs::create_dir_all(&temp_dir).await?;
    let output = Command::new("convert")
        .args([input_path_str, &format!("{extension}:-")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if output.status.success() {
        let input_path = input_path.with_extension(extension);
        let input_path = input_path
            .file_name()
            .ok_or("no file name")?
            .to_string_lossy();
        let path = format!("{}/{input_path}", temp_dir.display());
        let bytes = tokio::fs::read(&path).await?;
        tokio::spawn(async move {
            if let Err(e) = tokio::fs::remove_dir_all(&temp_dir).await {
                tracing::error!("could not remove temp thumb {e}");
            }
        });
        Ok(bytes)
    } else {
        Err(format!(
            "error! stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
        .into())
    }
}
