use std::ops::Deref;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::FutureExt;
use rust_socketio::{Error, Payload};
use rust_socketio::asynchronous::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;
use tokio::sync::Mutex;

use crate::config::SherryConfig;

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AuthPayload {
    authorization: Vec<String>,
}

#[derive(Clone)]
pub struct SocketClient {
    client: Arc<Mutex<Client>>,
}

#[derive(Clone)]
struct HandlerContext {
    config: Arc<Mutex<SherryConfig>>,
}

fn folder_created_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    async move {}.boxed()
}

fn folder_updated_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Updated: {:?}", payload);

    async move {
        ctx.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_deleted_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Deleted: {:?}", payload);

    async move {
        ctx.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_permission_granted_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Permission Granted: {:?}", payload);

    async move {
        ctx.config.lock().await.revalidate().await;
    }.boxed()
}

fn folder_permission_revoked_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder Permission Revoked: {:?}", payload);

    async move {
        ctx.config.lock().await.revalidate().await;
    }.boxed()
}


fn folder_file_created_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Created: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_updated_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Updated: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_deleted_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Deleted: {:?}", payload);

    async move {}.boxed()
}


fn error_handler<'a>(ctx: Arc<HandlerContext>, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::error!("Socket Error: {:?}", payload);

    async move {}.boxed()
}


fn get_cb_with_ctx<'a>(ctx: &HandlerContext, cb: fn(Arc<HandlerContext>, Payload, Client) -> BoxFuture<'a, ()>) -> impl Fn(Payload, Client) -> BoxFuture<'a, ()> {
    let ctx = Arc::new(ctx.clone());
    move |payload: Payload, socket: Client| {
        cb(ctx.clone(), payload, socket)
    }
}

impl SocketClient {
    async fn create_connection(config: &SherryConfig) -> Client {
        let data = config.get_main().await;
        let auth = config.get_auth().await;
        let ctx = HandlerContext { config: Arc::new(Mutex::new(config.clone())) };
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

                .on("FOLDER:FILE:CREATED", get_cb_with_ctx(&ctx, folder_file_created_handler))
                .on("FOLDER:FILE:UPDATED", get_cb_with_ctx(&ctx, folder_file_updated_handler))
                .on("FOLDER:FILE:DELETED", get_cb_with_ctx(&ctx, folder_file_deleted_handler))

                .on("error", get_cb_with_ctx(&ctx, error_handler)).connect().await;
            if res.is_err() {
                log::warn!("Failed to connect to socket.io server, retrying in 30 seconds...");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        };
        res.unwrap()
    }

    pub async fn new(config: &SherryConfig) -> Self {
        Self {
            client: Arc::new(Mutex::new(Self::create_connection(config).await)),
        }
    }

    pub async fn get_client(&self) -> Client {
        self.client.lock().await.deref().clone()
    }

    pub async fn reconnect(&self, config: &SherryConfig) {
        let mut client = self.client.lock().await;
        let _ = client.disconnect().await;
        *client = Self::create_connection(config).await;
    }
}
