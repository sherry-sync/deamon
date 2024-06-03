use std::borrow::Cow;
use std::fmt::Display;

use reqwest::{Body, Method, multipart, RequestBuilder};
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::event::file_event::{SyncEvent, SyncEventKind};

struct ApiClient {
    base: String,
    auth: String,
}

impl ApiClient {
    fn build_url<T>(&self, path: T) -> String
        where
            T: Into<Cow<'static, str>> + Display,
    {
        format!("{}/{}", self.base, path)
    }
    fn get_client<T>(&self, method: Method, path: T) -> RequestBuilder
        where
            T: Into<Cow<'static, str>> + Display,
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

    pub async fn auth_user() {}

    pub fn new(base: &String, auth: &String) -> Self {
        Self {
            base: base.clone(),
            auth: auth.clone(),
        }
    }
}
