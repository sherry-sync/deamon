use std::collections::HashMap;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::string::ToString;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use futures::task::SpawnExt;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, Debouncer, FileIdMap, new_debouncer};
use rust_socketio::asynchronous::Client;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;
use tokio::sync::Mutex;

use crate::auth::{initialize_auth_config, read_auth_config, revalidate_auth, SherryAuthorizationConfigJSON, write_auth_config};
use crate::constants::{AUTH_FILE, CONFIG_FILE, DEFAULT_API_URL, DEFAULT_SOCKET_URL};
use crate::files::{initialize_json_file, read_json_file, write_json_file};
use crate::helpers::{ordered_map, str_err_prefix};
use crate::server::socket::initialize_socket;

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

fn read_main_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    read_json_file(dir.join(CONFIG_FILE))
}

struct RevalidateConfigMeta {
    pub invalid_watchers: Vec<SherryConfigWatcherJSON>,
    pub valid_watchers: Vec<SherryConfigWatcherJSON>,
    pub new_watchers: Vec<SherryConfigWatcherJSON>,
    pub updated_watchers: Vec<SherryConfigWatcherJSON>,
    pub deleted_watchers: Vec<SherryConfigWatcherJSON>,

    pub valid_sources: HashMap<String, SherryConfigSourceJSON>,
    pub invalid_sources: HashMap<String, SherryConfigSourceJSON>,
}

fn revalidate_config(new: &SherryConfigJSON, old: &SherryConfigJSON, auth: &SherryAuthorizationConfigJSON) -> (SherryConfigJSON, RevalidateConfigMeta) {
    let mut invalid_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut valid_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut new_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut updated_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut deleted_watchers: Vec<SherryConfigWatcherJSON> = vec![];

    for watcher in new.watchers.iter() {
        if !auth.records.contains_key(&watcher.user_id) || !new.sources.contains_key(&watcher.source) {
            invalid_watchers.push(watcher.clone());
            continue;
        }
        if old.watchers.iter().find(|w| w.hashes_id == watcher.hashes_id).is_none() {
            new_watchers.push(watcher.clone());
        } else {
            let old_watcher = old.watchers.iter().find(|w| w.hashes_id == watcher.hashes_id).unwrap();
            if old_watcher != watcher {
                updated_watchers.push(watcher.clone());
            } else {
                valid_watchers.push(watcher.clone());
            }
        }
    }
    for watcher in old.watchers.iter() {
        if new.watchers.iter().find(|w| w.hashes_id == watcher.hashes_id).is_none() {
            deleted_watchers.push(watcher.clone());
        }
    }

    let current_watchers = [valid_watchers.clone(), new_watchers.clone(), updated_watchers.clone()].concat();
    let mut valid_sources: HashMap<String, SherryConfigSourceJSON> = HashMap::new();
    let mut invalid_sources: HashMap<String, SherryConfigSourceJSON> = HashMap::new();

    for (key, source) in new.sources.iter() {
        if current_watchers.iter().find(|w| w.source.eq(key)).is_some() {
            valid_sources.insert(key.clone(), source.clone());
        } else {
            invalid_sources.insert(key.clone(), source.clone());
        }
    }

    let mut valid_config = new.clone();
    valid_config.watchers = current_watchers;
    valid_config.sources = valid_sources.clone();

    (
        valid_config,
        RevalidateConfigMeta {
            invalid_watchers,
            valid_watchers,
            new_watchers,
            deleted_watchers,
            updated_watchers,

            valid_sources,
            invalid_sources,
        }
    )
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

#[derive(Clone)]
pub struct SherryConfig {
    data: Arc<Mutex<SherryConfigJSON>>,
    auth: Arc<Mutex<SherryAuthorizationConfigJSON>>,
    dir: PathBuf,
    receiver: Arc<Mutex<Receiver<SherryConfigUpdateEvent>>>,

    watchers_debouncer: Option<Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>>,
    socket: Option<Arc<Mutex<Client>>>,

    #[allow(dead_code)] // This values should be saved to avoid it's destroying
    debouncer: Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>,
}

impl SherryConfig {
    async fn set_main(&self, new_value: &SherryConfigJSON) {
        *self.data.lock().await = new_value.clone();
        write_main_config(&self.dir, &new_value).unwrap()
    }
    async fn set_auth(&mut self, new_value: &SherryAuthorizationConfigJSON) {
        *self.auth.lock().await = new_value.clone();
        write_auth_config(&self.dir, &new_value).unwrap();
    }
    async fn apply_update(&mut self, update: &SherryConfigUpdateEvent) {
        let (valid_auth, auth_revalidation_meta) = revalidate_auth(&update.new.auth, &update.old.auth);
        let (valid_config, config_revalidation_meta) = revalidate_config(&update.new.data, &update.old.data, &valid_auth);

        if valid_auth != update.new.auth {
            self.set_auth(&valid_auth).await;
        }
        if valid_config != update.new.data {
            self.set_main(&valid_config).await;
        }

        if update.old.data != valid_config {
            log::info!("Updating watchers");

            let arc = self.clone().watchers_debouncer.unwrap();
            let mut debouncer = arc.lock().await;
            let watcher = debouncer.watcher();

            for w in [
                config_revalidation_meta.invalid_watchers,
                config_revalidation_meta.deleted_watchers,
                config_revalidation_meta.updated_watchers.clone(),
            ].concat() {
                watcher.unwatch(Path::new(&w.local_path)).unwrap();
            }
            for w in [
                config_revalidation_meta.new_watchers,
                config_revalidation_meta.updated_watchers
            ].concat() {
                watcher.watch(Path::new(&w.local_path), RecursiveMode::Recursive).unwrap()
            }
        }

        if !auth_revalidation_meta.deleted_users.is_empty()
            || !auth_revalidation_meta.new_users.is_empty()
            || !auth_revalidation_meta.updated_users.is_empty()
            || !auth_revalidation_meta.invalid_users.is_empty()
        {
            log::info!("Updating socket");
            let socket = self.clone().socket.unwrap();
            let mut current_socket = socket.lock().await;
            let err = current_socket.disconnect().await;
            match err {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Error disconnecting socket: {}", e);
                }
            }
            *current_socket = initialize_socket(
                &valid_config.socket_url,
                &valid_auth.records.iter().map(|(_, v)| v.access_token.clone()).collect(),
            ).await
        }
    }

    pub async fn new(dir: &PathBuf) -> Result<SherryConfig, ()> {
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

        let rt = tokio::runtime::Handle::current();
        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            None,
            move |res: DebounceEventResult| {
                rt.block_on(async {
                    if res.is_err() { return; }
                    let event = res.unwrap();
                    let old = SherryConfigUpdateData {
                        data: (*current_config.lock().await).clone(),
                        auth: (*current_auth.lock().await).clone(),
                    };
                    let mut new = SherryConfigUpdateData {
                        data: (*current_config.lock().await).clone(),
                        auth: (*current_auth.lock().await).clone(),
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
                        *current_config.lock().await = new.data.clone();
                        *current_auth.lock().await = new.auth.clone();
                        tx.send(SherryConfigUpdateEvent { old, new }).unwrap();
                    }
                });
            },
        ).unwrap();

        debouncer.watcher().watch(dir, RecursiveMode::Recursive).unwrap();

        Ok(SherryConfig {
            data,
            auth,
            dir: dir.clone(),
            receiver: Arc::new(Mutex::new(rx)),

            watchers_debouncer: None,
            socket: None,

            debouncer: Arc::new(Mutex::new(debouncer)),
        })
    }
    pub async fn get_main(&self) -> SherryConfigJSON {
        self.data.lock().await.clone()
    }
    pub async fn get_auth(&self) -> SherryAuthorizationConfigJSON {
        self.auth.lock().await.clone()
    }
    pub fn get_path(&self) -> PathBuf {
        self.dir.clone()
    }
    pub fn get_receiver(&self) -> Arc<Mutex<Receiver<SherryConfigUpdateEvent>>> {
        Arc::clone(&self.receiver)
    }
    pub async fn revalidate(&mut self) {
        let update = SherryConfigUpdateData {
            data: self.get_main().await,
            auth: self.get_auth().await,
        };
        self.apply_update(&SherryConfigUpdateEvent {
            old: update.clone(),
            new: update,
        }).await;
    }
    pub async fn listen(self_mutex: &Arc<Mutex<SherryConfig>>, socket: &Arc<Mutex<Client>>, watcher: &Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>) {
        async {
            let mut instance = self_mutex.lock().await;
            instance.watchers_debouncer = Some(watcher.clone());
            instance.socket = Some(socket.clone());
            let data = instance.get_main().await;
            let auth = instance.get_auth().await;
            instance.apply_update(&SherryConfigUpdateEvent {
                old: SherryConfigUpdateData {
                    data: SherryConfigJSON {
                        api_url: "".to_string(),
                        socket_url: "".to_string(),
                        sources: Default::default(),
                        watchers: vec![],
                        webhooks: vec![],
                    },
                    auth: SherryAuthorizationConfigJSON { default: "".to_string(), records: Default::default() },
                },
                new: SherryConfigUpdateData { data, auth },
            }).await;
        }.await;
        let receiver = async {
            let config = self_mutex.lock().await;
            let receiver = config.get_receiver();
            receiver
        }.await;
        for update in receiver.lock().await.iter() {
            log::info!("Config updated {:?}", &update.new);
            self_mutex.lock().await.apply_update(&update).await;
        }
    }
}
