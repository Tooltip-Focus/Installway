use crate::model::file_assoc::FileAssoc;
use crate::model::plugin_entry::PluginEntry;
use crate::model::reg_entry::RegEntry;
use crate::model::shortcut_entry::ShortcutEntry;
use serde::{Deserialize, Serialize};

/// Persisted to `<install_dir>/installer_info.json` by the installer.
/// Read by the uninstaller (and any tooling) to locate registry entries
/// and walk the manifest for cleanup.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallInfo {
    pub product: String,
    /// Registry-safe internal id (see `InstallerPayload::product_id`). Empty on
    /// records written before the split — readers fall back to `registry_key` /
    /// a sanitized `product`.
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
    /// File associations registered at install time - the uninstaller removes
    /// exactly these.
    #[serde(default)]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts created at install (resolved absolute paths) - the uninstaller
    /// removes exactly these, and an upgrade reconciles a changed set.
    #[serde(default)]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Resolved registry entries written at install - the uninstaller removes
    /// exactly these (anti-stomp by value).
    #[serde(default)]
    pub registry: Vec<RegEntry>,
    /// Plugins recorded at install - the uninstaller runs their `down`.
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default; set per-app at build time.
    #[serde(default)]
    pub show_uninstall_complete: bool,
    /// `true` when the install directory required administrator rights. The
    /// uninstaller reads this to know upfront whether to request elevation.
    #[serde(default)]
    pub requires_admin: bool,
    /// Feature packs staged for this install. Persisted so the next upgrade keeps
    /// the same set by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}
