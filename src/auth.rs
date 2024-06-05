use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::config::SherryConfigJSON;
use crate::constants::{AUTH_FILE, EXPIRATION_THRESHOLD};
use crate::files::{initialize_json_file, read_json_file, write_json_file};
use crate::helpers::ordered_map;
use crate::server::api::{ApiAuthResponse, ApiClient};

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub user_id: String,
    pub email: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64, // timestamp in seconds
    pub expired: bool,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryAuthorizationConfigJSON {
    pub default: String,
    // user_id => credentials
    #[serde(serialize_with = "ordered_map")]
    pub records: HashMap<String, Credentials>,
}

pub async fn read_auth_config(dir: &Path) -> Result<SherryAuthorizationConfigJSON, String> {
    read_json_file(dir.join(AUTH_FILE)).await
}

pub async fn write_auth_config(dir: &Path, config: &SherryAuthorizationConfigJSON) -> Result<(), String> {
    write_json_file(dir.join(AUTH_FILE), config).await
}

pub async fn initialize_auth_config(dir: &PathBuf) -> Result<SherryAuthorizationConfigJSON, String> {
    initialize_json_file(dir.join(AUTH_FILE), SherryAuthorizationConfigJSON {
        default: "".to_string(),
        records: HashMap::new(),
    }).await
}

fn response_to_user(response: ApiAuthResponse) -> Credentials {
    Credentials {
        user_id: response.user_id,
        email: response.email,
        username: response.username,
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        expires_in: response.expires_in,
        expired: false,
    }
}

pub struct RevalidateAuthMeta {
    pub new_users: Vec<Credentials>,
    pub deleted_users: Vec<Credentials>,
    pub valid_users: Vec<Credentials>,
    pub updated_users: Vec<Credentials>,
    pub invalid_users: Vec<Credentials>,
}

pub async fn revalidate_auth(new: &SherryAuthorizationConfigJSON, old: &SherryAuthorizationConfigJSON, config: &SherryConfigJSON) -> (SherryAuthorizationConfigJSON, RevalidateAuthMeta) {
    let mut auth = new.clone();
    let api_url = &config.api_url;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i32;

    if auth.records.iter().find(|(_, u)| u.user_id == auth.default).is_none() {
        auth.default = "".to_string();
    }

    let mut new_users: Vec<Credentials> = vec![];
    let mut deleted_users: Vec<Credentials> = vec![];
    let mut valid_users: Vec<Credentials> = vec![];
    let mut updated_users: Vec<Credentials> = vec![];
    let mut invalid_users: Vec<Credentials> = vec![];
    for (key, user) in auth.records.iter() {
        let mut user = user.clone();

        if !user.expired {
            let user_expiration = user.expires_in as i32;
            if user_expiration < now {
                user.expired = true
            } else if user_expiration - EXPIRATION_THRESHOLD <= now {
                log::info!("Refreshing token for {}", user.username);
                match ApiClient::new(api_url, &user.access_token).refresh_token(&user.refresh_token).await {
                    Err(_) => user.expired = true,
                    Ok(v) => user = response_to_user(v)
                };
            }
        }

        if old.records.contains_key(key) {
            if &user != old.records.get(key).unwrap() {
                updated_users.push(user);
            } else {
                valid_users.push(user);
            }
        } else {
            new_users.push(user);
        }
    }
    for (key, user) in old.records.iter() {
        if !auth.records.contains_key(key) {
            deleted_users.push(user.clone());
        }
    }

    for user in &updated_users {
        auth.records.insert(user.user_id.clone(), user.clone());
    }

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
