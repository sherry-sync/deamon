use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_full::DebouncedEvent;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Instant;

use crate::config::{AccessRights, SherryConfigJSON, SherryConfigSourceJSON};
use crate::events::file_event::{filter_events, get_sync_events, minify_results, optimize_events, print_events};

pub fn process_result(source_config: &SherryConfigSourceJSON, results: &Vec<BasedDebounceEvent>) {
    if source_config.access == AccessRights::Read {
        return;
    }

    let mut events = Vec::new();
    for result in minify_results(&results) {
        events.extend(get_sync_events(source_config, result));
    }

    print_events("Sync Events", &events);
    let optimized_events = optimize_events(&events);
    print_events("Optimized Events", &optimized_events);
    let filtered_events = filter_events(&source_config, &optimized_events);
    print_events("Filtered Events", &filtered_events);
}

fn create_debounce(rt: &tokio::runtime::Handle, config: &Arc<Mutex<SherryConfigJSON>>, source_id: &String, is_running: &Arc<Mutex<bool>>) -> Sender<BasedDebounceEvent> {
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

        let config = config.lock();
        let source = config.sources.get(&source_id);
        if let Some(source) = source {
            process_result(source, &buffer);
        }
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
    config: Arc<Mutex<SherryConfigJSON>>,
    source_id: String,
    tx: Option<Sender<BasedDebounceEvent>>,
    rt: tokio::runtime::Handle,
}

impl EventProcessingDebounce {
    pub fn new(rt: &tokio::runtime::Handle, config: &Arc<Mutex<SherryConfigJSON>>, source_id: &String) -> EventProcessingDebounce {
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
