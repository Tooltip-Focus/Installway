use serde::{Deserialize, Serialize};

/// A registry value's data. The variant is paired with a [`RegistryKind`]:
/// `Text` for sz/expand_sz (and the hex string of binary), `Int` for
/// dword/qword, `List` for multi_sz.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum RegistryValue {
    Text(String),
    Int(u64),
    List(Vec<String>),
}
