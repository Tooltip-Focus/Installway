use crate::model::plugin_page::PluginPage;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One step in a `ui = true` plugin's wizard flow. `installway_pages` is a pure
/// step function: the host calls it with the answers so far (`ctx.inputs_json`)
/// and the plugin returns the next [`PageStep`]. The installer renders pages with
/// its own Win32 controls — the plugin never draws UI.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum PageStep {
    /// Show this page next.
    Page {
        page: PluginPage,
        /// Optional banner shown above the page (e.g. a validation error so the
        /// plugin can re-ask).
        #[serde(default)]
        notice: String,
        /// Allow the Back button on this page (when there's somewhere to go back
        /// to). The plugin can set `false` to pin the user here.
        #[serde(default = "default_true")]
        back: bool,
    },
    /// No more pages — proceed to install.
    Done,
}
