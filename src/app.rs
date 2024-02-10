use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebouncedEvent, Debouncer, FileIdMap, new_debouncer};
use parking_lot::Mutex;

use crate::{config, file_event};
use crate::config::{revalidate_sources, SherryConfigJSON, SherryConfigSourceJSON, SherryConfigUpdateEvent, SherryConfigWatcherJSON};

fn get_source_by_path<'a>(config: &'a SherryConfigJSON, result: &DebouncedEvent) -> Option<&'a SherryConfigWatcherJSON> {
    let result_path = result.paths.first();
    if result_path.is_none() {
        return None;
    }
    let result_path = result_path.unwrap();

    config.watchers.iter().find_map(|w| {
        if result_path.starts_with(&w.local_path) {
            return Some(w);
        }
        return None;
    })
}

fn process_result(source_config: &SherryConfigSourceJSON, results: &Vec<DebouncedEvent>, base: &PathBuf) {
    if !base.exists() {}

    let mut sync_events = Vec::new();

    for result in file_event::minify_results(&results) {
        sync_events.extend(file_event::get_sync_events(source_config, result, base));
    }
    println!("Sync Events:\n{:?}", sync_events);
}

pub struct App {
    config_path: PathBuf,
    config: Arc<Mutex<SherryConfigJSON>>,
    config_update_rx: Receiver<SherryConfigUpdateEvent>,

    #[allow(dead_code)] // This values should be saved to avoid it's destroying
    config_debounce: Debouncer<RecommendedWatcher, FileIdMap>,
}

impl App {
    fn initialize_watchers(&mut self) -> Debouncer<RecommendedWatcher, FileIdMap> {
        let main_watcher_config = Arc::clone(&self.config);
        let config_path = self.config_path.clone();
        let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |results| {
            if let Ok(results) = results {
                let config = main_watcher_config.lock();
                let mut source_map: HashMap<String, Vec<DebouncedEvent>> = HashMap::new();
                let mut base_map: HashMap<String, PathBuf> = HashMap::new();
                let mut should_revalidate = false;

                for result in results {
                    let source = get_source_by_path(&config, &result);
                    if source.is_none() {
                        continue;
                    }
                    let source = source.unwrap();

                    let local_path = PathBuf::from(&source.local_path);
                    if !local_path.exists() {
                        should_revalidate = true;
                        continue;
                    }

                    let source_results = source_map.entry(source.source.clone()).or_insert(Vec::new());
                    source_results.push(result);
                    base_map.insert(source.source.clone(), local_path);
                }

                for (key, value) in &source_map {
                    let source = config.sources.get(key);
                    if source.is_none() {
                        continue;
                    }

                    let base = base_map.get(key).unwrap();
                    if base.exists() {
                        process_result(source.unwrap(), value, base);
                    }
                }

                if should_revalidate {
                    revalidate_sources(&config_path, &config);
                }
            }
        }).unwrap();

        let watcher = debouncer.watcher();
        {
            let config = self.config.lock();
            for watcher_path in config.watchers.iter() {
                let path = Path::new(&watcher_path.local_path);
                if !path.exists() {
                    revalidate_sources(&self.config_path, &config);
                    break; // Config watcher will reset watchers, so don't need to set them here
                }
                watcher.watch(path, RecursiveMode::Recursive).unwrap();
            }
        }

        debouncer
    }

    pub fn new(config_dir: &PathBuf) -> Result<App, ()> {
        println!("Using configuration from: {:?}", config_dir);

        if config::initialize_config_dir(config_dir).is_err() {
            println!("Unable to initialize configuration, maybe access is denied");
            return Err(());
        }

        let shared_config: Arc<Mutex<SherryConfigJSON>> = Arc::new(Mutex::new(config::read_config(&config_dir).unwrap()));

        let (tx, rx) = channel::<SherryConfigUpdateEvent>();

        let mut config_debounce = new_debouncer(
            Duration::from_millis(200),
            None,
            config::get_config_watch_cb(config_dir.clone(), Arc::clone(&shared_config), tx),
        ).unwrap();

        config_debounce.watcher().watch(config_dir.as_path(), RecursiveMode::Recursive).unwrap();

        Ok(App {
            config_path: config_dir.clone(),
            config: shared_config,
            config_update_rx: rx,

            config_debounce,
        })
    }

    pub fn listen(&mut self) {
        let mut debounce = self.initialize_watchers();
        let watcher = debounce.watcher();

        for update in self.config_update_rx.iter() {
            for watcher_path in update.old.watchers.iter() {
                watcher.unwatch(Path::new(&watcher_path.local_path)).unwrap();
            }
            for watcher_path in update.new.watchers.iter() {
                let path = Path::new(&watcher_path.local_path);
                if !path.exists() {
                    revalidate_sources(&self.config_path, &update.new);
                    break // No need to continue, we will return here after config revalidation
                }
                watcher.watch(path, RecursiveMode::Recursive).unwrap();
            }
        }
    }
}