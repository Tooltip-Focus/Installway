use serde::{Deserialize, Serialize};

/// One file-type association: extension + a human description.
/// The shell `open` verb is wired to the product's main exe with `"%1"`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileAssoc {
    /// Extension including the leading dot, e.g. ".myx".
    pub ext: String,
    /// Friendly type description shown in Explorer, e.g. "My App Document".
    pub description: String,
}
