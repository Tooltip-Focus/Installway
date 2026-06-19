use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchInfo {
    pub file: String,
    #[serde(default)]
    pub size: u64,
}
