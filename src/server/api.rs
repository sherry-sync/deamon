use std::env;
use std::fmt::Display;

use log4rs::append::Append;
use reqwest::{Body, Method, multipart, RequestBuilder, Url};
use serde_json::json;
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::constants::{DEFAULT_API_URL, ENV_API_URL};
use crate::event::file_event::{SyncEvent, SyncEventKind};
use crate::server::types::{ApiAuthResponse, ApiFileResponse, ApiFolderResponse};

#[derive(Clone)]
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

    pub async fn refresh_token(&self, refresh_token: &String) -> Result<ApiAuthResponse, reqwest::Error> {
        self.get_client(Method::POST, "/auth/refresh")
            .json(&json!({"refreshToken": refresh_token}))
            .send().await?
            .json::<ApiAuthResponse>().await
    }

    pub async fn get_folder(&self, folder_id: &String) -> Result<ApiFolderResponse, reqwest::Error> {
        self.get_client(Method::GET, format!("/sherry/{folder_id}")).send().await?.json::<ApiFolderResponse>().await
    }

    pub async fn send_file(&self, event: &SyncEvent) -> Result<reqwest::Response, reqwest::Error> {
        let mut form = multipart::Form::new();
        if event.kind == SyncEventKind::Created || event.kind == SyncEventKind::Updated {
            form = form
                .part("file", multipart::Part::stream(Body::wrap_stream(FramedRead::new(
                    File::open(&event.local_path).await.unwrap(),
                    BytesCodec::new(),
                ))).file_name("file"));
        };

        form = form.text("sherryId", event.source_id.to_string())
            .text("eventType", event.kind.to_string().to_uppercase())
            .text("fileType", event.file_type.to_string().to_uppercase())
            .text("path", event.sync_path.to_string())
            .text("oldPath", event.old_sync_path.to_string())
            .text("size", event.size.to_string())
            .text("hash", event.update_hash.to_string());

        self.get_client(Method::POST, "/file/event").multipart(form).send().await
    }

    pub async fn check_file(&self, event: &SyncEvent) -> Result<reqwest::Response, reqwest::Error> {
        self.get_client(Method::POST, "/file/verify").json(&json!({
            "sherryId": event.source_id,
            "eventType": event.kind.to_string().to_uppercase(),
            "fileType": event.file_type.to_string().to_uppercase(),
            "path": event.sync_path.to_string(),
            "oldPath": event.old_sync_path.to_string(),
            "size": event.size,
            "hash": event.update_hash.to_string(),
        })).send().await
    }

    pub async fn get_folder_files(&self, sherry_id: &String) -> Result<Vec<ApiFileResponse>, reqwest::Error> {
        self.get_client(Method::GET, format!("/file/{sherry_id}")).send().await?.json().await
    }

    pub async fn get_file(&self, sherry_id: &String, path: &String) -> Result<reqwest::Response, reqwest::Error> {
        self.get_client(Method::GET, format!("/file/instance/{sherry_id}?path={path}")).send().await
    }

    pub fn new(base: &String, auth: &String) -> Self {
        Self {
            base: if base.is_empty() { env::var(ENV_API_URL).unwrap_or(DEFAULT_API_URL.to_string()) } else { base.clone() },
            auth: auth.clone(),
        }
    }
}
