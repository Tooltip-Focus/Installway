use crate::model::reg_kind::RegKind;
use crate::model::reg_value::RegValue;
use serde::{Deserialize, Serialize};

/// One free-form registry entry. In the payload the strings are templates; in
/// `InstallInfo` they are the resolved values actually written (so the
/// uninstaller can match + remove exactly what it wrote).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegEntry {
    /// Hive — `"HKCU"` only (the installer never elevates).
    pub hive: String,
    /// Subkey path under the hive, e.g. `Software\Acme\App`.
    pub key: String,
    /// Value name; empty = the key's `(Default)` value.
    #[serde(default)]
    pub name: String,
    pub kind: RegKind,
    pub value: RegValue,
}
