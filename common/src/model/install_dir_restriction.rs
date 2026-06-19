use serde::{Deserialize, Serialize};

/// Whether the *interactive* installer lets a fresh install target a non-empty
/// folder. A first install normally blocks a non-empty destination (a guard
/// against picking the wrong folder). These relax it for apps that must install
/// over an existing layout - e.g. replacing a legacy InstallShield or MSI
/// install in its own directory, where a (pre-install) plugin validates the old
/// install and a `purge_unknown_files` + uninstall `down` plugin clean it up.
/// Only affects the Choose-location page; silent/headless installs never run
/// this guard.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InstallDirRestriction {
    /// Default: block a fresh install into a non-empty folder.
    #[default]
    Enforce,
    /// Allow a non-empty folder only when it is the build-time default install
    /// dir (the known legacy location). Any other non-empty folder is blocked.
    DefaultDirOnly,
    /// Allow any non-empty folder.
    Bypass,
}
