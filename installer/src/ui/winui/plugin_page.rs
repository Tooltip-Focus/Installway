// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Renders a plugin-contributed [`PluginPage`] as WinUI elements.
//!
//! Every widget is a controlled control writing straight back into
//! [`Model::answers`](super::model::Model::answers), so collecting a page is a
//! map read rather than the HWND walk the Win32 backend needs. Keys are
//! `"<page_id>.<widget_id>"`, matching Win32 and the `--silent` path.

use super::compat::{Element, text_block, vstack};
use super::model::{Model, tr};
use super::wizard::SetState;
use common::model::choice_style::ChoiceStyle;
use common::model::plugin_page::{PluginInputs, PluginPage};
use common::model::plugin_widget::PluginWidget;
use windows_reactor::*;

const ROW_GAP: f64 = 10.0;

fn key(page_id: &str, widget_id: &str) -> String {
    format!("{page_id}.{widget_id}")
}

/// Seeds a freshly-shown page. Never errors: an unsatisfiable required field is
/// a validation failure at Next time, not at render time.
pub(super) fn widget_defaults(page: &PluginPage) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for w in &page.widgets {
        let (id, value) = match w {
            PluginWidget::Label { .. } | PluginWidget::Progress { .. } => continue,
            PluginWidget::Text { id, default, .. } => (id, default.clone()),
            PluginWidget::Checkbox { id, default, .. } => {
                (id, if *default { "true" } else { "false" }.to_string())
            }
            PluginWidget::SingleChoice {
                id,
                options,
                default,
                ..
            } => {
                let value = if !default.is_empty() {
                    default.clone()
                } else {
                    options.first().map(|o| o.value.clone()).unwrap_or_default()
                };
                (id, value)
            }
            PluginWidget::MultiChoice { id, default, .. } => (id, default.join(",")),
        };
        out.push((key(&page.id, id), value));
    }
    out
}

/// The first required widget left empty; `None` when the page validates.
pub(super) fn first_missing_required(page: &PluginPage, answers: &PluginInputs) -> Option<String> {
    for w in &page.widgets {
        let (id, required) = match w {
            PluginWidget::Label { .. } | PluginWidget::Progress { .. } => continue,
            PluginWidget::Text { id, required, .. }
            | PluginWidget::SingleChoice { id, required, .. }
            | PluginWidget::MultiChoice { id, required, .. } => (id, *required),
            // Always "true"/"false", so never missing.
            PluginWidget::Checkbox { .. } => continue,
        };
        if !required {
            continue;
        }
        let k = key(&page.id, id);
        if answers.get(&k).map(|v| v.trim().is_empty()).unwrap_or(true) {
            return Some(k);
        }
    }
    None
}

pub(super) fn render(page: &PluginPage, model: &Model, set: &SetState<Model>) -> Vec<Element> {
    page.widgets
        .iter()
        .map(|w| widget(page, w, model, set))
        .collect()
}

fn answer_setter(
    model: &Model,
    set: &SetState<Model>,
    key: String,
) -> impl Fn(String) + Clone + 'static {
    let model = model.clone();
    let set = set.clone();
    move |value: String| {
        let mut next = model.clone();
        next.answers.insert(key.clone(), value);
        set.call(next);
    }
}

fn widget(page: &PluginPage, w: &PluginWidget, model: &Model, set: &SetState<Model>) -> Element {
    match w {
        PluginWidget::Label { text, .. } => text_block(text.clone()).wrap().into(),

        PluginWidget::Text {
            id,
            label,
            placeholder,
            password,
            number,
            multiline,
            ..
        } => {
            let k = key(&page.id, id);
            let value = model.answers.get(&k).cloned().unwrap_or_default();
            let write = answer_setter(model, set, k);
            if *password {
                // PasswordBox has no multiline or number variant; masking wins.
                let mut b = PasswordBox::new()
                    .password(value)
                    .on_password_changed(write);
                if !placeholder.is_empty() {
                    b = b.placeholder_text(placeholder.clone());
                }
                labeled(label, b)
            } else if *number {
                // NumberBox round-trips f64, but the plugin expects the same
                // string shape the Win32 ES_NUMBER edit produced. NaN renders
                // blank, which stands in for the placeholder it lacks.
                let parsed = value.trim().parse::<f64>().unwrap_or(f64::NAN);
                let b = NumberBox::new()
                    .value((!parsed.is_nan()).then_some(parsed))
                    .on_value_changed(move |v: Option<f64>| {
                        let s = match v {
                            None => String::new(),
                            Some(v) if v.fract() == 0.0 => format!("{}", v as i64),
                            Some(v) => v.to_string(),
                        };
                        write(s);
                    });
                labeled(label, b)
            } else {
                let mut b = TextBox::new().text(value).on_text_changed(write);
                if !placeholder.is_empty() {
                    b = b.placeholder_text(placeholder.clone());
                }
                if *multiline {
                    b = b
                        .text_wrapping(TextWrapping::Wrap)
                        .accepts_return(true)
                        .height(96.0);
                }
                labeled(label, b)
            }
        }

        PluginWidget::Checkbox { id, label, .. } => {
            let k = key(&page.id, id);
            let checked = model.answers.get(&k).map(|v| v == "true").unwrap_or(false);
            let write = answer_setter(model, set, k);
            Element::from(
                CheckBox::new()
                    .is_checked(checked)
                    .on_is_checked_changed(move |v: bool| {
                        write(if v { "true".into() } else { "false".into() })
                    })
                    .content(label.clone()),
            )
        }

        PluginWidget::SingleChoice {
            id,
            label,
            options,
            style,
            ..
        } => {
            let k = key(&page.id, id);
            let current = model.answers.get(&k).cloned().unwrap_or_default();
            let selected = options.iter().position(|o| o.value == current);
            let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();
            let values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
            let write = answer_setter(model, set, k);
            let on_pick = move |i: Option<usize>| {
                if let Some(v) = i.and_then(|i| values.get(i)) {
                    write(v.clone());
                }
            };
            match style {
                ChoiceStyle::Combo => {
                    let b = ComboBox::new()
                        .items_source(labels)
                        .selected_index(selected)
                        .on_selection_changed(on_pick);
                    labeled(label, b)
                }
                ChoiceStyle::Radio => {
                    let b = RadioButtons::new()
                        .items_source(labels)
                        .selected_index(selected)
                        .on_selection_changed(on_pick);
                    labeled(label, b)
                }
            }
        }

        PluginWidget::MultiChoice {
            id, label, options, ..
        } => {
            let k = key(&page.id, id);
            let current = model.answers.get(&k).cloned().unwrap_or_default();
            let picked: Vec<&str> = current.split(',').filter(|s| !s.is_empty()).collect();
            let mut rows: Vec<Element> = Vec::with_capacity(options.len() + 1);
            if !label.is_empty() {
                rows.push(text_block(label.clone()).into());
            }
            for opt in options {
                let checked = picked.iter().any(|p| *p == opt.value);
                // Rebuild the joined value in the plugin's declared option order,
                // not the click order.
                let all: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
                let current = current.clone();
                let this = opt.value.clone();
                let write = answer_setter(model, set, k.clone());
                rows.push(Element::from(
                    CheckBox::new()
                        .is_checked(checked)
                        .on_is_checked_changed(move |on: bool| {
                            let mut set: Vec<&str> =
                                current.split(',').filter(|s| !s.is_empty()).collect();
                            if on {
                                if !set.contains(&this.as_str()) {
                                    set.push(&this);
                                }
                            } else {
                                set.retain(|v| *v != this);
                            }
                            let ordered: Vec<String> = all
                                .iter()
                                .filter(|v| set.contains(&v.as_str()))
                                .cloned()
                                .collect();
                            write(ordered.join(","));
                        })
                        .content(opt.label.clone()),
                ));
            }
            vstack(rows).spacing(6.0).into()
        }

        PluginWidget::Progress { marquee } => {
            if *marquee {
                Element::from(
                    ProgressBar::new()
                        .is_indeterminate(true)
                        .horizontal_alignment(HorizontalAlignment::Stretch),
                )
            } else {
                Element::from(
                    ProgressBar::new()
                        .value(model.plugin_progress as f64)
                        .minimum(0.0)
                        .maximum(100.0)
                        .horizontal_alignment(HorizontalAlignment::Stretch),
                )
            }
        }
    }
}

fn labeled(label: &str, control: impl Into<View>) -> Element {
    if label.is_empty() {
        Element::from(control)
    } else {
        vstack((text_block(label.to_string()), control.into()))
            .spacing(4.0)
            .into()
    }
}

pub(super) fn required_warning() -> String {
    tr().get("install.field_required")
}

pub(super) const fn row_gap() -> f64 {
    ROW_GAP
}
