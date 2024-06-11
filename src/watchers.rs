use std::collections::HashMap;
use std::path::{Path, PathBuf};

use futures::future;

use crate::auth::Credentials;
use crate::config::{SherryConfigJSON, SherryConfigSourceJSON, SherryConfigWatcherJSON};
use crate::event::file_event::{FileType, get_sync_path, SyncEvent, SyncEventKind};
use crate::files::{delete_file, write_file_from_stream};
use crate::hash::{FileHashJSON, recreate_hashes, update_hashes};
use crate::helpers::normalize_path;
use crate::server::api::{ApiClient, ApiFileResponse};

pub async fn fetch_watcher_files(dir: &PathBuf, config: &SherryConfigJSON, watcher: &SherryConfigWatcherJSON, source: &SherryConfigSourceJSON, user: &Credentials) -> (SherryConfigWatcherJSON, Result<(), String>) {
    log::info!("Fetching watcher files for {}, {}, {}", &watcher.local_path, &user.user_id, &source.id);

    let path = Path::new(&watcher.local_path);
    if !path.exists() {
        return (watcher.clone(), Err("Folder not exist or deleted".to_string()));
    }

    let client = ApiClient::new(&config.api_url, &user.access_token);

    let watcher_path = PathBuf::from(&watcher.local_path);

    let mut local_hashes = match recreate_hashes(dir, &watcher.hashes_id, source, &watcher_path).await {
        Ok(h) => h,
        Err(e) => return (watcher.clone(), Err(e.to_string()))
    };
    let mut remote_hashes = match client.get_folder_files(&source.id).await {
        Ok(h) => h,
        Err(e) => return (watcher.clone(), Err(e.to_string())),
    };

    let mut to_download = vec![];
    let mut to_delete = vec![];
    let mut to_upload = vec![];
    let mut to_sync: Vec<(Option<ApiFileResponse>, SyncEventKind, String)> = vec![];
    for (local_path, hash) in local_hashes.hashes.iter() {
        let local_path = PathBuf::from(&local_path);
        let sync_path = get_sync_path(&local_path, &watcher_path);
        if let Some(index) = remote_hashes.iter().position(|f| f.path == sync_path) {
            let remote = remote_hashes.swap_remove(index);
            if remote.hash == hash.hash {
                continue;
            }

            if remote.updated_at > hash.timestamp {
                to_download.push((local_path, sync_path, remote));
            } else {
                if remote.hash.is_empty() {
                    to_delete.push((local_path, sync_path, remote));
                } else {
                    to_upload.push((local_path, sync_path, hash, SyncEventKind::Update));
                }
            }
        } else {
            if hash.hash.is_empty() {
                to_sync.push((None, SyncEventKind::Delete, normalize_path(&local_path).to_str().unwrap().to_string()));
            } else {
                to_upload.push((local_path, sync_path, hash, SyncEventKind::Create));
            }
        }
    }
    for remote in remote_hashes {
        to_download.push((PathBuf::from(&remote.path), remote.path.clone(), remote.clone()))
    }

    futures::future::join_all(to_download.iter().map(|(local_path, sync_path, hash)| {
        let client = client.clone();
        async move {
            match client.get_file(&source.id, &sync_path).await {
                Ok(res) => match write_file_from_stream(&local_path, res.bytes_stream()).await {
                    Ok(_) => Some((hash.clone(), normalize_path(&local_path).to_str().unwrap().to_string())),
                    Err(_) => None
                }
                Err(_) => None
            }
        }
    })).await.iter().for_each(|to_update| {
        match to_update {
            Some((hash, path)) => to_sync.push((Some(hash.clone()), SyncEventKind::Update, path.clone())),
            None => {}
        }
    });

    futures::future::join_all(to_upload.iter().map(|(local_path, sync_path, hash, kind)| {
        let client = client.clone();
        let watcher_path = watcher_path.clone();
        async move {
            client.send_file(&SyncEvent {
                source_id: source.id.clone(),
                base: watcher_path.clone(),
                file_type: FileType::File,
                kind: kind.clone(),
                local_path: local_path.clone(),
                old_local_path: local_path.clone(),
                sync_path: sync_path.clone(),
                old_sync_path: sync_path.clone(),
                update_hash: hash.hash.clone(),
                size: local_path.metadata().unwrap().len(),
                timestamp: hash.timestamp,
            }).await.ok()
        }
    })).await;

    futures::future::join_all(to_delete.iter().map(|(local_path, sync_path, hash)| {
        async move {
            match delete_file(&local_path).await {
                Ok(_) => Some((hash.clone(), normalize_path(&local_path).to_str().unwrap().to_string())),
                Err(_) => None
            }
        }
    })).await.iter().for_each(|to_delete| {
        match to_delete {
            Some((hash, path)) => to_sync.push((Some(hash.clone()), SyncEventKind::Delete, path.clone())),
            None => {}
        }
    });

    for (remote, kind, key) in to_sync {
        match kind {
            SyncEventKind::Create | SyncEventKind::Update => {
                let remote = remote.unwrap();
                local_hashes.hashes.insert(key, FileHashJSON {
                    hash: remote.hash.clone(),
                    timestamp: remote.updated_at,
                    size: remote.size,
                });
            }
            SyncEventKind::Delete => {
                local_hashes.hashes.remove(&key);
            }
            _ => continue
        }
    }

    update_hashes(dir, &local_hashes).await.ok();

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
    dir: &PathBuf,
    config: &SherryConfigJSON,
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
                futures.push(fetch_watcher_files(dir, config, w, source, user));
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
