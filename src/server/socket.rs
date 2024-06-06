use std::sync::Arc;

use futures::future::BoxFuture;
use futures::FutureExt;
use rust_socketio::{Error, Payload};
use rust_socketio::asynchronous::{Client, ClientBuilder, ReconnectSettings};
use tokio::sync::Mutex;

use crate::config::SherryConfig;

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


fn folder_file_created_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Created: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_updated_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Updated: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_deleted_handler<'a>(ctx: Context, payload: Payload, socket: Client) -> BoxFuture<'a, ()> {
    log::info!("Folder File Deleted: {:?}", payload);

    async move {}.boxed()
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

                .on("FOLDER:FILE:CREATED", get_cb_with_ctx(&ctx, folder_file_created_handler))
                .on("FOLDER:FILE:UPDATED", get_cb_with_ctx(&ctx, folder_file_updated_handler))
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
