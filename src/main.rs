mod config;

use clap::Parser;
use std::{env};
use parking_lot::{Mutex};
use std::sync::{Arc};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use home::home_dir;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{Debouncer, FileIdMap, new_debouncer};
use path_clean::PathClean;
use crate::config::SherryConfigUpdateEvent;

const CONFIG_DIR: &str = ".sherry";

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_missing_value = None)]
    config: Option<String>,
}

fn resolve_config_dir(config: Option<String>) -> PathBuf {
    match config {
        Some(config) => {
            let path = PathBuf::from(config);
            if path.is_absolute() {
                path
            } else {
                env::current_dir().unwrap().join(path)
            }.clean()
        }
        None => home_dir().unwrap().join(CONFIG_DIR)
    }
}

fn initialize_config_watcher(config_dir: &PathBuf) -> Option<(Arc<Mutex<config::SherryConfigJSON>>, Debouncer<RecommendedWatcher, FileIdMap>, Receiver<SherryConfigUpdateEvent>)> {
    println!("Using configuration from: {:?}", config_dir);

    if config::initialize_config_dir(config_dir).is_err() {
        println!("Unable to initialize configuration, maybe access is denied");
        return None;
    }

    let shared_config: Arc<Mutex<config::SherryConfigJSON>> = Arc::new(Mutex::new(config::read_config(&config_dir).unwrap()));

    let (tx, rx) = channel::<SherryConfigUpdateEvent>();

    let mut config_debounce = new_debouncer(
        Duration::from_millis(200),
        None,
        config::get_config_watch_cb(config_dir.clone(), Arc::clone(&shared_config), tx)
    ).unwrap();

    config_debounce.watcher().watch(config_dir.as_path(), RecursiveMode::Recursive).unwrap();

    return Some((shared_config, config_debounce, rx));
}

fn initialize_watchers(shared_config: Arc<Mutex<config::SherryConfigJSON>>) -> Debouncer<RecommendedWatcher, FileIdMap> {
    let main_watcher_config = Arc::clone(&shared_config);
    let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |results| {
        if let Ok(results) = results {
            let config = main_watcher_config.lock();
            println!("Config: {:?}", *config);
            for result in results {
                println!("Result: {:?}", result);
            }
        }
    }).unwrap();

    let watcher = debouncer.watcher();
    {
        let config = shared_config.lock();
        for watcher_path in config.watchers.iter() {
            watcher.watch(Path::new(&watcher_path.local_path), RecursiveMode::Recursive).unwrap();
        }
    }

    return debouncer;
}

fn listen_config_update(watcher: &mut RecommendedWatcher, rx: Receiver<SherryConfigUpdateEvent>) {
    for update in rx {
        // TODO: Check the diff correctly

        for watcher_path in update.old.watchers.iter() {
            watcher.unwatch(Path::new(&watcher_path.local_path)).unwrap();
        }
        for watcher_path in update.new.watchers.iter() {
            watcher.watch(Path::new(&watcher_path.local_path), RecursiveMode::Recursive).unwrap();
        }
    }
}

fn main() {
    let args = Args::parse();

    let config_dir = resolve_config_dir(args.config);

    let config_args = initialize_config_watcher(&config_dir);
    if config_args.is_none() { return; }
    let (shared_config, _config_debounce, rx) = config_args.unwrap();

    listen_config_update(initialize_watchers(shared_config).watcher(), rx);
}
