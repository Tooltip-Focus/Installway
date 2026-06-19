use serde::{Deserialize, Serialize};

/// When a [`PluginEntry`] runs at install.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginPhase {
    /// Before any file is staged/committed. A required failure aborts cleanly.
    #[default]
    PreInstall,
    /// After the install is finalized (files in place, product registered).
    PostInstall,
}
