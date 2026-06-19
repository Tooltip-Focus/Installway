use serde::{Deserialize, Serialize};

/// One choice in a [`PluginWidget::SingleChoice`].
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChoiceOption {
    pub label: String,
    pub value: String,
}
