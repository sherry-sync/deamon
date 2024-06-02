use futures::future::BoxFuture;
use futures::FutureExt;
use rust_socketio::asynchronous::{Client, ClientBuilder};
use rust_socketio::Payload;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AuthPayload {
    authorization: Vec<String>,
}

fn folder_created_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder Created: {:?}", payload);

    async move {}.boxed()
}

fn folder_updated_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder Updated: {:?}", payload);

    async move {}.boxed()
}

fn folder_deleted_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder Deleted: {:?}", payload);

    async move {}.boxed()
}


fn folder_permission_granted_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder Permission Granted: {:?}", payload);

    async move {}.boxed()
}

fn folder_permission_revoked_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder Permission Revoked: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_created_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder File Created: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_updated_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder File Updated: {:?}", payload);

    async move {}.boxed()
}

fn folder_file_deleted_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::info!("Folder File Deleted: {:?}", payload);

    async move {}.boxed()
}

fn error_handler(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
    log::error!("Socket Error: {:?}", payload);

    async move {}.boxed()
}

pub async fn initialize_socket(url: &String, tokens: &Vec<String>) -> Client {
    ClientBuilder::new(url)
        .opening_header("authorization", tokens.join(";"))
        .on("FOLDER:CREATED", folder_created_handler)
        .on("FOLDER:UPDATED", folder_updated_handler)
        .on("FOLDER:DELETED", folder_deleted_handler)

        .on("FOLDER:PERMISSION:GRANTED", folder_permission_granted_handler)
        .on("FOLDER:PERMISSION:REVOKED", folder_permission_revoked_handler)

        .on("FOLDER:FILE:CREATED", folder_file_created_handler)
        .on("FOLDER:FILE:UPDATED", folder_file_updated_handler)
        .on("FOLDER:FILE:DELETED", folder_file_deleted_handler)

        .on("error", error_handler)
        .connect()
        .await
        .expect("Connection failed")
}
