use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, Debouncer, FileIdMap, new_debouncer};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

const CONFIG_FILE: &str = "config.json";

#[derive(SerdeDiff, Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AccessRights {
    Read,
    Write,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
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

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigWatcherJSON {
    pub source_id: String,
    pub local_path: String,
    pub hashes_id: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherryConfigJSON {
    pub sources: HashMap<String, SherryConfigSourceJSON>,
    pub watchers: Vec<SherryConfigWatcherJSON>,
    pub webhooks: Vec<String>,
}

pub struct SherryConfigUpdateEvent {
    pub old: SherryConfigJSON,
    pub new: SherryConfigJSON,
}


fn get_default_config() -> SherryConfigJSON {
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

fn revalidate_sources(dir: &Path, config: &SherryConfigJSON) -> bool {
    let old_config = config.clone();
    let mut new_config = old_config.clone();
    let mut required_sources = HashSet::new();
    new_config.watchers = new_config.watchers
        .iter()
        .filter(|w| {
            if PathBuf::from(&w.local_path).exists() && new_config.sources.get(w.source_id.as_str()).is_some() {
                required_sources.insert(w.source_id.clone());
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
        write_config(dir, &new_config).is_ok()
    } else {
        false
    }
}

fn read_config(dir: &Path) -> Result<SherryConfigJSON, String> {
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

fn initialize_config_dir(dir: &PathBuf) -> Result<SherryConfigJSON, ()> {
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

    Ok(sources.unwrap())
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

#[derive(Debug)]
pub struct SherryConfig {
    data: Arc<Mutex<SherryConfigJSON>>,
    dir: PathBuf,
    receiver: Arc<Mutex<Receiver<SherryConfigUpdateEvent>>>,

    #[allow(dead_code)] // This values should be saved to avoid it's destroying
    debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
}

impl SherryConfig {
    pub fn new(dir: &PathBuf) -> Result<SherryConfig, ()> {
        let data = initialize_config_dir(dir);
        if data.is_err() { return Err(()); }

        let data = Arc::new(Mutex::new(data.unwrap()));
        let (tx, rx) = channel::<SherryConfigUpdateEvent>();

        let current_config = Arc::clone(&data);
        let config_dir = dir.clone();
        let config_path = dir.join(CONFIG_FILE);
        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            None,
            move |res: DebounceEventResult| {
                if res.is_err() { return; }
                let event = res.unwrap();
                for event in &event {
                    for path in &event.paths {
                        if config_path.eq(path) {
                            if let Ok(new_config) = read_config(&config_dir) {
                                let old_config = (*current_config.lock()).clone();

                                if new_config != old_config {
                                    *current_config.lock() = new_config.clone();
                                    tx.send(SherryConfigUpdateEvent {
                                        old: old_config,
                                        new: new_config,
                                    }).unwrap();
                                }
                            }
                        }
                    }
                }
            },
        ).unwrap();

        debouncer.watcher().watch(dir, RecursiveMode::Recursive).unwrap();

        Ok(SherryConfig {
            data,
            dir: dir.clone(),
            receiver: Arc::new(Mutex::new(rx)),
            debouncer,
        })
    }
    pub fn get(&self) -> SherryConfigJSON {
        self.data.lock().clone()
    }
    pub fn get_path(&self) -> PathBuf {
        self.dir.clone()
    }
    pub fn get_receiver(&self) -> Arc<Mutex<Receiver<SherryConfigUpdateEvent>>> {
        Arc::clone(&self.receiver)
    }
    pub fn set(&self, new_value: &SherryConfigJSON) {
        write_config(&self.dir, &new_value).unwrap()
    }
    pub fn revalidate(&self) {
        revalidate_sources(&self.dir, &self.get());
    }
    pub fn apply_update(&self, watcher: &mut RecommendedWatcher) {
        apply_config_update(&self.dir, &self.get(), watcher)
    }
    pub fn listen(dir: &PathBuf, receiver: &Receiver<SherryConfigUpdateEvent>, watcher: &mut RecommendedWatcher) {
        for update in receiver.iter() {
            log::info!("Config updated {:?}", &update.new);
            cleanup_old_config(&update.old, watcher);
            apply_config_update(dir, &update.new, watcher);
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