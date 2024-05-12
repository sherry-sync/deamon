use reqwest::{Body, multipart};
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};
use crate::config::SherryConfigJSON;
use crate::event::file_event::{SyncEvent, SyncEventKind};

pub async fn send_file(config: SherryConfigJSON, auth: String, event: &SyncEvent) -> anyhow::Result<String> {
    let mut form = multipart::Form::new()
        .text("eventType", event.kind.to_string())
        .text("fileName", event.sync_path.to_string());
    if event.kind == SyncEventKind::Create || event.kind == SyncEventKind::Update {
        form = form
            .text("fileHash", event.update_hash.to_string())
            .part("file", multipart::Part::stream(Body::wrap_stream(FramedRead::new(
                File::open(&event.local_path).await?,
                BytesCodec::new(),
            ))));
    }

    Ok(reqwest::Client::new()
        .post(format!("{}/events", config.api_url))
        .header("Authorization", format!("Bearer {auth}"))
        .multipart(form).send().await?.text().await?)
}
