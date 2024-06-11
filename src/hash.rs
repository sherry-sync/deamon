use std::collections::HashMap;
use tokio::fs;
use std::path::PathBuf;

use glob::{glob, GlobResult};
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::config::SherryConfigSourceJSON;
use crate::constants::HASHES_DIR;
use crate::files::{initialize_json_file_with, write_json_file};
use crate::helpers::{get_now_as_millis, normalize_path, ordered_map, str_err_prefix};

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileHashJSON {
    pub hash: String,
    pub timestamp: i128,
    pub size: u64,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatcherHashJSON {
    pub id: String,
    pub source_id: String,
    pub local_path: String,
    #[serde(serialize_with = "ordered_map")]
    pub hashes: HashMap<String, FileHashJSON>,
}

pub async fn get_file_hash(path: &PathBuf) -> String {
    if path.is_dir() {
        return "".to_string();
    }
    match tokio::fs::read(path).await {
        Ok(content) => {
            seahash::hash(&content).to_string()
        }
        Err(_) => {
            "".to_string()
        }
    }
}

async fn build_hashes(hashes_id: &String, source: &SherryConfigSourceJSON, local_path: &PathBuf) -> WatcherHashJSON {
    let binding = local_path.join("**/*");
    let to_search = binding.to_str().unwrap();
    let glob_files = glob(to_search).unwrap();

    WatcherHashJSON {
        id: hashes_id.clone(),
        source_id: source.id.clone(),
        local_path: local_path.to_str().unwrap().to_string(),
        hashes: futures::future::join_all(glob_files
            .filter(|v: &GlobResult| v.as_ref().unwrap().is_file())
            .map(|v| async move {
                let res = normalize_path(&v.unwrap());
                (res.to_str().unwrap().to_string(), FileHashJSON {
                    hash: get_file_hash(&res).await,
                    timestamp: get_now_as_millis(),
                    size: res.metadata().unwrap().len(),
                })
            })).await.into_iter().collect(),
    }
}

pub async fn get_hashes(dir: &PathBuf, source: &SherryConfigSourceJSON, local_path: &PathBuf, hashes_id: &String) -> Result<WatcherHashJSON, String> {
    let hashes_dir = dir.join(HASHES_DIR);
    fs::create_dir_all(&hashes_dir).await.map_err(str_err_prefix("Error hashes dir creation"))?;
    initialize_json_file_with(&hashes_dir.join(format!("{}.json", hashes_id)), &|| async { build_hashes(hashes_id, source, local_path).await }).await
}

pub async fn update_hashes(dir: &PathBuf, hashes: &WatcherHashJSON) -> Result<(), String> {
    write_json_file(dir.join(HASHES_DIR).join(format!("{}.json", hashes.id)), hashes).await
}

pub async fn recreate_hashes(dir: &PathBuf, hashes_id: &String, source: &SherryConfigSourceJSON, local_path: &PathBuf) -> Result<WatcherHashJSON, String> {
    let hashes_dir = dir.join(HASHES_DIR);
    fs::create_dir_all(&hashes_dir).await.map_err(str_err_prefix("Error hashes dir creation"))?;
    let hashes = build_hashes(hashes_id, source, local_path).await;
    write_json_file(&hashes_dir.join(format!("{}.json", hashes_id)), &hashes).await?;
    Ok(hashes)
}