use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::ops::Deref;
use std::path::PathBuf;
use std::time::SystemTime;

use glob::Pattern;
use notify::event::{DataChange, ModifyKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;

use crate::config::SherryConfigSourceJSON;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SyncEventKind {
    Create,
    Update,
    Rename,
    Delete,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FileType {
    Dir,
    File,
}

#[derive(Debug, Clone)]
pub struct SyncEvent {
    pub file_type: FileType,
    pub kind: SyncEventKind,
    pub local_path: PathBuf,
    pub sync_path: String,
    pub old_sync_path: String,
    pub update_hash: String,
    pub size: u64,
    pub timestamp: SystemTime,
}

pub fn print_events(name: &str, events: &Vec<SyncEvent>) {
    log::info!("{name} [");
    for event in events {
        log::info!("  {:?}", event)
    }
    log::info!("]")
}


fn get_file_hash(path: &PathBuf) -> String {
    if path.is_dir() {
        return "".to_string();
    }
    match fs::read(path) {
        Ok(content) => {
            seahash::hash(&content).to_string()
        }
        Err(_) => {
            "".to_string()
        }
    }
}

fn result_cmp(a: &DebouncedEvent, b: &DebouncedEvent) -> Ordering {
    a.time.cmp(&b.time)
}

pub fn minify_results(results: &Vec<DebouncedEvent>) -> Vec<DebouncedEvent> {
    let mut results = results.clone();
    results.sort_by(result_cmp);

    let mut new_results = Vec::new();
    let mut remove_results = HashMap::new();
    for (i, result) in results.iter().enumerate() {
        match result.kind {
            EventKind::Modify(modify_kind) => {
                match modify_kind {
                    ModifyKind::Name(mode) => {
                        match mode {
                            RenameMode::From => {
                                let to = results.get(i + 1);
                                if to.is_some() {
                                    let to = to.unwrap();
                                    let from = result;
                                    new_results.push(DebouncedEvent {
                                        event: notify::Event {
                                            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                                            paths: vec![from.paths.first().unwrap().clone(), to.paths.first().unwrap().clone()],
                                            attrs: to.attrs.clone(),
                                        },
                                        time: to.time,
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
                remove_results.remove(result.paths.last().unwrap());
            }
            EventKind::Remove(_) => {
                remove_results.insert(result.paths.first().unwrap(), result.clone());
            }
            EventKind::Create(_) => {
                if remove_results.get(result.paths.first().unwrap()).is_none() {
                    new_results.push(result.clone())
                } else {
                    new_results.push(DebouncedEvent {
                        event: notify::Event {
                            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                            paths: result.paths.clone(),
                            attrs: result.attrs.clone(),
                        },
                        time: result.time.clone(),
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

const PATH_SEP: &str = "/";

fn get_sync_path(config: &SherryConfigSourceJSON, path: &PathBuf, base: &PathBuf) -> String {
    format!(
        "{}:{}",
        config.id,
        path.strip_prefix(base).unwrap().iter()
            .map(|c| c.to_os_string().into_string().unwrap())
            .collect::<Vec<String>>()
            .join(PATH_SEP)
    )
}

fn get_raw_sync_path(sync_path: &String) -> String {
    sync_path.splitn(2, ":").collect::<Vec<&str>>()[1].to_string()
}

fn get_dir_file_events(config: &SherryConfigSourceJSON, path: &PathBuf, base: &PathBuf, kind: &SyncEventKind) -> Vec<SyncEvent> {
    let mut events = Vec::new();
    if path.is_file() {
        let sync_path = get_sync_path(config, &path, base);
        events.push(SyncEvent {
            file_type: FileType::File,
            kind: kind.clone(),
            local_path: path.clone(),
            old_sync_path: sync_path.clone(),
            update_hash: "".to_string(),
            size: 0,
            sync_path,
            timestamp: SystemTime::now(),
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

pub fn get_sync_events(config: &SherryConfigSourceJSON, result: DebouncedEvent, base: &PathBuf) -> Vec<SyncEvent> {
    // Modify(Any) - file update
    // Modify(Name(Both)) file/dir rename
    // Create(Any) - file/dir created
    // Remove(Any) - file(dir) removed

    let mut events = Vec::new();


    let local_path = result.paths.last().unwrap().to_path_buf();
    let old_local_path = result.paths.first().unwrap().to_path_buf();
    if local_path.is_symlink() {
        return events;
    }

    let sync_path = get_sync_path(config, &local_path, base);
    let old_sync_path = get_sync_path(config, &old_local_path, base);

    if local_path.is_dir() {
        match result.kind {
            EventKind::Modify(kind) => {
                if kind == ModifyKind::Name(RenameMode::Both) {
                    events.push(SyncEvent {
                        file_type: FileType::Dir,
                        kind: SyncEventKind::Rename,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: SystemTime::now(),
                    });
                }
            }
            EventKind::Create(_) => {
                events.extend(get_dir_file_events(config, &local_path, base, &SyncEventKind::Create));
            }
            EventKind::Remove(_) => {
                events.push(SyncEvent {
                    file_type: FileType::Dir,
                    kind: SyncEventKind::Delete,
                    update_hash: "".to_string(),
                    size: 0,
                    local_path,
                    sync_path,
                    old_sync_path,
                    timestamp: SystemTime::now(),
                });
            }
            _ => {}
        }
        return events;
    }


    match result.kind {
        EventKind::Modify(kind) => {
            match kind {
                ModifyKind::Name(_) => {
                    events.push(SyncEvent {
                        file_type: FileType::File,
                        kind: SyncEventKind::Rename,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: SystemTime::now(),
                    })
                }
                _ => {
                    events.push(SyncEvent {
                        file_type: FileType::File,
                        kind: SyncEventKind::Update,
                        update_hash: "".to_string(),
                        size: 0,
                        local_path,
                        sync_path,
                        old_sync_path,
                        timestamp: SystemTime::now(),
                    })
                }
            }
        }
        EventKind::Create(_) => {
            events.push(SyncEvent {
                file_type: FileType::File,
                kind: SyncEventKind::Create,
                update_hash: "".to_string(),
                size: 0,
                local_path,
                sync_path,
                old_sync_path,
                timestamp: SystemTime::now(),
            })
        }
        EventKind::Remove(_) => {
            events.push(SyncEvent {
                file_type: FileType::File,
                kind: SyncEventKind::Delete,
                update_hash: "".to_string(),
                size: 0,
                local_path,
                sync_path,
                old_sync_path,
                timestamp: SystemTime::now(),
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
        SyncEventKind::Delete => {
            vec![b.clone()]
        }
        SyncEventKind::Create => {
            match b.kind {
                SyncEventKind::Create | SyncEventKind::Update => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Create,
                        ..b.clone()
                    }]
                }
                SyncEventKind::Delete => {
                    vec![b.clone()]
                }
                SyncEventKind::Rename => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Create,
                        old_sync_path: b.sync_path.clone(),
                        update_hash: a.update_hash.clone(),
                        size: a.size,
                        ..b.clone()
                    }]
                }
            }
        }
        SyncEventKind::Update => {
            match b.kind {
                SyncEventKind::Create | SyncEventKind::Update => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Update,
                        ..b.clone()
                    }]
                }
                SyncEventKind::Delete => {
                    vec![b.clone()]
                }
                SyncEventKind::Rename => {
                    vec![
                        SyncEvent {
                            kind: SyncEventKind::Delete,
                            ..a.clone()
                        },
                        SyncEvent {
                            kind: SyncEventKind::Create,
                            update_hash: a.update_hash.clone(),
                            size: a.size,
                            ..b.clone()
                        },
                    ]
                }
            }
        }
        SyncEventKind::Rename => {
            match b.kind {
                SyncEventKind::Create | SyncEventKind::Update => {
                    vec![
                        SyncEvent {
                            kind: SyncEventKind::Delete,
                            sync_path: a.old_sync_path.clone(),
                            ..a.clone()
                        },
                        SyncEvent {
                            kind: SyncEventKind::Create,
                            update_hash: a.update_hash.clone(),
                            size: a.size,
                            ..b.clone()
                        },
                    ]
                }
                SyncEventKind::Delete => {
                    vec![SyncEvent {
                        kind: SyncEventKind::Delete,
                        sync_path: a.old_sync_path.clone(),
                        ..a.clone()
                    }, b.clone()]
                }
                SyncEventKind::Rename => {
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

        if event.kind == SyncEventKind::Rename {
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

        let raw_sync_path = get_raw_sync_path(&e.sync_path);

        if !globs.is_empty() && !globs.iter().any(|p| p.matches(&raw_sync_path)) {
            return None;
        }

        if e.kind == SyncEventKind::Delete {
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
            update_hash: get_file_hash(&e.local_path),
            size: if metadata.is_dir() { 0 } else { metadata.len() },
            ..e.clone()
        })
    }).collect()
}
