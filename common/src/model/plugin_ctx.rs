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
