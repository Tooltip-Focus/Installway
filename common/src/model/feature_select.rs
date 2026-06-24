use serde::{Deserialize, Serialize};

/// A plugin's feature delta, emitted by `installway_features`. The host resolves
/// the active set as `(base ∪ enable) \ disable`.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FeatureSelection {
    #[serde(default)]
    pub enable: Vec<String>,
    /// Wins over `enable` and the base set.
    #[serde(default)]
    pub disable: Vec<String>,
}

impl FeatureSelection {
    /// `(base ∪ enable) \ disable`, sorted and de-duped.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_descriptor_parses() {
        let s: FeatureSelection = serde_json::from_str("{}").unwrap();
        assert!(s.enable.is_empty());
        assert!(s.disable.is_empty());
    }

    #[test]
    fn enable_disable_parse() {
        let s: FeatureSelection =
            serde_json::from_str(r#"{"enable":["A","B"],"disable":["C"]}"#).unwrap();
        assert_eq!(s.enable, vec!["A", "B"]);
        assert_eq!(s.disable, vec!["C"]);
    }

    #[test]
    fn resolve_unions_then_subtracts() {
        let base = vec!["Feat1".to_string()];
        let deltas = vec![
            FeatureSelection {
                enable: vec!["Feat2".into()],
                disable: vec![],
            },
            FeatureSelection {
                enable: vec!["Feat3".into()],
                disable: vec!["Feat1".into()],
            },
        ];
        let active = FeatureSelection::resolve(&base, &deltas);
        // Feat1 removed by disable; Feat2 + Feat3 added; sorted + de-duped.
        assert_eq!(active, vec!["Feat2".to_string(), "Feat3".to_string()]);
    }

    #[test]
    fn disable_beats_enable() {
        let deltas = vec![FeatureSelection {
            enable: vec!["X".into()],
            disable: vec!["X".into()],
        }];
        assert!(FeatureSelection::resolve(&[], &deltas).is_empty());
    }

    #[test]
    fn resolve_dedups_and_sorts() {
        // The same id from the base and two plugins collapses to one entry, and
        // the result is sorted regardless of input order.
        let base = vec!["B".to_string()];
        let deltas = vec![
            FeatureSelection {
                enable: vec!["B".into(), "A".into()],
                disable: vec![],
            },
            FeatureSelection {
                enable: vec!["A".into()],
                disable: vec![],
            },
        ];
        assert_eq!(
            FeatureSelection::resolve(&base, &deltas),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn resolve_empty_is_empty() {
        assert!(FeatureSelection::resolve(&[], &[]).is_empty());
    }
}
