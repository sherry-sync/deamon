use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;
use tokio::fs;

use crate::config::SherryConfigSourceJSON;

const HASHES_DIR: &str = "hashes";

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatcherHashJSON {
    pub id: String,
    pub source_id: String,
    pub local_path: String,
    pub hashes: HashMap<String, String>,
}

fn parse_hashes(content: String) -> serde_json::Result<WatcherHashJSON> {
    serde_json::from_str::<WatcherHashJSON>(&content)
}

fn build_hashes(hashes_id: &String, source: &SherryConfigSourceJSON, local_path: &String) -> WatcherHashJSON {
    WatcherHashJSON {
        id: hashes_id.clone(),
        source_id: source.id.clone(),
        local_path: local_path.clone(),
        hashes: Default::default(),
    }
}

pub async fn get_hashes(dir: &Path, source: &SherryConfigSourceJSON, local_path: &String, hashes_id: &String) -> Option<WatcherHashJSON> {
    let hashes_path = dir.join(HASHES_DIR).join(hashes_id);
    let mut hashes_str = String::from("");
    if hashes_path.exists() {
        if let Ok(hashes_content) = fs::read_to_string(hashes_path).await {
            hashes_str = hashes_content
        }
    } else {}

    if hashes_str.is_empty() {
        return None;
    }

    match parse_hashes(hashes_str) {
        Ok(hashes) => Some(hashes),
        Err(_) => None
    }
}