use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_full::DebouncedEvent;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Instant;

use crate::config::{AccessRights, SherryConfigWatcherJSON};
use crate::event::file_event::{complete_events, filter_events, get_sync_events, minify_results, optimize_events, log_events, SyncEvent, SyncEventKind};
use crate::hash::{get_hashes, update_hashes};

pub async fn process_result(app: crate::app::App, source_id: &String, results: &Vec<BasedDebounceEvent>) {
    let dir = app.config.lock().await.get_path();
    let config = app.config.lock().await.get_main().await;

    let source = config.sources.get(source_id);
    if source.is_none() {
        return;
    }
    let source = source.unwrap();
    let watchers: HashMap<String, &SherryConfigWatcherJSON> = config.watchers
        .iter()
        .filter_map(|e| if e.source.eq(source_id) { Some((e.local_path.clone(), e)) } else { None })
        .collect();

    if source.access == AccessRights::Read {
        return;
    }
    
    let events = &minify_results(&results)
        .iter()
        .flat_map(|r| get_sync_events(&source, r))
        .collect::<Vec<SyncEvent>>();

    log_events("Received", &events);

    let events = complete_events(&filter_events(&source, &optimize_events(&events))).await;

    log_events("Optimized", &events);

    let mut hashes_map = HashMap::new();
    let mut updated_hashes = HashMap::new();
    for e in events {
        let watcher = match watchers.get(&e.base.to_str().unwrap().to_string()) {
            Some(watcher) => watcher,
            None => continue,
        };

        let hashes_id = watcher.hashes_id.clone();
        let base = e.base.clone();

        let hashes = match hashes_map.get(&base) {
            Some(v) => v,
            None => {
                let h = get_hashes(&dir, &source, &base, &hashes_id).await.unwrap();
                hashes_map.insert(base.clone(), h);
                hashes_map.get(&base).unwrap()
            }
        };

        let mut to_update = updated_hashes.entry(base.clone()).or_insert(hashes.clone());
        match e.kind {
            SyncEventKind::Delete => {
                to_update.hashes.remove(&e.local_path.to_str().unwrap().to_string());
            }
            SyncEventKind::Rename => {
                to_update.hashes.remove(&e.old_local_path.to_str().unwrap().to_string());
                to_update.hashes.insert(e.local_path.to_str().unwrap().to_string(), e.update_hash);
            }
            _ => {
                to_update.hashes.insert(e.local_path.to_str().unwrap().to_string(), e.update_hash);
            }
        }

        // Validate by API

        // Send update
    }
    for (k, v) in updated_hashes {
        if *hashes_map.get(&k).unwrap() != v {
            update_hashes(&dir, &v).await.unwrap();
        }
    }
}

fn create_debounce(rt: &tokio::runtime::Handle, app: crate::app::App, source_id: &String, is_running: &Arc<Mutex<bool>>) -> Sender<BasedDebounceEvent> {
    let source_id = source_id.clone();
    let is_running = Arc::clone(is_running);

    let (tx, mut rx) = mpsc::channel::<BasedDebounceEvent>(100);
    rt.spawn(async move {
        { *is_running.lock().await = true; }

        let timeout = Duration::from_secs(1);
        let mut buffer = Vec::new();
        let mut last_event_time = Instant::now();

        loop {
            let mut is_conn_closed = false;
            while let Some(event) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await.ok() {
                match event {
                    Some(event) => {
                        last_event_time = Instant::now();
                        buffer.push(event);
                    }
                    None => {
                        is_conn_closed = true;
                        break;
                    }
                }
            }
            if is_conn_closed || last_event_time.elapsed() >= timeout { break; }
        }

        { *is_running.lock().await = false; }

        process_result(app, &source_id, &buffer).await;
    });

    tx
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasedDebounceEvent {
    pub event: DebouncedEvent,
    pub base: PathBuf,
}

pub struct EventProcessingDebounce {
    _is_running: Arc<Mutex<bool>>,
    app: crate::app::App,
    source_id: String,
    tx: Option<Sender<BasedDebounceEvent>>,
    rt: tokio::runtime::Handle,
}

impl EventProcessingDebounce {
    pub fn new(rt: &tokio::runtime::Handle, app: &crate::app::App, source_id: &String) -> EventProcessingDebounce {
        EventProcessingDebounce {
            _is_running: Arc::new(Mutex::new(false)),
            app: app.clone(),
            source_id: source_id.clone(),
            tx: None,
            rt: rt.clone(),
        }
    }

    pub async fn send(&mut self, event: BasedDebounceEvent) {
        if !self.is_running().await {
            self.tx = Some(create_debounce(&self.rt, self.app.clone(), &self.source_id, &self._is_running));
        }
        let tx = self.tx.clone().unwrap();
        if let Err(e) = tx.send(event).await {
            println!("Error sending event result: {:?}", e);
        }
    }

    pub async fn is_running(&self) -> bool {
        self._is_running.lock().await.clone()
    }
}
