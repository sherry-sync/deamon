use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

use crate::constants::AUTH_FILE;
use crate::helpers::{initialize_json_file, read_json_file, write_json_file};

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    id: String,
    email: String,
    username: String,
    access_token: String,
    refresh_token: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SherryAuthorizationConfigJSON {
    default: String,
    // user_id => credentials
    records: HashMap<String, Credentials>,
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