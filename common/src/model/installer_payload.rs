use crate::model::file_assoc::FileAssoc;
use crate::model::install_dir_restriction::InstallDirRestriction;
use crate::model::manifest::Manifest;
use crate::model::payload_kind::PayloadKind;
use crate::model::plugin_entry::PluginEntry;
use crate::model::reg_entry::RegEntry;
use crate::model::shortcut_entry::ShortcutEntry;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct InstallerPayload {
    pub kind: PayloadKind,
    /// Human-facing display name: ARP `DisplayName`, version-info ProductName,
    /// installer/uninstaller UI text, and the shortcut label.
    pub product: String,
    /// Registry-safe internal identifier, distinct from the display `product`.
    /// Drives the HKCU Uninstall key, association ProgIDs, the per-user data
    /// folder (`%LOCALAPPDATA%\<publisher>\Uninstall\<product_id>`) and upgrade
    /// detection. Validated at build time. `#[serde(default)]` so payloads
    /// predating the split still parse (empty → fall back to a sanitized
    /// `product`).
    #[serde(default)]
    pub product_id: String,
    /// Publisher / vendor name. Used for the per-user uninstall data folder
    /// and the Add/Remove Programs "Publisher" field. Mandatory at build time.
    #[serde(default)]
    pub publisher: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub min_installer_version: String,
    pub payload_blake3: String,
    pub created_at_unix: i64,
    pub manifest: Manifest,
    /// Optional EULA text shown on the License page of the installer UI.
    /// `None` (or missing field on older payloads) falls back to a built-in placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
    /// File-type associations to register under `HKCU\Software\Classes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts (`.lnk`) to create at install; nothing is created unless
    /// declared here. `dir`/`target`/`args` are templates expanded at install
    /// time (see the shortcut docs for the token list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Dev flag: ignore the installed version and reinstall from scratch
    /// (skip patch from-version check, rewrite all files, remove orphans).
    #[serde(default)]
    pub force_reinstall: bool,
    /// Remove existing files not in this build's manifest (unknown / leftover
    /// files) during a Full install. Opt-in at build time so an upgrade or
    /// reinstall from a full version leaves a clean directory. Ignored for
    /// patch payloads. Unlike [`force_reinstall`], known files are still
    /// hash-skipped (not rewritten) and the version check is unaffected.
    #[serde(default)]
    pub purge_unknown_files: bool,
    /// Hide the License page in the interactive UI.
    #[serde(default)]
    pub skip_license: bool,
    /// Hide the Choose-location page; install straight to the default path.
    #[serde(default)]
    pub skip_path: bool,
    /// Whether a fresh interactive install may target a non-empty folder.
    /// Defaults to [`InstallDirRestriction::Enforce`]; see that type's docs.
    #[serde(default)]
    pub install_dir_restriction: InstallDirRestriction,
    /// Default install directory the UI proposes, set per-app at build time.
    /// May contain `%VAR%` env tokens (e.g. `%LOCALAPPDATA%\Programs\MyApp`).
    /// `None` falls back to `%LOCALAPPDATA%\Programs\<product>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_install_dir: Option<String>,
    /// When set, an *upgrade* (a run over an already-installed copy) uses the
    /// compact minimal UI instead of the full wizard. The first install always
    /// uses the full wizard. Decided by this (the new installer's) payload.
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    /// Free-form registry entries (HKCU) written at install and removed at
    /// uninstall. Key/value strings are templates expanded at install time
    /// (see the registry docs for the token list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegEntry>,
    /// Native DLL plugins bundled in the payload zip, run at install.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default; enable per-app at build time.
    #[serde(default)]
    pub show_uninstall_complete: bool,
}
