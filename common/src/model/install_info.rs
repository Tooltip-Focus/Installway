use crate::model::file_assoc::FileAssoc;
use crate::model::plugin_entry::PluginEntry;
use crate::model::registry_entry::RegistryEntry;
use crate::model::shortcut_entry::ShortcutEntry;
use crate::model::uninstall_dir_policy::UninstallDirPolicy;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Persisted to `<install_dir>/installer_info.json` by the installer.
/// Read by the uninstaller (and any tooling) to locate registry entries
/// and walk the manifest for cleanup.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct InstallInfo {
    pub product: String,
    #[serde(default)]
    pub product_id: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hintway_tenant_id: Option<String>,
    pub version: String,
    pub install_dir: String,
    pub installed_at_unix: i64,
    /// HKCU subkey under `Software\Microsoft\Windows\CurrentVersion\Uninstall`.
    pub registry_key: String,
    /// Optional path (relative to install_dir) of the product's main exe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// File associations registered at install time.
    #[serde(default)]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts created at install, the uninstaller
    /// removes exactly these, and an upgrade reconciles a changed set.
    #[serde(default)]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Resolved registry entries written at install, the uninstaller removes
    /// exactly these.
    #[serde(default)]
    pub registry: Vec<RegistryEntry>,
    /// Plugins recorded at install, the uninstaller runs their `down`.
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default;
    #[serde(default)]
    pub show_uninstall_complete: bool,
    /// `true` when the install directory required administrator rights.
    #[serde(default)]
    pub requires_admin: bool,
    /// Feature packs staged for this install. Persisted so the next upgrade knows
    /// which feature files are on disk and can clean up any it deactivates, and
    /// (under `feature_mode = "sticky"`) so it can re-seed the active base from
    /// this set instead of the new build's defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// What the uninstaller removes from the install directory.
    #[serde(default)]
    pub uninstall_dir_policy: UninstallDirPolicy,
    /// Absolute paths of the directories the installer created: the install
    /// dir and its missing ancestors, plus the payload's sub-directories. Merged
    /// across upgrades. A directory absent from this list existed before the
    /// install and is never removed under [`UninstallDirPolicy::Tracked`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_dirs: Vec<String>,
}

impl InstallInfo {
    /// Its file name in the uninstall data dir.
    pub const FILE: &str = "installer_info.json";

    /// The record kept in the uninstall data dir `data_dir`.
    pub fn read(data_dir: &Path) -> anyhow::Result<Self> {
        crate::utils::read_json(data_dir, Self::FILE)
    }
}
