use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use notify::event::{DataChange, ModifyKind, RenameMode};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;
use crate::config::SherryConfigSourceJSON;

#[derive(Debug, Copy, Clone)]
pub enum SyncEventKind {
    Create,
    Update,
    Rename,
    Delete,
}

#[derive(Debug)]
pub enum FileType {
    Dir,
    File,
}

#[derive(Debug)]
pub struct SyncEvent {
    pub file_type: FileType,
    pub kind: SyncEventKind,
    pub local_path: PathBuf,
    pub sync_path: String,
    pub old_sync_path: String,
    pub update_hash: String,
}

fn get_file_hash(path: &PathBuf) -> String {
    if path.is_dir() {
        return "".to_string();
    }
    match fs::read_to_string(path) {
        Ok(content) => {
            seahash::hash(content.as_bytes()).to_string()
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

fn get_sync_path(config: &SherryConfigSourceJSON, path: &PathBuf, base: &PathBuf) -> String {
    format!("{}:{}", config.id, path.strip_prefix(base).unwrap().to_str().unwrap())
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
            update_hash: get_file_hash(&path),
            sync_path,
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
                        local_path,
                        sync_path,
                        old_sync_path,
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
                    local_path,
                    sync_path,
                    old_sync_path,
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
                        local_path,
                        sync_path,
                        old_sync_path,
                    })
                }
                _ => {
                    events.push(SyncEvent {
                        file_type: FileType::File,
                        kind: SyncEventKind::Update,
                        update_hash: get_file_hash(&local_path),
                        local_path,
                        sync_path,
                        old_sync_path,
                    })
                }
            }
        }
        EventKind::Create(_) => {
            events.push(SyncEvent {
                file_type: FileType::File,
                kind: SyncEventKind::Create,
                update_hash: get_file_hash(&local_path),
                local_path,
                sync_path,
                old_sync_path,
            })
        }
        EventKind::Remove(_) => {
            events.push(SyncEvent {
                file_type: FileType::File,
                kind: SyncEventKind::Delete,
                local_path,
                sync_path,
                old_sync_path,
                update_hash: "".to_string(),
            })
        }
        _ => {}
    }

    events
}
