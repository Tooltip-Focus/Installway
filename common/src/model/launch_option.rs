use serde::{Deserialize, Serialize};

/// Controls the "launch the product now" checkbox on the interactive
/// installer's final (Done) page. Set per-app at build time. Has no effect on
/// silent/minimal installs.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LaunchOption {
    /// Default: checkbox visible and checked.
    #[default]
    Checked,
    /// Checkbox visible but left unchecked, the user opts in to launch.
    Unchecked,
    /// No checkbox, the installer never offers to launch the product.
    Hidden,
}
