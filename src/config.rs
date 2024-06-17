use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::string::ToString;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, Debouncer, FileIdMap, new_debouncer};
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;
use tokio::sync::Mutex;

use crate::auth::{initialize_auth_config, read_auth_config, revalidate_auth, SherryAuthorizationConfigJSON, write_auth_config};
use crate::constants::{AUTH_FILE, CONFIG_FILE, DEFAULT_API_URL, DEFAULT_SOCKET_URL, ENV_API_URL, ENV_SOCKET_URL};
use crate::files::{initialize_json_file, read_json_file, write_json_file};
use crate::helpers::{ordered_map, str_err_prefix};
use crate::server::api::ApiClient;
use crate::server::socket::SocketClient;
use crate::server::types::{ApiFolderPermissionAccessRights, ApiFolderResponse};
use crate::watchers::actualize_watchers;

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

async fn write_main_config(dir: &Path, config: &SherryConfigJSON) -> Result<(), String> {
    write_json_file(dir.join(CONFIG_FILE), config).await
}

async fn read_main_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    read_json_file(dir.join(CONFIG_FILE)).await
}

fn response_role_to_access(role: ApiFolderPermissionAccessRights) -> AccessRights {
    match role {
        ApiFolderPermissionAccessRights::Read => AccessRights::Read,
        ApiFolderPermissionAccessRights::Write => AccessRights::Write,
        ApiFolderPermissionAccessRights::Owner => AccessRights::Owner,
    }
}

fn response_to_folder(response: &ApiFolderResponse, user_id: &String) -> Result<SherryConfigSourceJSON, &'static str>
{
    Ok(SherryConfigSourceJSON {
        id: response.sherry_id.clone(),
        name: response.name.clone(),
        access: response_role_to_access(response.sherry_permission.iter().find(|p| p.user_id.eq(user_id)).ok_or("Invalid folder permission")?.role),
        user_id: user_id.clone(),
        owner_id: response.user_id.clone(),
        max_file_size: response.max_file_size,
        max_dir_size: response.max_dir_size,
        allow_dir: response.allow_dir,
        allowed_file_names: response.allowed_file_names.iter().map(|n| n.name.clone()).collect(),
        allowed_file_types: response.allowed_file_types.iter().map(|t| t._type.clone()).collect(),
    })
}

struct RevalidateConfigMeta {
    pub invalid_watchers: Vec<SherryConfigWatcherJSON>,
    pub valid_watchers: Vec<SherryConfigWatcherJSON>,
    pub new_watchers: Vec<SherryConfigWatcherJSON>,
    pub updated_watchers: Vec<SherryConfigWatcherJSON>,
    pub deleted_watchers: Vec<SherryConfigWatcherJSON>,

    pub valid_sources: HashMap<String, SherryConfigSourceJSON>,
    pub invalid_sources: HashMap<String, SherryConfigSourceJSON>,
    pub updated_sources: HashMap<String, SherryConfigSourceJSON>,
}

async fn revalidate_config(new: &SherryConfigJSON, old: &SherryConfigJSON, auth: &SherryAuthorizationConfigJSON, is_init: bool, dir: &PathBuf) -> (SherryConfigJSON, RevalidateConfigMeta) {
    let mut invalid_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut valid_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut new_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut updated_watchers: Vec<SherryConfigWatcherJSON> = vec![];
    let mut deleted_watchers: Vec<SherryConfigWatcherJSON> = vec![];

    for watcher in new.watchers.iter() {
        if !auth.records.contains_key(&watcher.user_id) || !new.sources.contains_key(&watcher.source) || !PathBuf::from(&watcher.local_path).exists() {
            invalid_watchers.push(watcher.clone());
            continue;
        }
        match old.watchers.iter().find(|w| w.local_path == watcher.local_path) {
            None => new_watchers.push(watcher.clone()),
            Some(old_watcher) => {
                if old_watcher != watcher {
                    updated_watchers.push(watcher.clone());
                } else {
                    valid_watchers.push(watcher.clone());
                }
            }
        }
    }
    for watcher in old.watchers.iter() {
        if new.watchers.iter().find(|w| w.hashes_id == watcher.hashes_id).is_none() {
            deleted_watchers.push(watcher.clone());
        }
    }

    log::info!("Invalid Watchers: {:?}", &invalid_watchers);
    log::info!("Valid Watchers: {:?}", &valid_watchers);
    log::info!("New Watchers: {:?}", &new_watchers);
    log::info!("Updated Watchers: {:?}", &updated_watchers);


    let mut current_watchers = [valid_watchers.clone(), new_watchers.clone(), updated_watchers.clone()].concat();
    let mut valid_sources: HashMap<String, SherryConfigSourceJSON> = HashMap::new();
    let mut updated_sources: HashMap<String, SherryConfigSourceJSON> = HashMap::new();
    let mut invalid_sources: HashMap<String, SherryConfigSourceJSON> = HashMap::new();

    for (key, source) in new.sources.iter() {
        let source = source.clone();

        if current_watchers.iter().find(|w| w.source.eq(key)).is_none() {
            invalid_sources.insert(key.clone(), source);
            continue;
        }

        match ApiClient::new(&new.api_url, &auth.records.get(&source.user_id).unwrap().access_token).get_folder(&source.id).await {
            Ok(folder) => {
                match response_to_folder(&folder, &source.user_id) {
                    Ok(actual_source) => {
                        if actual_source != source {
                            updated_sources.insert(key.clone(), actual_source);
                        } else {
                            valid_sources.insert(key.clone(), actual_source);
                        }
                    }
                    Err(_) => {
                        invalid_sources.insert(key.clone(), source);
                    }
                }
            }
            Err(_) => {
                invalid_sources.insert(key.clone(), source);
                current_watchers.retain(|w| {
                    if w.source.eq(key) {
                        invalid_watchers.push(w.clone());
                        false
                    } else {
                        true
                    }
                });
            }
        }
    }

    let actualize_result = actualize_watchers(
        dir,
        new,
        &auth.records,
        &valid_sources,
        &match is_init {
            true => current_watchers.clone(),
            false => current_watchers.iter()
                .filter(|w| w.complete == false)
                .map(|w| w.clone())
                .collect(),
        },
    ).await;
    current_watchers.retain(|w| {
        if actualize_result.invalid_watchers.contains(w) {
            invalid_watchers.push(w.clone());
            false
        } else {
            true
        }
    });
    current_watchers = current_watchers.iter().map(|w|
        actualize_result.valid_watchers.iter().find(|ww| ww.local_path == w.local_path).get_or_insert(w).clone()
    ).collect::<Vec<SherryConfigWatcherJSON>>();

    let mut valid_config = new.clone();
    valid_config.watchers = current_watchers;
    valid_config.sources = valid_sources.clone().into_iter().chain(updated_sources.clone()).collect();

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
            updated_sources,
        }
    )
}

async fn initialize_main_config(dir: &Path) -> Result<SherryConfigJSON, String> {
    initialize_json_file(dir.join(CONFIG_FILE), SherryConfigJSON {
        api_url: env::var(ENV_API_URL).unwrap_or(DEFAULT_API_URL.to_string()),
        socket_url: env::var(ENV_SOCKET_URL).unwrap_or(DEFAULT_SOCKET_URL.to_string()),
        sources: HashMap::new(),
        watchers: Vec::new(),
        webhooks: Vec::new(),
    }).await
}

async fn initialize_config_dir(dir: &PathBuf) -> Result<(SherryConfigJSON, SherryAuthorizationConfigJSON), String> {
    if !dir.exists() {
        tokio::fs::create_dir_all(dir).await.map_err(str_err_prefix("Error Creating Config Dir"))?;
    }

    Ok((initialize_main_config(&dir).await?, initialize_auth_config(&dir).await?))
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

    watchers_debouncer: Arc<Mutex<Option<Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>>>>,
    socket: Arc<Mutex<Option<Arc<Mutex<SocketClient>>>>>,

    debouncer: Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>,
}

impl SherryConfig {
    async fn set_main(&self, new_value: &SherryConfigJSON) {
        *self.data.lock().await = new_value.clone();
    }
    async fn set_auth(&mut self, new_value: &SherryAuthorizationConfigJSON) {
        *self.auth.lock().await = new_value.clone();
    }
    async fn commit(&self) {
        write_main_config(&self.dir, &self.get_main().await).await.unwrap();
        write_auth_config(&self.dir, &self.get_auth().await).await.unwrap();
    }

    async fn apply_update(&mut self, update: &SherryConfigUpdateEvent, is_init: bool) {
        let (valid_auth, auth_revalidation_meta) = revalidate_auth(&update.new.auth, &update.old.auth, &update.new.data).await;
        let (valid_config, config_revalidation_meta) = revalidate_config(&update.new.data, &update.old.data, &valid_auth, is_init, &self.get_path()).await;

        let mut should_commit = false;
        if valid_auth != update.new.auth {
            self.set_auth(&valid_auth).await;
            should_commit = true;
        }
        if valid_config != update.new.data {
            self.set_main(&valid_config).await;
            should_commit = true;
        }
        if should_commit {
            self.commit().await;
        }

        if update.old.data != valid_config {
            log::info!("Updating watchers");

            let debouncer = self.get_data_debouncer().await;
            let mut debouncer = debouncer.lock().await;
            let watcher = debouncer.watcher();

            for w in [
                config_revalidation_meta.invalid_watchers,
                config_revalidation_meta.deleted_watchers,
                config_revalidation_meta.updated_watchers.clone(),
            ].concat() {
                watcher.unwatch(Path::new(&w.local_path)).ok();
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
            self.get_socket().await.lock().await.reconnect().await;
        }
    }
    pub async fn new(dir: &PathBuf) -> Result<SherryConfig, ()> {
        let data = initialize_config_dir(dir).await;
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
        let debouncer = new_debouncer(
            Duration::from_millis(1000),
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
                                if let Ok(new_config) = read_main_config(&config_dir).await {
                                    new.data = new_config;
                                } else {
                                    let _ = write_main_config(&config_dir, &old.data).await;
                                }
                            }
                            if auth_path.eq(path) {
                                if let Ok(new_config) = read_auth_config(&config_dir).await {
                                    new.auth = new_config;
                                } else {
                                    let _ = write_auth_config(&config_dir, &old.auth).await;
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

        Ok(SherryConfig {
            data,
            auth,
            dir: dir.clone(),
            receiver: Arc::new(Mutex::new(rx)),

            watchers_debouncer: Arc::new(Mutex::new(None)),
            socket: Arc::new(Mutex::new(None)),

            debouncer: Arc::new(Mutex::new(debouncer)),
        })
    }
    pub async fn get_main(&self) -> SherryConfigJSON {
        self.data.lock().await.clone()
    }
    pub async fn get_auth(&self) -> SherryAuthorizationConfigJSON {
        self.auth.lock().await.clone()
    }
    async fn get_data_debouncer(&self) -> Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>> {
        let a = self.watchers_debouncer.lock().await;
        a.clone().unwrap()
    }
    async fn get_socket(&self) -> Arc<Mutex<SocketClient>> {
        let a = self.socket.lock().await;
        a.clone().unwrap()
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
        }, false).await;
    }
    pub async fn reinitialize(&mut self) {
        log::info!("Reinitialize state");
        {
            let debouncer = self.get_data_debouncer().await;
            let mut debouncer = debouncer.lock().await;
            let data_watcher = debouncer.watcher();
            self.get_main().await.watchers.iter().for_each(|w| {
                let _ = data_watcher.unwatch(Path::new(&w.local_path));
            });
        }

        let data = self.get_main().await;
        let auth = self.get_auth().await;
        self.apply_update(&SherryConfigUpdateEvent {
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
        }, true).await;
    }
    pub async fn listen(self_mutex: &Arc<Mutex<SherryConfig>>, socket: &Arc<Mutex<SocketClient>>, watcher: &Arc<Mutex<Debouncer<RecommendedWatcher, FileIdMap>>>) {
        async {
            let mut instance = self_mutex.lock().await;
            { instance.debouncer.lock().await.watcher().watch(&instance.get_path(), RecursiveMode::Recursive).unwrap(); }
            *instance.watchers_debouncer.lock().await = Some(watcher.clone());
            *instance.socket.lock().await = Some(socket.clone());
            instance.reinitialize().await
        }.await;
        let receiver = async {
            let config = self_mutex.lock().await;
            let receiver = config.get_receiver();
            receiver
        }.await;
        for update in receiver.lock().await.iter() {
            log::info!("Config updated {:?}", &update.new);
            self_mutex.lock().await.apply_update(&update, false).await;
        }
    }
}
