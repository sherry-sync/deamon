use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

const SOURCES_FILE: &str = "sources.json";

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum AccessRights {
    Read,
    Write,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SherrySourcesSourceJSON {
    id: String,
    access: AccessRights,
    max_file_size: u64,
    max_dir_size: u64,
    allow_dir: bool,
    allowed_file_names: Vec<String>,
    allowed_file_types: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SherrySourcesWatcherJSON {
    source: String,
    local_path: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SherrySourcesJSON {
    sources: HashMap<String, SherrySourcesSourceJSON>,
    watchers: Vec<SherrySourcesWatcherJSON>,
}


pub fn get_default_source_config() -> SherrySourcesJSON {
    SherrySourcesJSON {
        sources: HashMap::new(),
        watchers: Vec::new(),
    }
}

pub fn read_sources_config(dir: &Path) -> Result<SherrySourcesJSON, ()> {
    let sources_path = dir.join(SOURCES_FILE);
    let content = fs::read_to_string(sources_path);
    if content.is_err() {
        return Err(());
    }

    let sources: serde_json::Result<SherrySourcesJSON> = serde_json::from_str(&content.unwrap());
    if sources.is_err() {
        return Err(());
    }

    Ok(sources.unwrap())
}

pub fn initialize_config_dir(dir: &PathBuf) -> Result<(), ()> {
    if !dir.exists() && fs::create_dir_all(dir).is_err() {
        return Err(());
    }

    if read_sources_config(dir).is_err() && fs::write(dir.join(SOURCES_FILE), serde_json::to_string_pretty(&get_default_source_config()).unwrap()).is_err() {
        return Err(());
    }

    Ok(())
}

pub fn get_config_watch_cb(dir: PathBuf) -> impl Fn(notify::Result<notify::Event>) {
    let sources_dir = dir.join(SOURCES_FILE);
    move |res: notify::Result<notify::Event>| {
        if res.is_err() {
            return;
        }

        let event = res.unwrap();
        for path in &event.paths {
            if sources_dir.eq(path) {
                println!("{:?} {:?}", dir, event);
            }
        }
    }
}
