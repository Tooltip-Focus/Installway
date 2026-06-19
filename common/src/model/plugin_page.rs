use crate::model::plugin_widget::PluginWidget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_true() -> bool {
    true
}

/// One contributed wizard page.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginPage {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub widgets: Vec<PluginWidget>,
    /// Show Back/Next/Cancel buttons.
    #[serde(default = "default_true")]
    pub buttons: bool,
}

/// Collected page answers, keyed `"<page_id>.<widget_id>"`. `BTreeMap` keeps a
/// deterministic order (stable logs/tests). Serialized into `PluginCtx.inputs_json`.
/// `MultiChoice` answers join selected values with `,`; option `value` strings must
/// not themselves contain `,` (no escaping is applied).
pub type PluginInputs = BTreeMap<String, String>;
