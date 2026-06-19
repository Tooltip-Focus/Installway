use crate::model::choice_option::ChoiceOption;
use crate::model::choice_style::ChoiceStyle;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One form control. `kind` is the serde tag; each maps to a built-in Win32
/// control. Unknown kinds are rejected at parse — the host must be able to draw
/// whatever it is handed.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginWidget {
    /// Static read-only text; contributes no value.
    Label {
        #[serde(default)]
        id: String,
        text: String,
    },
    /// Free text entry. `password` masks the input, `number` restricts to digits,
    /// `multiline` makes a taller box. Value is the typed string.
    Text {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        default: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        password: bool,
        #[serde(default)]
        number: bool,
        #[serde(default)]
        multiline: bool,
    },
    /// On/off; value is `"true"` / `"false"`.
    Checkbox {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        default: bool,
    },
    /// Pick one of `options`; value is the chosen option's `value`.
    SingleChoice {
        id: String,
        #[serde(default)]
        label: String,
        options: Vec<ChoiceOption>,
        #[serde(default)]
        style: ChoiceStyle,
        /// Option `value` selected initially; empty = first option.
        #[serde(default)]
        default: String,
        #[serde(default = "default_true")]
        required: bool,
    },
    /// Pick any number of `options` (a checkbox group). Value is the selected
    /// option `value`s joined by `,` (empty when none).
    MultiChoice {
        id: String,
        #[serde(default)]
        label: String,
        options: Vec<ChoiceOption>,
        #[serde(default)]
        default: Vec<String>,
        #[serde(default)]
        required: bool,
    },
    /// Progress bar widget. No value collected.
    /// `marquee: true` (default): infinite animation.
    /// `marquee: false`: deterministic 0–100 bar; plugin drives the percentage
    /// via `emit_progress` calls from `installway_up`.
    Progress {
        #[serde(default = "default_true")]
        marquee: bool,
    },
}
