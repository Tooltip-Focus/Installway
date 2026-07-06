use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FeatureSelection {
    #[serde(default)]
    pub enable: Vec<String>,
    /// Wins over `enable`.
    #[serde(default)]
    pub disable: Vec<String>,
}

impl FeatureSelection {
    pub fn resolve(base: &[String], deltas: &[FeatureSelection]) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = base.iter().cloned().collect();
        for d in deltas {
            for f in &d.enable {
                set.insert(f.clone());
            }
        }
        for d in deltas {
            for f in &d.disable {
                set.remove(f);
            }
        }
        set.into_iter().collect()
    }
}
