use crate::model::patch_info::PatchInfo;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    pub hash: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<PatchInfo>,
    /// Feature pack this file belongs to. `None` = always-installed base;
    /// `Some(id)` = staged only when that feature is active for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
}
