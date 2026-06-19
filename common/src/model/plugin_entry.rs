use crate::model::plugin_phase::PluginPhase;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// A native DLL plugin (migration-style `up`/`down`). The DLL lives in the
/// signed payload zip at `file` and is copied to the per-user data dir for the
/// uninstall `down`. `blake3` is verified before the DLL is loaded.
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
    /// Opt-in: this plugin contributes custom wizard pages. When set, the host
    /// queries its `installway_pages` before showing the wizard.
    #[serde(default)]
    pub ui: bool,
}
