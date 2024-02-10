use parking_lot::{Mutex};
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use notify_debouncer_full::DebounceEventResult;
use serde::{Deserialize, Serialize};
use serde_diff::{SerdeDiff};

const CONFIG_FILE: &str = "config.json";

#[derive(SerdeDiff, Serialize, Deserialize, Copy, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub enum AccessRights {
    Read,
    Write,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigSourceJSON {
    pub id: String,
    pub access: AccessRights,
    pub max_file_size: u64,
    pub max_dir_size: u64,
    pub allow_dir: bool,
    pub allowed_file_names: Vec<String>,
    pub allowed_file_types: Vec<String>,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigWatcherJSON {
    pub source: String,
    pub local_path: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigJSON {
    pub sources: HashMap<String, SherryConfigSourceJSON>,
    pub watchers: Vec<SherryConfigWatcherJSON>,
    pub webhooks: Vec<String>,
}

impl SherryConfigJSON {
    pub fn clone(&self) -> SherryConfigJSON {
        serde_json::from_str(serde_json::to_string(self).unwrap().as_str()).unwrap()
    }
}

pub struct SherryConfigUpdateEvent {
    pub old: SherryConfigJSON,
    pub new: SherryConfigJSON,
}


pub fn get_default_config() -> SherryConfigJSON {
    SherryConfigJSON {
        sources: HashMap::new(),
        watchers: Vec::new(),
        webhooks: Vec::new(),
    }
}

fn get_config_string(dir: &Path) -> std::io::Result<String> {
    let f = OpenOptions::new()
        .read(true)
        .open(dir.join(CONFIG_FILE));
    if f.is_err() {
        return Err(f.err().unwrap());
    }
    let mut f = f.unwrap();
    let mut buf = String::new();
    f.read_to_string(&mut buf).unwrap();
    f.rewind().unwrap();
    Ok(buf)
}

fn write_config(dir: &Path, config: &SherryConfigJSON) -> Result<(), ()> {
    match fs::write(dir.join(CONFIG_FILE), serde_json::to_string_pretty(&config).unwrap()) {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn read_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    let content = get_config_string(dir);
    if content.is_err() {
        let err = content.err().unwrap().to_string();
        println!("Error Read: {}", err);
        return Err(err);
    }

    let sources: serde_json::Result<SherryConfigJSON> = serde_json::from_str(&content.unwrap());
    if sources.is_err() {
        let err = sources.err().unwrap().to_string();
        println!("Error Parse: {}", err);
        return Err(err);
    }

    Ok(sources.unwrap())
}



pub fn initialize_config_dir(dir: &PathBuf) -> Result<(), ()> {
    if !dir.exists() && fs::create_dir_all(dir).is_err() {
        return Err(());
    }

    let mut content = get_config_string(dir);
    if content.is_err() {
        if write_config(dir, &get_default_config()).is_err() {
            return Err(());
        } else {
            content = get_config_string(dir);
        }
    }

    let sources: serde_json::Result<SherryConfigJSON> = serde_json::from_str(&content.unwrap());
    if sources.is_err() {
        println!("Error: {}", sources.err().unwrap());
        return Err(());
    }

    Ok(())
}

pub fn get_config_watch_cb(dir: PathBuf, config_mutex: Arc<Mutex<SherryConfigJSON>>, sender: Sender<SherryConfigUpdateEvent>) -> impl Fn(DebounceEventResult) {
    let config_dir = dir.join(CONFIG_FILE);
    let dir = dir.clone();
    let owned_sender = sender.clone();
    move |res: DebounceEventResult| {
        if res.is_err() {
            return;
        }

        let event = res.unwrap();
        for event in &event {
            for path in &event.paths {
                if config_dir.eq(path) {
                    let mut data = config_mutex.lock();
                    if let Ok(config) = read_config(&dir) {
                        owned_sender.send(SherryConfigUpdateEvent {
                            old: (*data).clone(),
                            new: config.clone()
                        }).unwrap();

                        *data = config;
                        println!("Config updated {:?}", *data);
                    }
                }
            }
        }
    }
}
