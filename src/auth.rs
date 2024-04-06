use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

pub const AUTH_FILE: &str = "auth.json";

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Credentials {
    email: String,
    nickname: String,
    refresh_token: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryAuthorizationConfigJSON {
    //                 tag => credentials
    records: HashMap<String, Credentials>,
}

fn get_default_auth_config() -> SherryAuthorizationConfigJSON {
    SherryAuthorizationConfigJSON {
        records: HashMap::new()
    }
}

fn get_auth_config_string(dir: &Path) -> std::io::Result<String> {
    let f = OpenOptions::new()
        .read(true)
        .open(dir.join(AUTH_FILE));
    if f.is_err() {
        return Err(f.err().unwrap());
    }
    let mut f = f.unwrap();
    let mut buf = String::new();
    f.read_to_string(&mut buf).unwrap();
    f.rewind().unwrap();
    Ok(buf)
}

fn write_auth_config(dir: &Path, config: &SherryAuthorizationConfigJSON) -> Result<(), ()> {
    match fs::write(dir.join(AUTH_FILE), serde_json::to_string_pretty(&config).unwrap()) {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn read_auth_config(dir: &Path) -> Result<SherryAuthorizationConfigJSON, String> {
    let content = get_auth_config_string(dir);
    if content.is_err() {
        let err = content.err().unwrap().to_string();
        println!("Error Read: {}", err);
        return Err(err);
    }

    let sources: serde_json::Result<SherryAuthorizationConfigJSON> = serde_json::from_str(&content.unwrap());
    if sources.is_err() {
        let err = sources.err().unwrap().to_string();
        println!("Error Parse: {}", err);
        return Err(err);
    }

    Ok(sources.unwrap())
}

pub fn initialize_auth_config(dir: &PathBuf) -> Result<SherryAuthorizationConfigJSON, ()> {
    let mut content = get_auth_config_string(dir);
    if content.is_err() {
        if write_auth_config(dir, &get_default_auth_config()).is_err() {
            return Err(());
        } else {
            content = get_auth_config_string(dir);
        }
    }

    let sources: serde_json::Result<SherryAuthorizationConfigJSON> = serde_json::from_str(&content.unwrap());
    if sources.is_err() {
        println!("Error: {}", sources.err().unwrap());
        return Err(());
    }

    Ok(sources.unwrap())
}