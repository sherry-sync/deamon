use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, Watcher};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use tokio::sync::Mutex;
use rust_socketio::asynchronous::Client;
use serde_json::json;

use crate::config::{SherryConfig, SherryConfigJSON, SherryConfigWatcherJSON};
use crate::event::event_processing::{BasedDebounceEvent, EventProcessingDebounce};
use crate::logs::initialize_logs;
use crate::server::socket::initialize_socket;

fn get_source_by_path<'a>(config: &'a SherryConfigJSON, path: &PathBuf) -> Option<&'a SherryConfigWatcherJSON> {
    config.watchers.iter().find_map(|w| {
        if path.starts_with(&w.local_path) {
            return Some(w);
        }
        return None;
    })
}

#[derive(Clone)]
pub struct App {
    pub config: Arc<Mutex<SherryConfig>>,
    pub socket: Arc<Mutex<Client>>,
}

impl App {
    pub async fn new(config_dir: &PathBuf) -> Result<App, ()> {
        initialize_logs(config_dir);

        log::info!("Using configuration from: {:?}", config_dir);
        log::info!("Using recommended watcher: {:?}", RecommendedWatcher::kind());

        let config = match SherryConfig::new(config_dir).await {
            Err(_) => {
                println!("Unable to initialize configuration, maybe access is denied");
                return Err(());
            }
            Ok(v) => v
        };

        let socket = initialize_socket(
            &config.get_main().await.socket_url,
            &config.get_auth().await.records.iter().map(|(_, v)| v.access_token.clone()).collect()
        ).await;

        socket.emit("connect", json!({})).await.expect("Socket connection failed");
        log::info!("Connected to socket");

        let config = Arc::new(Mutex::new(config));
        let socket = Arc::new(Mutex::new(socket));

        Ok(App { config, socket })
    }

    pub async fn listen(&mut self) {
        let main_watcher_config = Arc::clone(&self.config);
        let mut event_processing_debounce_map = HashMap::new();
        let app = self.clone();
        let rt = tokio::runtime::Handle::current();
        let debouncer = new_debouncer(Duration::from_millis(200), None, move |results: DebounceEventResult| {
            rt.block_on(async {
                if let Ok(results) = results {
                    
                    let config = main_watcher_config.lock().await.get_main().await;
                    log::info!("Processing events: {:?}", results);
                    let mut should_revalidate = false;

                    
                    for result in results {
                        let source_path = result.paths.first();
                        if source_path.is_none() {
                            continue;
                        }

                        let source = get_source_by_path(&config, &source_path.unwrap());
                        if source.is_none() {
                            continue;
                        }
                        let watcher = source.unwrap();
                        if !watcher.complete {
                            continue;
                        }

                        let local_path = PathBuf::from(&watcher.local_path);
                        if !local_path.exists() {
                            should_revalidate = true;
                            continue;
                        }
                        let source_id = watcher.source.clone();
                        let source = config.sources.get(source_id.as_str());
                        if source.is_none() {
                            should_revalidate = true;
                            continue;
                        }

                        let debounce = event_processing_debounce_map
                            .entry(source_id.clone())
                            .or_insert(EventProcessingDebounce::new(&rt, &app, &source_id));
                        debounce.send(BasedDebounceEvent {
                            event: result,
                            base: local_path,
                        }).await;
                    }

                    for source in event_processing_debounce_map.keys().cloned().collect::<Vec<String>>() {
                        if !{ event_processing_debounce_map.get(&source).unwrap().is_running().await } {
                            event_processing_debounce_map.remove(&source);
                        }
                    }

                    if should_revalidate {
                        main_watcher_config.lock().await.revalidate().await;
                    }
                }
            });
        }).unwrap();
        SherryConfig::listen(&self.config, &self.socket, &Arc::new(Mutex::new(debouncer))).await;
    }
}
