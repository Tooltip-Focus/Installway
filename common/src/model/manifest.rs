use crate::model::file_entry::FileEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub exe: String,
    pub files: HashMap<String, FileEntry>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    #[serde(default)]
    pub full_size: u64,
    #[serde(default)]
    pub total_patch_size: u64,
}
