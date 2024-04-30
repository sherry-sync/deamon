use futures::future::BoxFuture;
use futures::FutureExt;
use rust_socketio::asynchronous::{Client, ClientBuilder};
use rust_socketio::Payload;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AuthPayload {
    authorizations: Vec<String>,
}

fn settings_change_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()>{
    async move {

    }.boxed()
}

fn file_upsert_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()>{
    async move {

    }.boxed()
}

fn file_remove_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()>{
    async move {

    }.boxed()
}

fn access_remove_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()>{
    async move {

    }.boxed()
}

pub async fn initialize_socket(url: String, tokens: Vec<String>) -> Client {
    ClientBuilder::new(url)
        .auth(serde_json::to_value(AuthPayload {
            authorizations: tokens
        }).unwrap())
        .on("folder:settings-changed", settings_change_handler)
        .on("folder:file-added", file_upsert_handler)
        .on("folder:file-changed", file_upsert_handler)
        .on("folder:file-removed", file_remove_handler)
        .on("folder:access-removed", access_remove_handler)
        .connect()
        .await
        .expect("Connection failed")
}
