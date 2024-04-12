use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_full::DebouncedEvent;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Instant;

use crate::config::{AccessRights, SherryConfig, SherryConfigWatcherJSON};
use crate::events::file_event::{complete_events, filter_events, get_sync_events, minify_results, optimize_events, SyncEvent};
use crate::hash::get_hashes;

pub async fn process_result(config: Arc<Mutex<SherryConfig>>, source_id: &String, results: &Vec<BasedDebounceEvent>) {
    let dir = config.lock().get_path();
    let config = config.lock().get_main();

    let source = config.sources.get(source_id);
    if source.is_none() {
        return;
    }
    let source = source.unwrap();
    let watchers: HashMap<String, &SherryConfigWatcherJSON> = config.watchers
        .iter()
        .filter_map(|e| if e.source_id.eq(source_id) { Some((e.local_path.clone(), e)) } else { None })
        .collect();

    if source.access == AccessRights::Read {
        return;
    }

    let events =
        complete_events(
            &filter_events(&source,
                           &optimize_events(
                               &minify_results(&results)
                                   .iter()
                                   .flat_map(|r| get_sync_events(&source, r))
                                   .collect::<Vec<SyncEvent>>()
                           ),
            )
        );

    let mut hashes_map = HashMap::new();
    for e in events {
        let watcher = match watchers.get(&e.base.to_str().unwrap().to_string()) {
            Some(watcher) => watcher,
            None => continue,
        };

        let hashes_id = watcher.hashes_id.clone();
        let base = e.base.clone();

        let hashes = hashes_map
            .entry(base.clone())
            .or_insert_with(|| get_hashes(&dir, &source, &base, &hashes_id).unwrap());

        match hashes.hashes.get(&e.local_path.to_str().unwrap().to_string()) {
            None => {}
            Some(_) => {
                
            }
        }
    }
}

fn create_debounce(rt: &tokio::runtime::Handle, config: &Arc<Mutex<SherryConfig>>, source_id: &String, is_running: &Arc<Mutex<bool>>) -> Sender<BasedDebounceEvent> {
    let config = Arc::clone(config);
    let source_id = source_id.clone();
    let is_running = Arc::clone(is_running);

    let (tx, mut rx) = mpsc::channel::<BasedDebounceEvent>(100);
    { *is_running.lock() = true; }

    rt.spawn(async move {
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
        { *is_running.lock() = false; }

        process_result(config, &source_id, &buffer).await;
    });

    tx
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasedDebounceEvent {
    pub event: DebouncedEvent,
    pub base: PathBuf,
    pub hash_id: String,
}

pub struct EventProcessingDebounce {
    _is_running: Arc<Mutex<bool>>,
    config: Arc<Mutex<SherryConfig>>,
    source_id: String,
    tx: Option<Sender<BasedDebounceEvent>>,
    rt: tokio::runtime::Handle,
}

impl EventProcessingDebounce {
    pub fn new(rt: &tokio::runtime::Handle, config: &Arc<Mutex<SherryConfig>>, source_id: &String) -> EventProcessingDebounce {
        EventProcessingDebounce {
            _is_running: Arc::new(Mutex::new(false)),
            config: Arc::clone(config),
            source_id: source_id.clone(),
            tx: None,
            rt: rt.clone(),
        }
    }

    pub fn send(&mut self, event: BasedDebounceEvent) {
        if !self.is_running() {
            self.tx = Some(create_debounce(&self.rt, &self.config, &self.source_id, &self._is_running));
        }
        let tx = self.tx.clone().unwrap();
        let rt = self.rt.clone();
        rt.spawn(async move {
            if let Err(e) = tx.send(event).await {
                println!("Error sending event result: {:?}", e);
            }
        });
    }

    pub fn is_running(&self) -> bool {
        self._is_running.lock().clone()
    }
}
