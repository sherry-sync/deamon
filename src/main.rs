mod config;

use clap::Parser;
use std::{env};
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use std::time::Duration;
use home::home_dir;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::new_debouncer;
use path_clean::PathClean;

const CONFIG_DIR: &str = ".sherry";

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_missing_value = None)]
    config: Option<String>,
}

fn main() {
    let args = Args::parse();

    let config_dir: PathBuf = match args.config {
        Some(config) => {
            let path = PathBuf::from(config);
            if path.is_absolute() {
                path
            } else {
                env::current_dir().unwrap().join(path)
            }.clean()
        }
        None => home_dir().unwrap().join(CONFIG_DIR)
    };


    println!("Using configuration from: {:?}", config_dir);

    if config::initialize_config_dir(&config_dir).is_err() {
        println!("Unable to initialize configuration, maybe access is denied");
        return;
    }

    let shared_config: Arc<Mutex<config::SherryConfigJSON>> = Arc::new(Mutex::new(config::read_config(&config_dir).unwrap()));

    let mut config_watcher = notify::recommended_watcher(
        config::get_config_watch_cb(config_dir.clone(), Arc::clone(&shared_config))
    ).unwrap();
    config_watcher.watch(config_dir.as_path(), RecursiveMode::Recursive).unwrap();



    let watchers_config = Arc::clone(&shared_config);
    let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |results| {
        if let Ok(results) = results {
            let config = watchers_config.lock().unwrap();
            println!("Config: {:?}", *config);
            for result in results {
                println!("Result: {:?}", result);
            }
        }
    }).unwrap();
    let watcher = debouncer.watcher();
    {
        let config = shared_config.lock().unwrap();
        for watcher_path in config.watchers.iter() {
            watcher.watch(Path::new(&watcher_path.local_path), RecursiveMode::Recursive).unwrap();
        }
    }

    loop {}

    // let paths = initialize();
    // println!("{:?}", paths);
    //
    // let async_block = async {
    //     let (mut tx, mut rx) = unbounded();
    //
    //     let mut watcher = recommended_watcher(move |res| {
    //         futures::executor::block_on(async {
    //             tx.send(res).await.unwrap();
    //         })
    //     })
    //         .unwrap();
    //
    //     watcher
    //         .watch(Path::new("../test-dirs/a"), RecursiveMode::Recursive)
    //         .unwrap();
    //     watcher
    //         .watch(Path::new("../test-dirs/b"), RecursiveMode::Recursive)
    //         .unwrap();
    //     watcher
    //         .watch(Path::new("../test-dirs/c"), RecursiveMode::Recursive)
    //         .unwrap();
    //
    //     while let Some(res) = rx.next().await {
    //         match res {
    //             Ok(event) => println!("changed: {:?}", event),
    //             Err(e) => println!("watch error: {:?}", e),
    //         }
    //     }
    // };
    //
    // futures::executor::block_on(async_block);
}
