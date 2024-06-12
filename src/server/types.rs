use serde::{Deserialize, Serialize};
use serde_diff::SerdeDiff;

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiAuthResponse {
    pub user_id: String,
    pub email: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64, // timestamp in seconds
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFileResponse {
    pub sherry_id: String,
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub created_at: i128,
    pub updated_at: i128,
}


#[derive(SerdeDiff, Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ApiFolderPermissionAccessRights {
    Read,
    Write,
    Owner,
}

#[derive(SerdeDiff, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderPermissionResponse {
    pub sherry_permission_id: String,
    pub role: ApiFolderPermissionAccessRights,
    pub sherry_id: String,
    pub user_id: String,
}


#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderAllowedFileNameResponse {
    pub file_name_id: String,
    pub name: String,
    pub sherry_id: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderAllowedFileTypeResponse {
    pub file_type_id: String,
    pub _type: String,
    pub sherry_id: String,
}

#[derive(SerdeDiff, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiFolderResponse {
    pub sherry_id: String,
    pub name: String,
    pub allow_dir: bool,
    pub user_id: String,
    pub max_file_size: u64,
    pub max_dir_size: u64,
    pub allowed_file_names: Vec<ApiFolderAllowedFileNameResponse>,
    pub allowed_file_types: Vec<ApiFolderAllowedFileTypeResponse>,
    pub sherry_permission: Vec<ApiFolderPermissionResponse>,
}
