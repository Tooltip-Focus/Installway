use serde::{Deserialize, Serialize};

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
    /// Feature-pack id this shortcut is tied to.
    #[serde(default)]
    pub feature: String,
}
