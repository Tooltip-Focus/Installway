use serde::{Deserialize, Serialize};

/// Registry value type for a [`RegEntry`].
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegKind {
    Sz,
    ExpandSz,
    Dword,
    Qword,
    MultiSz,
    Binary,
}
