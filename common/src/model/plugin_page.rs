use crate::model::default_true;
use crate::model::plugin_widget::PluginWidget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// Collected page answers, keyed `"<page_id>.<widget_id>"`.
pub type PluginInputs = BTreeMap<String, String>;
