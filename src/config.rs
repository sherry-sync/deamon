use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::string::ToString;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, Debouncer, FileIdMap, new_debouncer};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::auth::{initialize_auth_config, read_auth_config, SherryAuthorizationConfigJSON, write_auth_config};
use crate::constants::{AUTH_FILE, CONFIG_FILE, DEFAULT_API_URL, DEFAULT_SOCKET_URL};
use crate::files::{initialize_json_file, read_json_file, write_json_file};
use crate::helpers::{ordered_map, str_err_prefix};

#[derive(SerdeDiff, Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AccessRights {
    Read,
    Write,
    Owner,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigSourceJSON {
    pub id: String,
    pub name: String,
    pub access: AccessRights,
    pub user_id: String,
    pub owner_id: String,
    pub max_file_size: u64,
    pub max_dir_size: u64,
    pub allow_dir: bool,
    pub allowed_file_names: Vec<String>,
    pub allowed_file_types: Vec<String>,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigWatcherJSON {
    pub source: String,
    pub local_path: String,
    pub hashes_id: String,
    pub user_id: String,
    pub complete: bool,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigJSON {
    pub api_url: String,
    pub socket_url: String,
    // userId@folderId -> source
    #[serde(serialize_with = "ordered_map")]
    pub sources: HashMap<String, SherryConfigSourceJSON>,
    pub watchers: Vec<SherryConfigWatcherJSON>,
    pub webhooks: Vec<String>,
}

fn write_main_config(dir: &Path, config: &SherryConfigJSON) -> Result<(), String> {
    write_json_file(dir.join(CONFIG_FILE), config)
}

fn revalidate_sources(dir: &Path, config: &SherryConfigJSON) -> bool {
    let old_config = config.clone();
    let mut new_config = old_config.clone();
    let mut required_sources = HashSet::new();
    new_config.watchers = new_config.watchers
        .iter()
        .filter(|w| {
            if PathBuf::from(&w.local_path).exists() && new_config.sources.get(w.source.as_str()).is_some() {
                required_sources.insert(w.source.clone());
                true
            } else { false }
        })
        .cloned()
        .collect();

    new_config.sources = required_sources
        .iter()
        .fold(HashMap::new(), |mut acc, v| {
            acc.insert(v.clone(), new_config.sources.get(v).unwrap().clone());
            acc
        });

    if old_config != new_config {
        write_main_config(dir, &new_config).is_ok()
    } else {
        false
    }
}

fn read_main_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    read_json_file(dir.join(CONFIG_FILE))
}

fn initialize_main_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    initialize_json_file(dir.join(CONFIG_FILE), SherryConfigJSON {
        api_url: DEFAULT_API_URL.to_string(),
        socket_url: DEFAULT_SOCKET_URL.to_string(),
        sources: HashMap::new(),
        watchers: Vec::new(),
        webhooks: Vec::new(),
    })
}

fn initialize_config_dir(dir: &PathBuf) -> Result<(SherryConfigJSON, SherryAuthorizationConfigJSON), String> {
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(str_err_prefix("Error Creating Config Dir"))?;
    }

    Ok((initialize_main_config(&dir)?, initialize_auth_config(&dir)?))
}


fn cleanup_old_config(config: &SherryConfigJSON, watcher: &mut RecommendedWatcher) {
    for watcher_path in config.watchers.iter() {
        watcher.unwatch(Path::new(&watcher_path.local_path)).unwrap();
    }
}

fn apply_config_update(config_path: &PathBuf, config: &SherryConfigJSON, watcher: &mut RecommendedWatcher) {
    if revalidate_sources(config_path, config) {
        return;
    }

    for watcher_path in config.watchers.iter() {
        let path = Path::new(&watcher_path.local_path);
        if !path.exists() {
            if revalidate_sources(config_path, config) {
                break; // Config watcher will reset watchers, so don't need to set them here
            }
        }
        watcher.watch(path, RecursiveMode::Recursive).unwrap();
    }
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryConfigUpdateData {
    data: SherryConfigJSON,
    auth: SherryAuthorizationConfigJSON,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryConfigUpdateEvent {
    pub old: SherryConfigUpdateData,
    pub new: SherryConfigUpdateData,
}

#[derive(Debug)]
pub struct SherryConfig {
    data: Arc<Mutex<SherryConfigJSON>>,
    auth: Arc<Mutex<SherryAuthorizationConfigJSON>>,
    dir: PathBuf,
    receiver: Arc<Mutex<Receiver<SherryConfigUpdateEvent>>>,

    #[allow(dead_code)] // This values should be saved to avoid it's destroying
    debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
}

impl SherryConfig {
    pub fn new(dir: &PathBuf) -> Result<SherryConfig, ()> {
        let data = initialize_config_dir(dir);
        if data.is_err() { return Err(()); }
        let (data, auth) = data.unwrap();

        let data = Arc::new(Mutex::new(data));
        let auth = Arc::new(Mutex::new(auth));
        let (tx, rx) = channel::<SherryConfigUpdateEvent>();

        let current_config = Arc::clone(&data);
        let current_auth = Arc::clone(&auth);
        let config_dir = dir.clone();

        let config_path = dir.join(CONFIG_FILE);
        let auth_path = dir.join(AUTH_FILE);

        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            None,
            move |res: DebounceEventResult| {
                if res.is_err() { return; }
                let event = res.unwrap();
                let old = SherryConfigUpdateData {
                    data: (*current_config.lock()).clone(),
                    auth: (*current_auth.lock()).clone(),
                };
                let mut new = SherryConfigUpdateData {
                    data: (*current_config.lock()).clone(),
                    auth: (*current_auth.lock()).clone(),
                };
                for event in &event {
                    for path in &event.paths {
                        if config_path.eq(path) {
                            if let Ok(new_config) = read_main_config(&config_dir) {
                                new.data = new_config;
                            }
                        }
                        if auth_path.eq(path) {
                            if let Ok(new_config) = read_auth_config(&config_dir) {
                                new.auth = new_config;
                            }
                        }
                    }
                }

                if old != new {
                    *current_config.lock() = new.data.clone();
                    *current_auth.lock() = new.auth.clone();
                    tx.send(SherryConfigUpdateEvent { old, new }).unwrap();
                }
            },
        ).unwrap();

        debouncer.watcher().watch(dir, RecursiveMode::Recursive).unwrap();

        Ok(SherryConfig {
            data,
            auth,
            dir: dir.clone(),
            receiver: Arc::new(Mutex::new(rx)),
            debouncer,
        })
    }
    pub fn get_main(&self) -> SherryConfigJSON {
        self.data.lock().clone()
    }
    pub fn get_auth(&self) -> SherryAuthorizationConfigJSON {
        self.auth.lock().clone()
    }
    pub fn get_path(&self) -> PathBuf {
        self.dir.clone()
    }
    pub fn get_receiver(&self) -> Arc<Mutex<Receiver<SherryConfigUpdateEvent>>> {
        Arc::clone(&self.receiver)
    }
    pub fn set_main(&self, new_value: &SherryConfigJSON) {
        write_main_config(&self.dir, &new_value).unwrap()
    }
    pub fn set_auth(&self, new_value: &SherryAuthorizationConfigJSON) {
        write_auth_config(&self.dir, &new_value).unwrap()
    }
    pub fn revalidate(&self) {
        revalidate_sources(&self.dir, &self.get_main());
    }
    pub fn apply_update(&self, watcher: &mut RecommendedWatcher) {
        apply_config_update(&self.dir, &self.get_main(), watcher)
    }
    pub fn listen(dir: &PathBuf, receiver: &Receiver<SherryConfigUpdateEvent>, watcher: &mut RecommendedWatcher) {
        for update in receiver.iter() {
            log::info!("Config updated {:?}", &update.new);
            cleanup_old_config(&update.old.data, watcher);
            apply_config_update(dir, &update.new.data, watcher);
        }
    }
}

pub fn get_source_by_path<'a>(config: &'a SherryConfigJSON, path: &PathBuf) -> Option<&'a SherryConfigWatcherJSON> {
    config.watchers.iter().find_map(|w| {
        if path.starts_with(&w.local_path) {
            return Some(w);
        }
        return None;
    })
}