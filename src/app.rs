use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, Watcher};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use parking_lot::Mutex;

use crate::config::{get_source_by_path, SherryConfig};
use crate::events::event_processing::{BasedDebounceEvent, EventProcessingDebounce};
use crate::logs::initialize_logs;

pub struct App {
    config: Arc<Mutex<SherryConfig>>,
}

impl App {
    pub fn new(config_dir: &PathBuf) -> Result<App, ()> {
        println!("Using configuration from: {:?}", config_dir);
        println!("Using recommended watcher: {:?}", RecommendedWatcher::kind());

        let config = SherryConfig::new(config_dir);

        if config.is_err() {
            println!("Unable to initialize configuration, maybe access is denied");
            return Err(());
        }

        let config = Arc::new(Mutex::new(config.unwrap()));

        initialize_logs(config_dir);

        Ok(App { config })
    }

    pub async fn listen(&mut self) {
        let main_watcher_config = Arc::clone(&self.config);
        let mut event_processing_debounce_map = HashMap::new();
        let rt = tokio::runtime::Handle::current();
        let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |results: DebounceEventResult| {
            if let Ok(results) = results {
                let config = main_watcher_config.lock().get();
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

                    let local_path = PathBuf::from(&watcher.local_path);
                    if !local_path.exists() {
                        should_revalidate = true;
                        continue;
                    }
                    let source_id = watcher.source_id.clone();
                    let source = config.sources.get(source_id.as_str());
                    if source.is_none() {
                        should_revalidate = true;
                        continue;
                    }

                    let debounce = event_processing_debounce_map
                        .entry(source_id.clone())
                        .or_insert(EventProcessingDebounce::new(&rt, &main_watcher_config, &source_id));
                    debounce.send(BasedDebounceEvent {
                        event: result,
                        base: local_path,
                        hash_id: watcher.hashes_id.clone(),
                    })
                }

                for source in event_processing_debounce_map.keys().cloned().collect::<Vec<String>>() {
                    if !{ event_processing_debounce_map.get(&source).unwrap().is_running() } {
                        event_processing_debounce_map.remove(&source);
                    }
                }

                if should_revalidate {
                    main_watcher_config.lock().revalidate();
                }
            }
        }).unwrap();

        {
            let watcher = debouncer.watcher();
            self.config.lock().apply_update(watcher)
        }

        let watcher = debouncer.watcher();

        let (dir, receiver) = {
            let config = self.config.lock();
            (config.get_path(), config.get_receiver())
        };

        SherryConfig::listen(&dir, &receiver.lock(), watcher);
    }
}