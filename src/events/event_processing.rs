use std::collections::HashMap;
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

#[derive(Debug, Clone)]
pub struct BasedEvent {
    pub events: Vec<DebouncedEvent>,
    pub base: PathBuf,
    pub key: String,
}

pub fn process_result(source_config: &SherryConfigSourceJSON, results: &Vec<DebouncedEvent>, base: &PathBuf) {
    if source_config.access == AccessRights::Read {
        return;
    }

    let mut events = Vec::new();
    for result in minify_results(&results) {
        events.extend(get_sync_events(source_config, result, base));
    }

    print_events("Sync Events", &events);
    let optimized_events = optimize_events(&events);
    print_events("Optimized Events", &optimized_events);
    let filtered_events = filter_events(&source_config, &optimized_events);
    print_events("Filtered Events", &filtered_events);
}

pub fn process_debounced_events(events: Vec<HashMap<String, BasedEvent>>, config: &Arc<Mutex<SherryConfigJSON>>) {
    let mut result_map: HashMap<String, BasedEvent> = HashMap::new();
    for map in events {
        for (k, v) in map {
            let source = result_map.entry(k).or_insert(BasedEvent {
                events: Vec::new(),
                base: v.base,
                key: v.key,
            });
            source.events.extend(v.events);
        }
    }


    let config = Arc::clone(config);
    let config = config.lock();

    for (_, event) in result_map {
        let source = config.sources.get(&event.key);
        if source.is_none() || event.events.is_empty() {
            continue;
        }

        process_result(source.unwrap(), &event.events, &event.base);
    }
}

pub fn spawn_debounced_sender(config: &Arc<Mutex<SherryConfigJSON>>) -> Sender<HashMap<String, BasedEvent>> {
    let config = Arc::clone(config);
    let (tx, mut rx) = mpsc::channel::<HashMap<String, BasedEvent>>(100);

    tokio::spawn(async move {
        let timeout = Duration::from_secs(1);
        let mut buffer: Vec<HashMap<String, BasedEvent>> = Vec::new();
        let mut last_event_time = Instant::now();

        loop {
            if last_event_time.elapsed() >= timeout {
                if !buffer.is_empty() {
                    process_debounced_events(buffer, &config);
                    buffer = Vec::new();
                }
                last_event_time = Instant::now();
            }
            while let Some(event) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await.ok() {
                last_event_time = Instant::now();
                buffer.push(event.unwrap());
            }
        }
    });

    tx
}