use serde::{Deserialize, Serialize};

/// One shortcut (`.lnk`) the installer creates.
///
/// In the payload the strings are templates; in `InstallInfo` they are the
/// resolved values actually written (absolute `dir`/`target`), so the
/// uninstaller removes exactly the files it created and an upgrade can
/// reconcile a changed list by resolved `.lnk` path.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShortcutEntry {
    /// Directory the `.lnk` is placed in. Tokens: `%DESKTOP%`, `%START_MENU%`
    /// (per-user Programs), `%INSTALL_DIR%`, plus `%VAR%` env vars.
    pub dir: String,
    /// Shortcut file name, without the `.lnk` extension (also the label).
    pub name: String,
    /// Shortcut target. A relative path resolves against the install dir (the
    /// product exe); same tokens as `dir` are expanded.
    pub target: String,
    /// Free-form command-line arguments appended to the shortcut. Empty = none.
    #[serde(default)]
    pub args: String,
}
