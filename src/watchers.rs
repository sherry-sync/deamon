use std::collections::HashMap;
use std::path::Path;

use futures::future;
use tokio::fs;

use crate::auth::Credentials;
use crate::config::{SherryConfigSourceJSON, SherryConfigWatcherJSON};
use crate::helpers::str_err_prefix;

pub async fn fetch_watcher_files(watcher: &SherryConfigWatcherJSON, user: &Credentials, source: &SherryConfigSourceJSON) -> (SherryConfigWatcherJSON, Result<(), String>) {
    log::info!("Fetching watcher files for {}, {}, {}", &watcher.local_path, &user.user_id, &source.id);

    let path = Path::new(&watcher.local_path);
    if !path.exists() {
        match fs::create_dir_all(&path).await.map_err(str_err_prefix("Can't create watcher directory")) {
            Ok(_) => (),
            Err(e) => return (watcher.clone(), Err(e))
        }
    }

    (
        SherryConfigWatcherJSON {
            complete: true,
            ..watcher.clone()
        },
        Ok(())
    )
}

pub struct ActualizedWatcherMeta {
    pub invalid_watchers: Vec<SherryConfigWatcherJSON>,
    pub valid_watchers: Vec<SherryConfigWatcherJSON>,
}

pub async fn actualize_watchers(
    users: &HashMap<String, Credentials>,
    sources: &HashMap<String, SherryConfigSourceJSON>,
    watchers: &Vec<SherryConfigWatcherJSON>,
) -> ActualizedWatcherMeta {
    let mut invalid_watchers = vec![];
    let mut valid_watchers = vec![];

    let mut futures = vec![];
    for w in watchers {
        if let Some(user) = users.get(&w.user_id) {
            if let Some(source) = sources.get(&w.source) {
                futures.push(fetch_watcher_files(w, user, source));
            } else {
                invalid_watchers.push(w.clone());
            }
        } else {
            invalid_watchers.push(w.clone());
        }
    }

    future::join_all(futures).await.iter().for_each(|(w, res)| {
        match res {
            Ok(_) => valid_watchers.push(w.clone()),
            Err(e) => {
                log::error!("Failed to actualize watcher {}: {e}", w.source);
                invalid_watchers.push(w.clone());
            }
        }
    });

    ActualizedWatcherMeta { invalid_watchers, valid_watchers }
}
