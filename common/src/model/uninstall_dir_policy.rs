use serde::{Deserialize, Serialize};

/// What the uninstaller does with the install directory once the tracked
/// payload files are gone. The install directory can hold files the installer
/// never wrote (added by the app, by the user, or there before the install),
/// so only [`UninstallDirPolicy::Purge`] removes content it does not track.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UninstallDirPolicy {
    /// Default: remove only the tracked files, then the directories the
    /// installer created (`InstallInfo::created_dirs`) that are left empty.
    /// A directory that existed before the install is never removed.
    #[default]
    Tracked,
    /// Remove the whole install directory, whatever it contains.
    Purge,
}
