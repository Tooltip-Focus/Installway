use serde::{Deserialize, Serialize};

/// One file-type association: extension + a human description.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FileAssoc {
    /// Extension including the leading dot, e.g. ".myx".
    pub ext: String,
    /// Friendly type description shown in Explorer, e.g. "My App Document".
    pub description: String,
    /// Icon file template, expanded at install. Empty means [`Self::DEFAULT_ICON`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// `DefaultIcon` index: `>= 0` is a 0-based icon index, `< 0` is `-<resource id>`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub icon_index: i32,
}

impl FileAssoc {
    pub const DEFAULT_ICON: &'static str = "%EXE%";

    pub fn icon_template(&self) -> &str {
        if self.icon.is_empty() {
            Self::DEFAULT_ICON
        } else {
            &self.icon
        }
    }
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}
