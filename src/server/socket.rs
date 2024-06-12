use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::FutureExt;
use rust_socketio::{Error, Payload};
use rust_socketio::asynchronous::{Client, ClientBuilder, ReconnectSettings};
use tokio::sync::Mutex;

use crate::auth::SherryAuthorizationConfigJSON;
use crate::config::{SherryConfig, SherryConfigJSON, SherryConfigSourceJSON, SherryConfigWatcherJSON};
use crate::files::{delete_file, write_files_from_stream};
use crate::hash::{FileHashJSON, get_hashes, update_hashes};
use crate::helpers::normalize_path;
use crate::server::api::ApiClient;
use crate::server::types::ApiFileResponse;

type Context = Arc<Mutex<SocketClient>>;

fn folder_created_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Created: {:?}", payload);

    async move {}.boxed()
}

fn folder_updated_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Updated: {:?}", payload);

    async move {
        ctx.lock().await.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_deleted_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Deleted: {:?}", payload);

    async move {
        ctx.lock().await.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_permission_granted_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Permission Granted: {:?}", payload);

    async move {
        ctx.lock().await.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_permission_revoked_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Permission Revoked: {:?}", payload);

    async move {
        ctx.lock().await.config.lock().await.revalidate().await;
    }.boxed()
}

struct FilePayloadProcessResult {
    remote_file: ApiFileResponse,
    config: SherryConfigJSON,
    auth: SherryAuthorizationConfigJSON,
    dir: PathBuf,
    sources: HashMap<String, SherryConfigSourceJSON>,
    watchers_paths: Vec<(SherryConfigWatcherJSON, PathBuf)>,
    client: ApiClient,
}

async fn process_file_payload(ctx: Context, payload: Payload) -> Option<FilePayloadProcessResult> {
    let remote_file = match payload {
        Payload::Text(res) => serde_json::from_value::<ApiFileResponse>(res.first().unwrap().clone()).unwrap(),
        _ => { return None; }
    };
    let (config, auth, dir) = async {
        let c = ctx.lock().await;
        let c = c.config.lock().await;
        (c.get_main().await, c.get_auth().await, c.get_path())
    }.await;

    let source_id = &remote_file.sherry_id;
    let sources = config.sources.iter().filter_map(|(k, s)| {
        if &s.id == source_id {
            Some((k.clone(), s.clone()))
        } else {
            None
        }
    }).collect::<HashMap<String, SherryConfigSourceJSON>>();

    if sources.is_empty() {
        return None;
    };

    let watchers_paths = config.watchers.iter()
        .filter_map(|w| {
            if sources.contains_key(&w.source) {
                Some((w.clone(), PathBuf::from(&w.local_path).join(&remote_file.path)))
            } else {
                None
            }
        })
        .collect::<Vec<(SherryConfigWatcherJSON, PathBuf)>>();

    let mut user = None;
    for (_, source) in sources.iter() {
        user = auth.records.get(&source.user_id);
        if user.is_some() {
            break;
        }
    };

    if user.is_none() {
        return None;
    }
    let user = user.unwrap();

    let client = ApiClient::new(&config.api_url, &user.access_token);

    Some(FilePayloadProcessResult {
        remote_file,
        auth,
        config,
        dir,
        sources,
        watchers_paths,
        client,
    })
}

fn folder_file_upserted_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Upsert: {:?}", payload);

    async move {
        let result = match process_file_payload(ctx.clone(), payload).await {
            Some(res) => res,
            None => { return; }
        };
        let dir = result.dir;
        let remote_file = result.remote_file;
        let sources = result.sources;
        let watchers_paths = result.watchers_paths;
        let client = result.client;

        let file_content = client.get_file(&remote_file.sherry_id, &remote_file.path).await;
        if file_content.is_err() {
            return;
        }

        write_files_from_stream(&watchers_paths.iter().map(|(_, p)| p.clone()).collect(), file_content.unwrap().bytes_stream()).await.ok();

        let dir = dir.clone();
        futures::future::join_all(watchers_paths.iter().map(|(watcher, file_path)| {
            let dir = dir.clone();
            let local_path = PathBuf::from(&watcher.local_path);
            let remote_file = remote_file.clone();
            let source = sources.get(&watcher.source).unwrap();
            async move {
                let mut hashes = get_hashes(&dir, &source, &local_path, &watcher.hashes_id).await.unwrap();
                hashes.hashes.insert(normalize_path(&file_path).to_str().unwrap().to_string(), FileHashJSON {
                    hash: remote_file.hash.clone(),
                    timestamp: remote_file.updated_at,
                    size: remote_file.size,
                });
                update_hashes(&dir, &hashes).await.ok();
            }
        })).await;
    }.boxed()
}

fn folder_file_rename_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Renamed: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_deleted_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Deleted: {:?}", payload);

    async move {
        let result = match process_file_payload(ctx.clone(), payload).await {
            Some(res) => res,
            None => { return; }
        };
        let dir = result.dir;
        let sources = result.sources;
        let watchers_paths = result.watchers_paths;

        futures::future::join_all(watchers_paths.iter().map(|(watcher, file_path)| {
            let dir = dir.clone();
            let source = sources.get(&watcher.source).unwrap();
            let local_path = PathBuf::from(&watcher.local_path);
            async move {
                if let Err(_) = delete_file(file_path).await { return; }
                let mut hashes = get_hashes(&dir, &source, &local_path, &watcher.hashes_id).await.unwrap();
                hashes.hashes.remove(&normalize_path(&file_path).to_str().unwrap().to_string());
                update_hashes(&dir, &hashes).await.ok();
            }
        })).await;
    }.boxed()
}


fn error_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::error!("Socket Error: {:?}", payload);

    async move {}.boxed()
}

fn reconnect_handler<'a>(ctx: Context) -> BoxFuture<'a, ReconnectSettings> {
    log::info!("Socket Reconnect");

    async move {
        let mut config = {
            let mut client = ctx.lock().await;
            client.reconnect().await;
            let a = client.config.lock().await;
            a.clone()
        };

        config.reinitialize().await;
        ReconnectSettings::new()
    }.boxed()
}

fn get_cb_with_ctx<'a, R>(ctx: &Context, cb: fn(Context, Payload, Client) -> BoxFuture<'a, R>) -> impl FnMut(Payload, Client) -> BoxFuture<'a, R> {
    let ctx = ctx.clone();
    move |payload: Payload, socket: Client| {
        cb(ctx.clone(), payload, socket)
    }
}

fn get_reconnect_cb_with_ctx<'a>(ctx: &Context, cb: fn(Context) -> BoxFuture<'a, ReconnectSettings>) -> impl FnMut() -> BoxFuture<'a, ReconnectSettings> {
    let ctx = ctx.clone();
    move || {
        cb(ctx.clone())
    }
}

#[derive(Clone)]
pub struct SocketClient {
    pub _is_up: Arc<Mutex<bool>>,
    pub client: Arc<Mutex<Option<Client>>>,
    pub config: Arc<Mutex<SherryConfig>>,
}

impl SocketClient {
    async fn is_up(&self) -> bool {
        self._is_up.lock().await.clone()
    }
    async fn connect(&mut self) {
        let (data, auth) = async {
            let config = self.config.lock().await;
            (config.get_main().await, config.get_auth().await)
        }.await;

        let ctx = Arc::new(Mutex::new(self.clone()));
        let tokens = auth.records.iter().filter(|(_, v)| !v.expired).map(|(_, v)| v.access_token.clone()).collect::<Vec<String>>().join(";");
        let mut res: Result<Client, Error> = Err(Error::StoppedEngineIoSocket);

        while res.is_err() {
            res = ClientBuilder::new(&data.socket_url)
                .opening_header("authorization", tokens.clone())
                .on("FOLDER:CREATED", get_cb_with_ctx(&ctx, folder_created_handler))
                .on("FOLDER:UPDATED", get_cb_with_ctx(&ctx, folder_updated_handler))
                .on("FOLDER:DELETED", get_cb_with_ctx(&ctx, folder_deleted_handler))

                .on("FOLDER:PERMISSION:GRANTED", get_cb_with_ctx(&ctx, folder_permission_granted_handler))
                .on("FOLDER:PERMISSION:REVOKED", get_cb_with_ctx(&ctx, folder_permission_revoked_handler))

                .on("FOLDER:FILE:CREATED", get_cb_with_ctx(&ctx, folder_file_upserted_handler))
                .on("FOLDER:FILE:UPDATED", get_cb_with_ctx(&ctx, folder_file_upserted_handler))
                .on("FOLDER:FILE:RENAME", get_cb_with_ctx(&ctx, folder_file_rename_handler))
                .on("FOLDER:FILE:DELETED", get_cb_with_ctx(&ctx, folder_file_deleted_handler))

                .on("error", get_cb_with_ctx(&ctx, error_handler))
                .on_reconnect(get_reconnect_cb_with_ctx(&ctx, reconnect_handler))

                .reconnect_on_disconnect(true)
                .connect().await;
            if res.is_err() {
                log::warn!("Failed to connect to socket.io server, retrying in 10 seconds...");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        };

        *self.client.lock().await = Some(res.unwrap());
        *self._is_up.lock().await = true;
    }

    pub async fn new(config: &SherryConfig) -> Self {
        let mut res = Self {
            client: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(config.clone())),
            _is_up: Arc::new(Mutex::new(false)),
        };

        res.connect().await;

        res
    }

    pub async fn reconnect(&mut self) {
        *self._is_up.lock().await = false;
        match self.client.lock().await.clone() {
            Some(c) => {
                let _ = c.disconnect().await;
            }
            _ => {}
        }
        self.connect().await;
    }
}
