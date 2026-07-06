use crate::model::file_assoc::FileAssoc;
use crate::model::plugin_entry::PluginEntry;
use crate::model::registry_entry::RegistryEntry;
use crate::model::shortcut_entry::ShortcutEntry;
use serde::{Deserialize, Serialize};

/// Persisted to `<install_dir>/installer_info.json` by the installer.
/// Read by the uninstaller (and any tooling) to locate registry entries
/// and walk the manifest for cleanup.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallInfo {
    pub product: String,
    #[serde(default)]
    pub product_id: String,
    #[serde(default)]
    pub publisher: String,
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
    /// Feature packs staged for this install. Persisted so the next upgrade keeps
    /// the same set by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}
