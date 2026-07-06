use crate::model::default_true;
use crate::model::plugin_page::PluginPage;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum PageStep {
    Page {
        page: PluginPage,
        #[serde(default)]
        notice: String,
        /// Allow the Back button on this page.
        #[serde(default = "default_true")]
        back: bool,
    },
    /// No more pages, proceed to install.
    Done,
}
