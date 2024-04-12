use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glob::glob;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::config::SherryConfigSourceJSON;
use crate::constants::HASHES_DIR;
use crate::helpers::initialize_json_file_with;

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatcherHashJSON {
    pub id: String,
    pub source_id: String,
    pub local_path: String,
    pub hashes: HashMap<String, String>,
}

pub fn get_file_hash(path: &PathBuf) -> String {
    if path.is_dir() {
        return "".to_string();
    }
    match std::fs::read(path) {
        Ok(content) => {
            seahash::hash(&content).to_string()
        }
        Err(_) => {
            "".to_string()
        }
    }
}

fn build_hashes(hashes_id: &String, source: &SherryConfigSourceJSON, local_path: &PathBuf) -> WatcherHashJSON {
    WatcherHashJSON {
        id: hashes_id.clone(),
        source_id: source.id.clone(),
        local_path: local_path.to_str().unwrap().to_string(),
        hashes: glob(Path::new(local_path).join("**").to_str().unwrap()).unwrap().filter_map(|v| {
            let res = v.unwrap();
            return if res.is_file() { Some((res.to_str().unwrap().to_string(), get_file_hash(&res))) } else { None };
        }).into_iter().collect(),
    }
}

pub fn get_hashes(dir: &PathBuf, source: &SherryConfigSourceJSON, local_path: &PathBuf, hashes_id: &String) -> Result<WatcherHashJSON, String> {
    initialize_json_file_with(&dir.join(HASHES_DIR).join(hashes_id), &|| { build_hashes(hashes_id, source, local_path) })
}