use std::fmt::Display;

use reqwest::{Body, Method, multipart, RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;
use serde_json::json;
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::event::file_event::{SyncEvent, SyncEventKind};

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiAuthResponse {
    pub user_id: String,
    pub email: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64, // timestamp in seconds
}


#[derive(SerdeDiff, Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ApiFolderPermissionAccessRights {
    Read,
    Write,
    Owner,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderPermissionResponse {
    pub sherry_permission_id: String,
    pub role: ApiFolderPermissionAccessRights,
    pub sherry_id: String,
    pub user_id: String,
}


#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderAllowedFileNameResponse {
    pub file_name_id: String,
    pub name: String,
    pub sherry_id: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderAllowedFileTypeResponse {
    pub file_type_id: String,
    pub _type: String,
    pub sherry_id: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderResponse {
    pub sherry_id: String,
    pub name: String,
    pub allow_dir: bool,
    pub user_id: String,
    pub max_file_size: u64,
    pub max_dir_size: u64,
    pub allowed_file_names: Vec<ApiFolderAllowedFileNameResponse>,
    pub allowed_file_types: Vec<ApiFolderAllowedFileTypeResponse>,
    pub sherry_permission: Vec<ApiFolderPermissionResponse>,
}


pub struct ApiClient {
    base: String,
    auth: String,
}

impl ApiClient {
    fn build_url<T>(&self, path: T) -> Url
        where
            T: Into<String> + Display,
    {
        Url::parse(&format!("{}{}", self.base, path)).unwrap()
    }
    fn get_client<T>(&self, method: Method, path: T) -> RequestBuilder
        where
            T: Into<String> + Display,
    {
        reqwest::Client::new()
            .request(method, self.build_url(path))
            .header("Authorization", format!("Bearer {}", &self.auth))
    }
    pub async fn send_file(&self, event: &SyncEvent) -> anyhow::Result<String> {
        let mut form = multipart::Form::new()
            .text("eventType", event.kind.to_string())
            .text("fileName", event.sync_path.to_string())
            .text("fileType", event.file_type.to_string());
        if event.kind == SyncEventKind::Create || event.kind == SyncEventKind::Update {
            form = form
                .text("fileHash", event.update_hash.to_string())
                .part("file", multipart::Part::stream(Body::wrap_stream(FramedRead::new(
                    File::open(&event.local_path).await?,
                    BytesCodec::new(),
                ))));
        }

        Ok(self.get_client(Method::POST, "/events").multipart(form).send().await?.text().await?)
    }

    pub async fn refresh_token(&self, refresh_token: &String) -> Result<ApiAuthResponse, reqwest::Error> {
        self.get_client(Method::POST, "/auth/refresh")
            .json(&json!({"refreshToken": refresh_token}))
            .send().await?
            .json::<ApiAuthResponse>().await
    }

    pub async fn get_folder(&self, folder_id: &String) -> Result<ApiFolderResponse, reqwest::Error> {
        self.get_client(Method::GET, format!("/sherry/{folder_id}")).send().await?.json::<ApiFolderResponse>().await
    }

    pub fn new(base: &String, auth: &String) -> Self {
        Self {
            base: base.clone(),
            auth: auth.clone(),
        }
    }
}
