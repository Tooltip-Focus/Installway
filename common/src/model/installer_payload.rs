use crate::model::file_assoc::FileAssoc;
use crate::model::install_dir_restriction::InstallDirRestriction;
use crate::model::launch_option::LaunchOption;
use crate::model::manifest::Manifest;
use crate::model::payload_kind::PayloadKind;
use crate::model::plugin_entry::PluginEntry;
use crate::model::plugin_phase::PluginPhase;
use crate::model::registry_entry::RegistryEntry;
use crate::model::registry_kind::RegistryKind;
use crate::model::registry_value::RegistryValue;
use crate::model::shortcut_entry::ShortcutEntry;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallerPayload {
    pub kind: PayloadKind,
    /// Human-facing display name: ARP `DisplayName`, version-info ProductName,
    /// installer/uninstaller UI text, and the shortcut label.
    pub product: String,
    #[serde(default)]
    pub product_id: String,
    /// Used for the per-user uninstall data folder
    /// and the Add/Remove Programs "Publisher" field.
    #[serde(default)]
    pub publisher: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub min_installer_version: String,
    pub payload_blake3: String,
    pub created_at_unix: i64,
    pub manifest: Manifest,
    /// Optional EULA text shown on the License page of the installer UI.
    /// `None` falls back to a built-in placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
    /// File-type associations to register under `HKCU\Software\Classes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts (`.lnk`) to create at install.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Ignore the installed version and reinstall from scratch
    /// (rewrite all files, remove orphans).
    #[serde(default)]
    pub force_reinstall: bool,
    /// Remove existing files not in this build's manifest (unknown / leftover
    /// files) during a Full install. Opt-in so an upgrade or
    /// reinstall leaves a clean directory. Unlike [`force_reinstall`], known files
    /// are still hash-skipped (not rewritten).
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
    /// Default install directory the UI proposes.
    /// May contain `%VAR%` env tokens (e.g. `%LOCALAPPDATA%\Programs\MyApp`).
    /// `None` falls back to `%LOCALAPPDATA%\Programs\<product>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_install_dir: Option<String>,
    /// When set, an *upgrade* (a run over an already-installed copy) uses the
    /// compact minimal UI instead of the full wizard. The first install always
    /// uses the full wizard.
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    /// Free-form registry entries (HKCU) written at install and removed at
    /// uninstall.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegistryEntry>,
    /// Native DLL plugins bundled in the payload zip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default;
    #[serde(default)]
    pub show_uninstall_complete: bool,
    #[serde(default)]
    pub launch_option: LaunchOption,
    /// Feature packs resolved active for this run.
    #[serde(skip)]
    pub active_features: Vec<String>,
}

impl Default for InstallerPayload {
    fn default() -> Self {
        Self {
            kind: PayloadKind::Patch,
            product: "P".into(),
            product_id: "P_id".into(),
            publisher: "Pub".into(),
            from_version: Some("1.0".into()),
            to_version: "1.1".into(),
            min_installer_version: "1.0.0".into(),
            payload_blake3: "abc".into(),
            created_at_unix: 123,
            manifest: Manifest {
                version: "1.1".into(),
                exe: Some("a.exe".into()),
                files: Default::default(),
                deleted_files: vec![],
                full_size: 0,
                total_patch_size: 0,
                features: vec![],
                default_features: vec![],
                feature_mode: Default::default(),
            },
            license_text: None,
            associations: vec![FileAssoc {
                ext: ".x".into(),
                description: "X".into(),
            }],
            shortcuts: vec![ShortcutEntry {
                dir: r"%DESKTOP%".into(),
                name: "P".into(),
                target: "a.exe".into(),
                args: "--flag".into(),
                feature: String::new(),
            }],
            force_reinstall: true,
            purge_unknown_files: true,
            skip_license: true,
            skip_path: false,
            install_dir_restriction: InstallDirRestriction::DefaultDirOnly,
            default_install_dir: Some(r"%LOCALAPPDATA%\Programs\P".into()),
            upgrade_minimal_ui: true,
            registry: vec![RegistryEntry {
                hive: "HKCU".into(),
                key: r"Software\Acme\App".into(),
                name: "Build".into(),
                kind: RegistryKind::Dword,
                value: RegistryValue::Int(42),
            }],
            plugins: vec![PluginEntry {
                name: "p1".into(),
                file: "plugins/p1.dll".into(),
                blake3: "abc".into(),
                phase: PluginPhase::PreInstall,
                required: true,
                ui: true,
            }],
            show_uninstall_complete: true,
            launch_option: LaunchOption::Unchecked,
            active_features: vec![],
        }
    }
}
