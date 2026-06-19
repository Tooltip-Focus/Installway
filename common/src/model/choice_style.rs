use serde::{Deserialize, Serialize};

/// How a [`PluginWidget::SingleChoice`] is drawn.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceStyle {
    #[default]
    Radio,
    Combo,
}
