// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Installer UIs over a shared set of Win32 helpers ([`helpers`]):
//! the full wizard ([`win32`]) and the compact auto-update window ([`minimal`]).

mod helpers;
pub mod minimal;
pub mod win32;

use anyhow::{Result, bail};
#[cfg(debug_assertions)]
use common::model::install_dir_restriction::InstallDirRestriction;
#[cfg(debug_assertions)]
use common::model::installer_payload::InstallerPayload;
#[cfg(debug_assertions)]
use common::model::launch_option::LaunchOption;
#[cfg(debug_assertions)]
use common::model::manifest::Manifest;
use common::model::page_step::PageStep;
#[cfg(debug_assertions)]
use common::model::payload_kind::PayloadKind;
use common::model::plugin_page::PluginInputs;
use common::model::plugin_page::PluginPage;
use common::model::plugin_widget::PluginWidget;

/// Fill one page's answers from each widget's declared default, for the
/// non-interactive paths (`--silent` / compact upgrade UI) where there is no form
/// to fill. A required text/choice with no usable default is an error (the user
/// must run the interactive installer). Keys are `"<page_id>.<widget_id>"`.
fn page_defaults(page: &PluginPage, plugin: &str) -> Result<PluginInputs> {
    let mut out = PluginInputs::new();
    for w in &page.widgets {
        let (id, value) = match w {
            PluginWidget::Label { .. } | PluginWidget::Progress { .. } => continue,
            PluginWidget::Text {
                id,
                default,
                required,
                ..
            } => {
                if *required && default.trim().is_empty() {
                    bail!(
                        "plugin '{plugin}' page '{}' field '{id}' is required but has no \
                         default; run the interactive installer",
                        page.id
                    );
                }
                (id, default.clone())
            }
            PluginWidget::Checkbox { id, default, .. } => {
                (id, if *default { "true" } else { "false" }.to_string())
            }
            PluginWidget::SingleChoice {
                id,
                options,
                default,
                required,
                ..
            } => {
                let value = if !default.is_empty() {
                    default.clone()
                } else if let Some(first) = options.first() {
                    first.value.clone()
                } else if *required {
                    bail!(
                        "plugin '{plugin}' page '{}' choice '{id}' is required but has no \
                         options; run the interactive installer",
                        page.id
                    );
                } else {
                    String::new()
                };
                (id, value)
            }
            PluginWidget::MultiChoice {
                id,
                default,
                required,
                ..
            } => {
                if *required && default.is_empty() {
                    bail!(
                        "plugin '{plugin}' page '{}' choice '{id}' is required but has no \
                         default; run the interactive installer",
                        page.id
                    );
                }
                (id, default.join(","))
            }
        };
        out.insert(format!("{}.{}", page.id, id), value);
    }
    Ok(out)
}

/// Headless (`--silent` / compact UI) plugin-page answers: drive each plugin's
/// step loop, filling every page from its defaults until `Done`. Empty when no
/// plugin contributes pages; errors when a required field has no default (or a
/// gate keeps rejecting the defaults).
pub fn headless_plugin_inputs(
    loaded: &crate::payload::LoadedPayload,
    install_dir: &std::path::Path,
) -> Result<common::plugin::InputsByPlugin> {
    let self_exe = std::env::current_exe()?;
    let Some(ui) =
        crate::extract::extract_ui_plugins(&loaded.payload, install_dir, &self_exe, loaded.zip())
    else {
        return Ok(common::plugin::InputsByPlugin::default());
    };

    const MAX_STEPS: usize = 100;
    let mut out = common::plugin::InputsByPlugin::new();
    for (entry, dll) in &ui.plugins {
        let mut answers = PluginInputs::new();
        let mut done = false;
        for _ in 0..MAX_STEPS {
            let answers_json = serde_json::to_string(&answers)?;
            match common::plugin::query_step(&ui.self_exe, &ui.base_ctx, entry, dll, &answers_json)?
            {
                PageStep::Done => {
                    done = true;
                    break;
                }
                PageStep::Page { page, .. } => {
                    if page.buttons {
                        answers.extend(page_defaults(&page, &entry.name)?);
                    }
                    // buttons:false = auto-run page; `up` runs in install pipeline, skip here.
                }
            }
        }
        if !done {
            bail!(
                "plugin '{}' did not finish in {MAX_STEPS} page steps headless (a validation \
                 gate may be rejecting the defaults); run the interactive installer",
                entry.name
            );
        }
        if !answers.is_empty() {
            out.insert(entry.name.clone(), answers);
        }
    }
    Ok(out)
}

/// Dev-only sample payload so `--preview` can render a view without a real,
/// signed installer payload. `view` may contain `patch` to preview the patch
/// subheader; otherwise a full install is described.
#[cfg(debug_assertions)]
pub(crate) fn sample_payload(view: &str) -> InstallerPayload {
    let is_patch = view.contains("patch");
    InstallerPayload {
        kind: if is_patch {
            PayloadKind::Patch
        } else {
            PayloadKind::Full
        },
        product: "Sample App".to_string(),
        product_id: "SampleApp".to_string(),
        publisher: "Acme Corp".to_string(),
        from_version: is_patch.then(|| "1.1.0".to_string()),
        to_version: "1.2.0".to_string(),
        min_installer_version: "1.0.0".to_string(),
        payload_blake3: String::new(),
        created_at_unix: 0,
        manifest: Manifest {
            version: "1.2.0".to_string(),
            exe: "bin/app.exe".to_string(),
            files: std::collections::HashMap::new(),
            deleted_files: Vec::new(),
            full_size: 12_345_678,
            total_patch_size: 0,
            features: Vec::new(),
            default_features: Vec::new(),
        },
        license_text: None,
        associations: Vec::new(),
        shortcuts: Vec::new(),
        force_reinstall: false,
        purge_unknown_files: false,
        skip_license: false,
        skip_path: false,
        install_dir_restriction: InstallDirRestriction::Enforce,
        default_install_dir: None,
        upgrade_minimal_ui: false,
        show_uninstall_complete: false,
        launch_option: LaunchOption::Checked,
        registry: Vec::new(),
        plugins: Vec::new(),
        active_features: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::page_defaults;
    use common::model::choice_option::ChoiceOption;
    use common::model::choice_style::ChoiceStyle;
    use common::model::plugin_page::PluginPage;
    use common::model::plugin_widget::PluginWidget;

    fn page(widgets: Vec<PluginWidget>) -> PluginPage {
        PluginPage {
            id: "p".into(),
            title: "T".into(),
            subtitle: String::new(),
            widgets,
            buttons: true,
        }
    }

    #[test]
    fn defaults_fill_each_widget() {
        let pg = page(vec![
            PluginWidget::Checkbox {
                id: "news".into(),
                label: "n".into(),
                default: true,
            },
            PluginWidget::SingleChoice {
                id: "country".into(),
                label: "c".into(),
                options: vec![
                    ChoiceOption {
                        label: "France".into(),
                        value: "FR".into(),
                    },
                    ChoiceOption {
                        label: "DOM".into(),
                        value: "DOM".into(),
                    },
                ],
                style: ChoiceStyle::Radio,
                default: String::new(),
                required: true,
            },
            PluginWidget::Text {
                id: "ref".into(),
                label: "r".into(),
                default: "x".into(),
                required: false,
                placeholder: String::new(),
                password: false,
                number: false,
                multiline: false,
            },
            PluginWidget::MultiChoice {
                id: "addons".into(),
                label: "a".into(),
                options: vec![
                    ChoiceOption {
                        label: "Docs".into(),
                        value: "docs".into(),
                    },
                    ChoiceOption {
                        label: "Samples".into(),
                        value: "samples".into(),
                    },
                ],
                default: vec!["docs".into(), "samples".into()],
                required: false,
            },
        ]);
        let m = page_defaults(&pg, "plug").unwrap();
        assert_eq!(m["p.news"], "true");
        assert_eq!(m["p.country"], "FR"); // empty default -> first option
        assert_eq!(m["p.ref"], "x");
        assert_eq!(m["p.addons"], "docs,samples"); // defaults joined
    }

    #[test]
    fn required_text_without_default_errors() {
        let pg = page(vec![PluginWidget::Text {
            id: "key".into(),
            label: "k".into(),
            default: String::new(),
            required: true,
            placeholder: String::new(),
            password: false,
            number: false,
            multiline: false,
        }]);
        assert!(page_defaults(&pg, "plug").is_err());
    }
}
