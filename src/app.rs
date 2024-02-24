use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use chrono;
use log::{LevelFilter, Log};
use log4rs::append::file::FileAppender;
use log4rs::encode::pattern::PatternEncoder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebouncedEvent, Debouncer, FileIdMap, new_debouncer};
use parking_lot::Mutex;
use regex::Regex;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Instant;

use crate::{config, file_event};
use crate::config::{revalidate_sources, SherryConfigJSON, SherryConfigSourceJSON, SherryConfigUpdateEvent, SherryConfigWatcherJSON};
use crate::file_event::SyncEvent;

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

fn print_events(name: &str, events: &Vec<SyncEvent>) {
    log::info!("{name} [");
    for event in events {
        log::info!("  {:?}", event)
    }
    log::info!("]")
}

fn process_result(source_config: &SherryConfigSourceJSON, results: &Vec<DebouncedEvent>, base: &PathBuf) {
    let mut events = Vec::new();
    for result in file_event::minify_results(&results) {
        events.extend(file_event::get_sync_events(source_config, result, base));
    }

    print_events("Sync Events", &events);
}

#[derive(Debug, Clone)]
struct BasedEvent {
    events: Vec<DebouncedEvent>,
    base: PathBuf,
    key: String,
}

fn process_debounced_events(events: Vec<HashMap<String, BasedEvent>>, config: &Arc<Mutex<SherryConfigJSON>>) {
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
        if source.is_none() {
            continue;
        }

        process_result(source.unwrap(), &event.events, &event.base);
    }
}

fn spawn_debounced_sender(config: &Arc<Mutex<SherryConfigJSON>>) -> Sender<HashMap<String, BasedEvent>> {
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
        let tx = spawn_debounced_sender(&self.config);
        let rt = Handle::current();
        let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |results| {
            if let Ok(results) = results {
                let config = main_watcher_config.lock();
                let mut source_map: HashMap<String, BasedEvent> = HashMap::new();
                let mut should_revalidate = false;

                for result in results {
                    let source = get_source_by_path(&config, &result);
                    if source.is_none() {
                        continue;
                    }
                    let watcher = source.unwrap();

                    let local_path = PathBuf::from(&watcher.local_path);
                    if !local_path.exists() {
                        should_revalidate = true;
                        continue;
                    }
                    let source = config.sources.get(watcher.source.as_str());
                    if source.is_none() {
                        should_revalidate = true;
                        continue;
                    }

                    let source_results = source_map.entry(watcher.source.clone()).or_insert(BasedEvent {
                        events: Vec::new(),
                        base: local_path,
                        key: watcher.source.clone(),
                    });
                    source_results.events.push(result);
                }

                let tx = tx.clone();
                rt.spawn(async move {
                    if let Err(e) = tx.send(source_map).await {
                        println!("Error sending event result: {:?}", e);
                    }
                });

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

    fn initialize_config(config_dir: &PathBuf) -> (Arc<Mutex<SherryConfigJSON>>, Debouncer<RecommendedWatcher, FileIdMap>, Receiver<SherryConfigUpdateEvent>) {
        let shared_config: Arc<Mutex<SherryConfigJSON>> = Arc::new(Mutex::new(config::read_config(&config_dir).unwrap()));

        let (tx, rx) = channel::<SherryConfigUpdateEvent>();

        let mut config_debounce = new_debouncer(
            Duration::from_millis(200),
            None,
            config::get_config_watch_cb(config_dir.clone(), Arc::clone(&shared_config), tx),
        ).unwrap();

        config_debounce.watcher().watch(config_dir.as_path(), RecursiveMode::Recursive).unwrap();

        (shared_config, config_debounce, rx)
    }

    fn initialize_logs(config_dir: &PathBuf) {
        let log_filename = format!("{:}.log", Regex::new(r"[:.+ ]").unwrap().replace_all(chrono::Utc::now().to_rfc3339().as_str(), "-"));

        log4rs::init_config(log4rs::config::runtime::Config::builder()
            .appender(
                log4rs::config::Appender::builder().build("logfile", Box::new(
                    FileAppender::builder()
                        .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%dT%H:%M:%S)} | {({l}):5.5} | {m}{n}")))
                        .build(config_dir.join("logs").join(log_filename)).unwrap()),
                )
            )
            .build(log4rs::config::Root::builder()
                .appender("logfile")
                .build(LevelFilter::Info)).unwrap()
        ).unwrap();

        log::info!("Logs initialized");
    }

    pub fn new(config_dir: &PathBuf) -> Result<App, ()> {
        println!("Using configuration from: {:?}", config_dir);

        if config::initialize_config_dir(config_dir).is_err() {
            println!("Unable to initialize configuration, maybe access is denied");
            return Err(());
        }

        let (shared_config, config_debounce, rx) = App::initialize_config(config_dir);

        App::initialize_logs(config_dir);

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
                    break; // No need to continue, we will return here after config revalidation
                }
                watcher.watch(path, RecursiveMode::Recursive).unwrap();
            }
        }
    }
}