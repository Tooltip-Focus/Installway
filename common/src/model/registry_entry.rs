use crate::model::registry_kind::RegistryKind;
use crate::model::registry_value::RegistryValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegistryEntry {
    /// Hive
    pub hive: String,
    /// Subkey path under the hive, e.g. `Software\Acme\App`.
    pub key: String,
    /// Value name; empty = the key's `(Default)` value.
    #[serde(default)]
    pub name: String,
    pub kind: RegistryKind,
    pub value: RegistryValue,
}
