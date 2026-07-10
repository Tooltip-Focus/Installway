use serde::{Deserialize, Serialize};

/// How an install seeds the "to activate" feature-pack base — the starting set a
/// plugin's `{ enable, disable }` delta is applied on top of. Only matters on an
/// **upgrade** over a copy that recorded its features; a fresh install seeds from
/// the build defaults either way. Set per build in `pack.toml` (`feature_mode`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeatureMode {
    /// Default: an upgrade inherits the previously-installed feature set as the
    /// base, so features stay put unless a plugin (or a probe) changes them. The
    /// build defaults seed only a fresh install.
    #[default]
    Sticky,
    /// Every install re-seeds the base from *this* build's declared default
    /// features, fresh installs and upgrades alike. The running build's set wins;
    /// a feature a prior install added is dropped unless this build defaults it on
    /// or a plugin re-enables it.
    Override,
}

impl FeatureMode {
    /// True for the default (`Sticky`), so serialization can omit the field.
    pub fn is_default(&self) -> bool {
        matches!(self, FeatureMode::Sticky)
    }
}
