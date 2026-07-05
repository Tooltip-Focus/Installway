use crate::model::default_true;
use crate::model::plugin_phase::PluginPhase;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginEntry {
    pub name: String,
    /// In-zip / data-dir-relative path, e.g. `plugins/<name>.dll`.
    pub file: String,
    pub blake3: String,
    pub phase: PluginPhase,
    /// A required plugin's `up` failure fails the install. Default `true`.
    #[serde(default = "default_true")]
    pub required: bool,
    /// This plugin contributes custom wizard pages. When set, the host
    /// queries its `installway_pages` before showing the wizard.
    #[serde(default)]
    pub ui: bool,
}
