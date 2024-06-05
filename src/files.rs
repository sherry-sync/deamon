use std::future::Future;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::helpers::str_err_prefix;

pub async fn write_json_file<T, P: AsRef<Path>>(path: P, value: &T) -> Result<(), String>
    where
        T: ?Sized + serde::Serialize,
{
    fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(str_err_prefix("Error JSON Encode"))?,
    ).await.map_err(str_err_prefix("Error File Write"))
}

pub async fn get_file_string<P: AsRef<Path>>(path: P) -> Result<String, String> {
    let mut buf = String::new();
    fs::File::open(path).await
        .map_err(str_err_prefix("Error File Open"))?
        .read_to_string(&mut buf).await
        .map_err(str_err_prefix("Error Read String"))?;
    Ok(buf)
}

pub async fn read_json_file<T, P: AsRef<Path>>(path: P) -> Result<T, String>
    where
        T: DeserializeOwned,
{
    serde_json::from_str(&get_file_string(path).await?)
        .map_err(str_err_prefix("Error JSON Parse"))
}

pub async fn initialize_json_file<T, P: AsRef<Path>>(path: P, default: T) -> Result<T, String>
    where
        T: DeserializeOwned + Serialize,
{
    match read_json_file(&path).await {
        Ok(v) => Ok(v),
        Err(_) => {
            write_json_file(&path, &default).await?;
            Ok(default)
        }
    }
}

pub async fn initialize_json_file_with<T, P: AsRef<Path>, C, Fut>(path: P, default: &C) -> Result<T, String>
    where
        T: DeserializeOwned + Serialize,
        C: Fn() -> Fut,
        Fut: Future<Output=T>,
{
    match read_json_file(&path).await {
        Ok(v) => Ok(v),
        Err(_) => {
            let value = default().await;
            write_json_file(&path, &value).await?;
            Ok(value)
        }
    }
}
