use fmt::Display;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;

use glob::Pattern;
use notify::event::{DataChange, ModifyKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::config::{SherryConfigSourceJSON, SherryConfigWatcherJSON};
use crate::event::event_processing::BasedDebounceEvent;
use crate::hash::{get_file_hash, get_hashes};
use crate::helpers::{get_now_as_millis, normalize_path, PATH_SEP};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SyncEventKind {
    Created,
    Updated,
    Moved,
    Deleted,
}

impl Display for SyncEventKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, SerdeDiff, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FileType {
    Dir,
    File,
}

impl Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct SyncEvent {
    pub source_id: String,
    pub base: PathBuf,
    pub file_type: FileType,
    pub kind: SyncEventKind,
    pub local_path: PathBuf,
    pub old_local_path: PathBuf,
    pub sync_path: String,
    pub old_sync_path: String,
    pub update_hash: String,
    pub size: u64,
    pub timestamp: i128,
}

pub fn log_events(name: &str, events: &Vec<SyncEvent>) {
    log::info!("{name} [");
    for event in events {
        log::info!("  {:?}", event)
    }
    log::info!("]")
}


fn result_cmp(a: &BasedDebounceEvent, b: &BasedDebounceEvent) -> Ordering {
    a.event.time.cmp(&b.event.time)
}

pub fn minify_results(results: &Vec<BasedDebounceEvent>) -> Vec<BasedDebounceEvent> {
    let mut results = results.clone();
    results.sort_by(result_cmp);

    let mut new_results = Vec::new();
    let mut remove_results = HashMap::new();
    for (i, result) in results.iter().enumerate() {
        match result.event.kind {
            EventKind::Modify(modify_kind) => {
                match modify_kind {
                    ModifyKind::Name(mode) => {
                        match mode {
                            RenameMode::From => {
                                let to = results.get(i + 1);
                                if to.is_some() {
                                    let to = &to.unwrap().event;
                                    let from = &result.event;
                                    new_results.push(BasedDebounceEvent {
                                        event: DebouncedEvent {
                                            event: notify::Event {
                                                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                                                paths: vec![from.paths.first().unwrap().clone(), to.paths.first().unwrap().clone()],
                                                attrs: to.attrs.clone(),
                                            },
                                            time: to.time,
                                        },
                                        base: result.base.clone(),
                                    })
                                }
                            }
                            RenameMode::Both => {
                                new_results.push(result.clone())
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        new_results.push(result.clone())
                    }
                }
                // If Modify event go after Remove, the remove is lie, so we skipping it
                remove_results.remove(result.event.paths.last().unwrap());
            }
            EventKind::Remove(_) => {
                remove_results.insert(result.event.paths.first().unwrap(), result.clone());
            }
            EventKind::Create(_) => {
                if remove_results.get(result.event.paths.first().unwrap()).is_none() {
                    new_results.push(result.clone())
                } else {
                    let result_event = &result.event;
                    new_results.push(BasedDebounceEvent {
                        event: DebouncedEvent {
                            event: notify::Event {
                                kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                                paths: result_event.paths.clone(),
                                attrs: result_event.attrs.clone(),
                            },
                            time: result_event.time.clone(),
                        },
                        base: result.base.clone(),
                    })
                }
            }
            _ => {}
        }
    }

    new_results.extend(remove_results.values().cloned());
    new_results.sort_by(result_cmp);
    new_results
}

pub fn get_sync_path(path: &PathBuf, base: &PathBuf) -> String {
    normalize_path(&PathBuf::from(path
        .strip_prefix(base).unwrap()
        .iter().collect::<Vec<&OsStr>>()
        .join(OsStr::new(PATH_SEP))
    )).to_str().unwrap().to_string()
}

fn get_dir_file_events(config: &SherryConfigSourceJSON, path: &PathBuf, base: &PathBuf, kind: &SyncEventKind) -> Vec<SyncEvent> {
    let mut events = Vec::new();
    let path = normalize_path(path);
    if path.is_file() {
        let sync_path = get_sync_path(&path, base);
        events.push(SyncEvent {
            source_id: config.id.clone(),
            base: base.clone(),
            file_type: FileType::File,
            kind: kind.clone(),
            local_path: path.clone(),
            old_local_path: path.clone(),
            old_sync_path: sync_path.clone(),
            sync_path,
            update_hash: "".to_string(),
            size: 0,
            timestamp: get_now_as_millis(),
        });
    } else if path.is_dir() {
        match path.read_dir() {
            Ok(dir) => {
                for entry in dir {
                    match entry {
                        Ok(entry) => {
                            events.extend(get_dir_file_events(config, &entry.path(), base, &kind));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    events
}

pub async fn get_sync_events(config: &SherryConfigSourceJSON, result: &BasedDebounceEvent, dir: &PathBuf, watcher: &SherryConfigWatcherJSON) -> Vec<SyncEvent> {
    // Modify(Any) - file update
    // Modify(Name(Both)) file/dir rename
    // Create(Any) - file/dir created
    // Remove(Any) - file(dir) removed

    let base = &result.base;
    let result = &result.event;

    let mut events = Vec::new();

    let local_path = normalize_path(&result.paths.last().unwrap().to_path_buf());
    let old_local_path = normalize_path(&result.paths.first().unwrap().to_path_buf());
    if local_path.is_symlink() {
        return events;
    }

    let sync_path = get_sync_path(&local_path, base);
    let old_sync_path = get_sync_path(&old_local_path, base);

    if !local_path.exists() {
        let hashes = get_hashes(dir, config, base, &watcher.hashes_id).await.unwrap();
        let parent_path = Regex::new(r"/+$").unwrap().replace_all(local_path.to_str().unwrap(), PATH_SEP).to_string();
        hashes.hashes.iter().for_each(|(local_path, _)| {
            if local_path.starts_with(&parent_path) {
                let local_path = PathBuf::from(local_path);
                let sync_path = get_sync_path(&local_path, base);
                events.push(SyncEvent {
                    source_id: config.id.clone(),
                    base: base.clone(),
                    file_type: FileType::File,
                    kind: SyncEventKind::Deleted,
                    local_path: local_path.clone(),
                    old_local_path: local_path.clone(),
                    sync_path: sync_path.clone(),
                    old_sync_path: sync_path.clone(),
                    update_hash: "".to_string(),
                    size: 0,
                    timestamp: get_now_as_millis(),
                })
            }
        })
    }

    if local_path.is_dir() {
        match result.kind {
            EventKind::Modify(kind) => {
                if kind == ModifyKind::Name(RenameMode::Both) {
                    events.push(SyncEvent {
                        source_id: config.id.clone(),
                        base: base.clone(),
                        file_type: FileType::Dir,
                        kind: SyncEventKind::Moved,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        old_local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: get_now_as_millis(),
                    });
                }
            }
            EventKind::Create(_) => {
                events.extend(get_dir_file_events(config, &local_path, base, &SyncEventKind::Created));
            }
            EventKind::Remove(_) => {
                events.push(SyncEvent {
                    source_id: config.id.clone(),
                    base: base.clone(),
                    file_type: FileType::Dir,
                    kind: SyncEventKind::Deleted,
                    update_hash: "".to_string(),
                    size: 0,
                    local_path,
                    old_local_path,
                    sync_path,
                    old_sync_path,
                    timestamp: get_now_as_millis(),
                });
            }
            _ => {}
        }
        return events;
    }

    let file_type = if local_path.is_file() { FileType::File } else { FileType::Dir };

    match result.kind {
        EventKind::Modify(kind) => {
            match kind {
                ModifyKind::Name(_) => {
                    events.push(SyncEvent {
                        source_id: config.id.clone(),
                        base: base.clone(),
                        file_type,
                        kind: SyncEventKind::Moved,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        old_local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: get_now_as_millis(),
                    })
                }
                _ => {
                    events.push(SyncEvent {
                        source_id: config.id.clone(),
                        base: base.clone(),
                        file_type,
                        kind: SyncEventKind::Updated,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        old_local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: get_now_as_millis(),
                    })
                }
            }
        }
        EventKind::Create(_) => {
            events.push(SyncEvent {
                source_id: config.id.clone(),
                base: base.clone(),
                file_type,
                kind: SyncEventKind::Created,
                update_hash: "".to_string(),
                size: 0,
                local_path,
                old_local_path,
                sync_path,
                old_sync_path,
                timestamp: get_now_as_millis(),
            })
        }
        EventKind::Remove(_) => {
            events.push(SyncEvent {
                source_id: config.id.clone(),
                base: base.clone(),
                file_type,
                kind: SyncEventKind::Deleted,
                update_hash: "".to_string(),
                size: 0,
                local_path,
                old_local_path,
                sync_path,
                old_sync_path,
                timestamp: get_now_as_millis(),
            })
        }
        _ => {}
    }

    events
}

#[derive(Debug, Clone)]
struct FileLifetime {
    events: Vec<SyncEvent>,
    next: Option<String>,
}


fn resolve_event_pair(a: &SyncEvent, b: &SyncEvent) -> Vec<SyncEvent> {
    match a.kind {
        SyncEventKind::Deleted => {
            vec![b.clone()]
        }
        SyncEventKind::Created => {
            match b.kind {
                SyncEventKind::Created | SyncEventKind::Updated => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Created,
                        ..b.clone()
                    }]
                }
                SyncEventKind::Deleted => {
                    vec![b.clone()]
                }
                SyncEventKind::Moved => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Created,
                        old_sync_path: b.sync_path.clone(),
                        update_hash: a.update_hash.clone(),
                        size: a.size,
                        ..b.clone()
                    }]
                }
            }
        }
        SyncEventKind::Updated => {
            match b.kind {
                SyncEventKind::Created | SyncEventKind::Updated => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Updated,
                        ..b.clone()
                    }]
                }
                SyncEventKind::Deleted => {
                    vec![b.clone()]
                }
                SyncEventKind::Moved => {
                    vec![
                        SyncEvent {
                            kind: SyncEventKind::Deleted,
                            ..a.clone()
                        },
                        SyncEvent {
                            kind: SyncEventKind::Created,
                            update_hash: a.update_hash.clone(),
                            size: a.size,
                            ..b.clone()
                        },
                    ]
                }
            }
        }
        SyncEventKind::Moved => {
            match b.kind {
                SyncEventKind::Created | SyncEventKind::Updated => {
                    vec![
                        SyncEvent {
                            kind: SyncEventKind::Deleted,
                            sync_path: a.old_sync_path.clone(),
                            ..a.clone()
                        },
                        SyncEvent {
                            kind: SyncEventKind::Created,
                            update_hash: a.update_hash.clone(),
                            size: a.size,
                            ..b.clone()
                        },
                    ]
                }
                SyncEventKind::Deleted => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Deleted,
                        sync_path: a.old_sync_path.clone(),
                        ..a.clone()
                    }, b.clone()]
                }
                SyncEventKind::Moved => {
                    if a.old_sync_path == b.sync_path {
                        vec![]
                    } else {
                        vec![SyncEvent {
                            old_sync_path: a.old_sync_path.clone(),
                            ..b.clone()
                        }]
                    }
                }
            }
        }
    }
}

fn event_time_cmp(a: &SyncEvent, b: &SyncEvent) -> Ordering {
    a.timestamp.cmp(&b.timestamp)
}

pub fn optimize_events(events: &Vec<SyncEvent>) -> Vec<SyncEvent> {
    let mut file_lifetimes: HashMap<String, FileLifetime> = HashMap::new();
    for event in events {
        let lifetime = file_lifetimes.entry(event.old_sync_path.clone()).or_insert(FileLifetime {
            events: Vec::new(),
            next: None,
        });
        lifetime.events.push(event.clone());

        if event.kind == SyncEventKind::Moved {
            lifetime.next = Some(event.sync_path.clone());
        }
    }

    let mut file_keys = Vec::new();
    for key in file_lifetimes.keys() {
        if file_lifetimes.values().find(|e| e.next.as_ref().is_some_and(|v| v == key.deref())).is_some() {
            file_keys.push(key.clone())
        } else {
            file_keys.insert(0, key.clone())
        }
    }

    let mut new_events = Vec::new();

    for key in file_keys {
        let mut file_lifecycle_events = Vec::new();
        let entry = file_lifetimes.remove(&key);
        if entry.is_none() { continue; }
        let mut entry = entry.unwrap();
        file_lifecycle_events.extend(entry.events);
        while entry.next.is_some() {
            let next_entry = file_lifetimes.remove(entry.next.as_ref().unwrap());
            if next_entry.is_none() { break; }
            entry = next_entry.unwrap();
            file_lifecycle_events.extend(entry.events);
        }
        file_lifecycle_events.sort_by(event_time_cmp);
        let mut events_count = file_lifecycle_events.len();
        loop {
            let mut start_index = 0;
            while start_index < file_lifecycle_events.len() && file_lifecycle_events[start_index..].len() > 1 {
                let new_events = resolve_event_pair(
                    &file_lifecycle_events.remove(start_index),
                    &file_lifecycle_events.remove(start_index),
                );
                let e_len = new_events.len();
                for (i, e) in new_events.into_iter().enumerate() {
                    file_lifecycle_events.insert(start_index + i, e);
                }
                if e_len > 1 {
                    start_index += e_len - 1;
                }
            }
            if file_lifecycle_events.len() >= events_count { break; }
            events_count = file_lifecycle_events.len();
        }
        new_events.extend(file_lifecycle_events);
    }

    new_events
}

pub fn filter_events(config: &SherryConfigSourceJSON, events: &Vec<SyncEvent>) -> Vec<SyncEvent> {
    let globs: Vec<Pattern> = config.allowed_file_names.iter()
        .filter_map(|s| match Pattern::new(s) {
            Ok(m) => Some(m),
            Err(_) => None
        }).collect();

    events.iter().filter_map(|e| {
        if !config.allow_dir && e.sync_path.contains(PATH_SEP) {
            return None;
        }

        if !globs.is_empty() && !globs.iter().any(|p| p.matches(&e.sync_path)) {
            return None;
        }

        if e.kind == SyncEventKind::Deleted {
            return Some(e.clone());
        }

        let metadata = e.local_path.metadata();
        if metadata.is_err() {
            return None;
        }
        let metadata = metadata.unwrap();
        if metadata.len() > config.max_file_size {
            return None;
        }

        Some(SyncEvent {
            size: if metadata.is_dir() { 0 } else { metadata.len() },
            ..e.clone()
        })
    }).collect()
}

pub async fn complete_events(events: &Vec<SyncEvent>) -> Vec<SyncEvent> {
    futures::future::join_all(events.iter().map(|e| async {
        SyncEvent {
            update_hash: get_file_hash(&e.local_path).await,
            ..e.clone()
        }
    })).await.into_iter().collect()
}
