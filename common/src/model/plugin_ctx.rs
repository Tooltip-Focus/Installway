use crate::model::install_info::InstallInfo;
use crate::model::installer_payload::InstallerPayload;
use std::path::Path;

/// Run context, sent to the child as JSON on stdin.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct PluginCtx {
    pub install_dir: String,
    /// Per-user data dir (where `installer_info.json` lives). The place for plugin
    /// state that should persist across upgrades; the uninstaller deletes it.
    #[serde(default)]
    pub data_dir: String,
    pub product: String,
    pub product_id: String,
    pub version: String,
    pub exe: String,
    pub log_path: String,
    /// `up` only: the user's page answers, keyed `"<page_id>.<widget_id>"`.
    #[serde(default)]
    pub inputs_json: String,
    /// Host UI language code (2 ISO-639 chars, e.g. `"en"`/`"fr"`), so a plugin
    /// can localize its pages/log to match the installer. Empty (older records)
    /// is treated as the default language.
    #[serde(default)]
    pub lang: String,
    /// Feature-pack catalog for `installway_pages` / `installway_features`: a JSON
    /// object `{ "all": [...], "active": [...] }` (declared features + the current
    /// base set), so a plugin can render a pre-checked picker. Empty when the
    /// build declares no features.
    #[serde(default)]
    pub features_json: String,
}

impl PluginCtx {
    pub fn for_install(
        payload: &InstallerPayload,
        install_dir: &Path,
        requires_admin: bool,
    ) -> Self {
        let data_dir = crate::paths::uninstall_dir_for(
            &payload.publisher,
            &payload.product_id,
            requires_admin,
        )
        .unwrap_or_else(|| install_dir.to_path_buf());
        Self {
            install_dir: install_dir.to_string_lossy().into_owned(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            product: payload.product.clone(),
            product_id: payload.product_id.clone(),
            version: payload.to_version.clone(),
            exe: exe_path(install_dir, &payload.manifest.exe),
            ..Self::with_host_env()
        }
    }

    pub fn for_uninstall(info: &InstallInfo, data_dir: &Path) -> Self {
        Self {
            install_dir: info.install_dir.clone(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            product: info.product.clone(),
            product_id: info.product_id.clone(),
            version: info.version.clone(),
            exe: exe_path(Path::new(&info.install_dir), &info.exe),
            ..Self::with_host_env()
        }
    }

    fn with_host_env() -> Self {
        Self {
            log_path: crate::log::current_path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            lang: crate::i18n::current_lang().to_string(),
            ..Self::default()
        }
    }
}

/// Normalize `install_dir` joined with `exe_rel` to a backslash path string
/// (plugins receive Windows-style paths).
fn exe_path(install_dir: &Path, exe_rel: &str) -> String {
    install_dir
        .join(exe_rel)
        .to_string_lossy()
        .replace('/', "\\")
}
