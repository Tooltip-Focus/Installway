use crate::model::file_entry::FileEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Manifest {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    pub files: HashMap<String, FileEntry>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    #[serde(default)]
    pub full_size: u64,
    #[serde(default)]
    pub total_patch_size: u64,
    /// Feature ids declared by the build. Empty when there are no feature packs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// Subset of `features` enabled by default on a fresh install (a plugin can
    /// still override). Empty means every feature is opt-in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_features: Vec<String>,
}

impl Manifest {
    /// Minimal stand-in when the recorded manifest is missing or unreadable:
    /// file removal no-ops, everything else (shortcuts, registry, dirs) runs.
    pub fn fallback(version: &str, exe: Option<&str>) -> Self {
        Manifest {
            version: version.to_string(),
            exe: exe.map(|s| s.to_string()),
            ..Default::default()
        }
    }
}
