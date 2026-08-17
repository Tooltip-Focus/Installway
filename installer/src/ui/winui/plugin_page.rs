// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Renders a plugin-contributed [`PluginPage`] as WinUI elements.
//!
//! Every widget is a controlled control writing straight back into
//! [`Model::answers`](super::model::Model::answers), so collecting a page is a
//! map read rather than the HWND walk the Win32 backend needs. Keys are
//! `"<page_id>.<widget_id>"`, matching Win32 and the `--silent` path.

use super::model::{Model, tr};
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
                let mut b = PasswordBox::new().value(value).on_password_changed(write);
                if !label.is_empty() {
                    b = b.header(label.clone());
                }
                if !placeholder.is_empty() {
                    b = b.placeholder_text(placeholder.clone());
                }
                b.into()
            } else if *number {
                // NumberBox round-trips f64, but the plugin expects the same
                // string shape the Win32 ES_NUMBER edit produced. NaN renders
                // blank, which stands in for the placeholder it lacks.
                let parsed = value.trim().parse::<f64>().unwrap_or(f64::NAN);
                let mut b = NumberBox::new(parsed).on_value_changed(move |v: f64| {
                    let s = if v.is_nan() {
                        String::new()
                    } else if v.fract() == 0.0 {
                        format!("{}", v as i64)
                    } else {
                        v.to_string()
                    };
                    write(s);
                });
                if !label.is_empty() {
                    b = b.header(label.clone());
                }
                b.into()
            } else {
                let mut b = text_box(value).on_text_changed(write);
                if !label.is_empty() {
                    b = b.header(label.clone());
                }
                if !placeholder.is_empty() {
                    b = b.placeholder_text(placeholder.clone());
                }
                if *multiline {
                    b = b.multiline().accepts_return(true).height(96.0);
                }
                b.into()
            }
        }

        PluginWidget::Checkbox { id, label, .. } => {
            let k = key(&page.id, id);
            let checked = model.answers.get(&k).map(|v| v == "true").unwrap_or(false);
            let write = answer_setter(model, set, k);
            check_box(checked)
                .content(label.clone())
                .on_checked(move |v: bool| write(if v { "true".into() } else { "false".into() }))
                .into()
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
            let selected = options
                .iter()
                .position(|o| o.value == current)
                .map(|i| i as i32)
                .unwrap_or(-1);
            let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();
            let values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
            let write = answer_setter(model, set, k);
            let on_pick = move |i: i32| {
                if let Some(v) = usize::try_from(i).ok().and_then(|i| values.get(i)) {
                    write(v.clone());
                }
            };
            match style {
                ChoiceStyle::Combo => {
                    let mut b = ComboBox::new(labels)
                        .selected_index(selected)
                        .on_selection_changed(on_pick);
                    if !label.is_empty() {
                        b = b.header(label.clone());
                    }
                    b.into()
                }
                ChoiceStyle::Radio => {
                    let mut b = RadioButtons::new(labels)
                        .selected_index(selected)
                        .on_selection_changed(on_pick);
                    if !label.is_empty() {
                        b = b.header(label.clone());
                    }
                    b.into()
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
                rows.push(
                    check_box(checked)
                        .content(opt.label.clone())
                        .on_checked(move |on: bool| {
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
                        .into(),
                );
            }
            vstack(rows).spacing(6.0).into()
        }

        PluginWidget::Progress { marquee } => {
            if *marquee {
                ProgressBar::indeterminate()
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .into()
            } else {
                ProgressBar::new(model.plugin_progress as f64)
                    .range(0.0, 100.0)
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .into()
            }
        }
    }
}

pub(super) fn required_warning() -> String {
    tr().get("install.field_required")
}

pub(super) const fn row_gap() -> f64 {
    ROW_GAP
}
