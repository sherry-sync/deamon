use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::constants::AUTH_FILE;
use crate::helpers::{ordered_map};
use crate::files::{initialize_json_file, read_json_file, write_json_file};

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub user_id: String,
    pub email: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryAuthorizationConfigJSON {
    pub default: String,
    // user_id => credentials
    #[serde(serialize_with = "ordered_map")]
    pub records: HashMap<String, Credentials>,
}

pub fn read_auth_config(dir: &Path) -> Result<SherryAuthorizationConfigJSON, String> {
    read_json_file(dir.join(AUTH_FILE))
}

pub fn write_auth_config(dir: &Path, config: &SherryAuthorizationConfigJSON) -> Result<(), String> {
    write_json_file(dir.join(AUTH_FILE), config)
}

pub fn initialize_auth_config(dir: &PathBuf) -> Result<SherryAuthorizationConfigJSON, String> {
    initialize_json_file(dir.join(AUTH_FILE), SherryAuthorizationConfigJSON {
        default: "".to_string(),
        records: HashMap::new(),
    })
}

pub struct RevalidateAuthMeta {
    pub new_users: Vec<Credentials>,
    pub deleted_users: Vec<Credentials>,
    pub valid_users: Vec<Credentials>,
    pub updated_users: Vec<Credentials>,
    pub invalid_users: Vec<Credentials>,
}

pub fn revalidate_auth(new: &SherryAuthorizationConfigJSON, old: &SherryAuthorizationConfigJSON) -> (SherryAuthorizationConfigJSON, RevalidateAuthMeta) {
    let mut auth = new.clone();

    if auth.records.iter().find(|(_, u)| u.user_id == auth.default).is_none() {
        auth.default = "".to_string();
    }

    let mut new_users: Vec<Credentials> = vec![];
    let mut deleted_users: Vec<Credentials> = vec![];
    let mut valid_users: Vec<Credentials> = vec![];
    let mut updated_users: Vec<Credentials> = vec![];
    let mut invalid_users: Vec<Credentials> = vec![];
    for (key, user) in auth.records.iter() {
        if old.records.contains_key(key) {
            if user != old.records.get(key).unwrap() {
                updated_users.push(user.clone());
            } else {
                valid_users.push(user.clone());
            }
        } else {
            new_users.push(user.clone());
        }
    }
    for (key, user) in old.records.iter() {
        if !auth.records.contains_key(key) {
            deleted_users.push(user.clone());
        }
    }

    // TODO: check tokens

    (
        auth,
        RevalidateAuthMeta {
            new_users,
            deleted_users,
            updated_users,
            valid_users,
            invalid_users,
        }
    )
}
