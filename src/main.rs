// use std::fs::OpenOptions;
// use std::io::{Read, Write};
// use std::path::{Path, PathBuf};
//
// use futures::{channel::mpsc::unbounded, SinkExt, StreamExt};
// use home::home_dir;
// use notify::{recommended_watcher, RecursiveMode, Watcher};
// use serde::{Deserialize, Serialize};
//
// fn create_app_home(path: &PathBuf) {
//     fs::create_dir(&path).unwrap();
//     let mut watchers_file = fs::File::create(path.join(CONFIG_FILE)).unwrap();
//     let config = SherryConfig {
//         version: String::from("0.0.1"),
//         directories: Vec::new(),
//     };
//     watchers_file.write_all("".as_bytes()).unwrap();
// }
//
// fn initialize() -> Vec<PathBuf> {
//     let home_path = home_dir().unwrap();
//     let app_home = home_path.join(CONFIG_DIR);
//
//     if !app_home.exists() {
//         create_app_home(&app_home);
//     }
//
//     let mut watchers_file = OpenOptions::new()
//         .read(true)
//         .open(app_home.join(CONFIG_FILE))
//         .unwrap();
//     let mut content: String = String::new();
//     watchers_file.read_to_string(&mut content).unwrap();
//
//     let mut path_arr = Vec::new();
//     content = content.replace("\r\n", "\n");
//     for line in content.split('\n') {
//         let path = PathBuf::from(line);
//         if path.exists() {
//             path_arr.push(path);
//         }
//     }
//     path_arr
// }

mod config;

use clap::Parser;
use std::{env};
use std::path::{PathBuf};
use home::home_dir;
use notify::{RecursiveMode, Watcher};
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

    let mut config_watcher = notify::recommended_watcher(config::get_config_watch_cb(config_dir.clone())).unwrap();
    config_watcher.watch(config_dir.as_path(), RecursiveMode::Recursive).unwrap();

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
