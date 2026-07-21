// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

pub mod choice_option;
pub mod choice_style;
pub mod feature_mode;
pub mod feature_select;
pub mod file_assoc;
pub mod file_entry;
pub mod install_dir_restriction;
pub mod install_info;
pub mod installer_payload;
pub mod launch_option;
pub mod manifest;
pub mod page_step;
pub mod patch_info;
pub mod payload_kind;
pub mod plugin_ctx;
pub mod plugin_entry;
pub mod plugin_page;
pub mod plugin_phase;
pub mod plugin_widget;
pub mod registry_entry;
pub mod registry_kind;
pub mod registry_value;
pub mod shortcut_entry;
pub mod signed_payload;

pub(crate) fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::choice_option::ChoiceOption;
    use super::choice_style::ChoiceStyle;
    use super::install_dir_restriction::InstallDirRestriction;
    use super::install_info::InstallInfo;
    use super::installer_payload::InstallerPayload;
    use super::launch_option::LaunchOption;
    use super::page_step::PageStep;
    use super::payload_kind::PayloadKind;
    use super::plugin_page::PluginPage;
    use super::plugin_phase::PluginPhase;
    use super::plugin_widget::PluginWidget;
    use super::registry_kind::RegistryKind;
    use super::registry_value::RegistryValue;
    use crate::model::feature_select::FeatureSelection;

    #[test]
    fn payload_parses_old_json_with_defaults() {
        // JSON predating publisher / force_reinstall / associations / license.
        let j = r#"{
            "kind":"Full","product":"P","from_version":null,"to_version":"1.0",
            "min_installer_version":"1.0.0","payload_blake3":"deadbeef",
            "created_at_unix":0,"manifest":{"version":"1.0","files":{}}
        }"#;
        let p: InstallerPayload = serde_json::from_str(j).unwrap();
        assert_eq!(p.publisher, "");
        assert_eq!(p.product_id, "");
        assert!(p.hintway_tenant_id.is_none());
        assert!(!p.force_reinstall);
        assert!(!p.purge_unknown_files);
        assert_eq!(p.install_dir_restriction, InstallDirRestriction::Enforce);
        assert!(!p.upgrade_minimal_ui);
        assert!(!p.show_uninstall_complete);
        assert_eq!(p.launch_option, LaunchOption::Checked);
        assert!(p.associations.is_empty());
        assert!(p.shortcuts.is_empty());
        assert!(p.license_text.is_none());
        assert_eq!(p.kind, PayloadKind::Full);
    }

    #[test]
    fn info_parses_old_json_with_defaults() {
        let j = r#"{
            "product":"P","version":"1.0","install_dir":"d",
            "installed_at_unix":0,"registry_key":"P","exe":"a.exe"
        }"#;
        let i: InstallInfo = serde_json::from_str(j).unwrap();
        assert_eq!(i.publisher, "");
        assert_eq!(i.product_id, "");
        assert!(i.hintway_tenant_id.is_none());
        assert!(i.associations.is_empty());
        assert!(i.shortcuts.is_empty());
    }

    #[test]
    fn payload_roundtrips() {
        let p = InstallerPayload {
            hintway_tenant_id: Some("tenant-123".into()),
            ..Default::default()
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: InstallerPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.publisher, "Pub");
        assert_eq!(back.product_id, "P_id");
        assert_eq!(back.hintway_tenant_id.as_deref(), Some("tenant-123"));
        assert_eq!(back.registry.len(), 1);
        assert_eq!(back.registry[0].kind, RegistryKind::Dword);
        assert_eq!(back.registry[0].value, RegistryValue::Int(42));
        assert_eq!(back.plugins.len(), 1);
        assert_eq!(back.plugins[0].phase, PluginPhase::PreInstall);
        assert!(back.plugins[0].required);
        assert!(back.plugins[0].ui);
        assert!(back.force_reinstall);
        assert!(back.purge_unknown_files);
        assert!(back.skip_license);
        assert!(!back.skip_path);
        assert_eq!(
            back.install_dir_restriction,
            InstallDirRestriction::DefaultDirOnly
        );
        assert_eq!(
            back.default_install_dir.as_deref(),
            Some(r"%LOCALAPPDATA%\Programs\P")
        );
        assert!(back.upgrade_minimal_ui);
        assert!(back.show_uninstall_complete);
        assert_eq!(back.launch_option, LaunchOption::Unchecked);
        assert_eq!(back.associations.len(), 1);
        assert_eq!(back.shortcuts.len(), 1);
        assert_eq!(back.shortcuts[0].dir, r"%DESKTOP%");
        assert_eq!(back.shortcuts[0].args, "--flag");
        assert_eq!(back.from_version.as_deref(), Some("1.0"));
    }

    #[test]
    fn plugin_widgets_round_trip() {
        let step = PageStep::Page {
            notice: String::new(),
            back: true,
            page: PluginPage {
                id: "main".into(),
                title: "Pick".into(),
                subtitle: String::new(),
                buttons: true,
                widgets: vec![
                    PluginWidget::Label {
                        id: String::new(),
                        text: "Hello".into(),
                    },
                    PluginWidget::Text {
                        id: "email".into(),
                        label: "Email".into(),
                        default: String::new(),
                        required: true,
                        placeholder: "you@x".into(),
                        password: false,
                        number: false,
                        multiline: false,
                    },
                    PluginWidget::Checkbox {
                        id: "news".into(),
                        label: "News".into(),
                        default: false,
                    },
                    PluginWidget::SingleChoice {
                        id: "country".into(),
                        label: "Country".into(),
                        options: vec![
                            ChoiceOption {
                                label: "France".into(),
                                value: "FR".into(),
                            },
                            ChoiceOption {
                                label: "DOM-TOM".into(),
                                value: "DOM".into(),
                            },
                        ],
                        style: ChoiceStyle::Combo,
                        default: "FR".into(),
                        required: true,
                    },
                    PluginWidget::MultiChoice {
                        id: "addons".into(),
                        label: "Add-ons".into(),
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
                        default: vec!["docs".into()],
                        required: false,
                    },
                ],
            },
        };
        let s = serde_json::to_string(&step).unwrap();
        let back: PageStep = serde_json::from_str(&s).unwrap();
        let PageStep::Page {
            page,
            back: allow_back,
            ..
        } = back
        else {
            panic!("expected page");
        };
        assert!(allow_back); // default
        assert_eq!(page.widgets.len(), 5);
        match &page.widgets[3] {
            PluginWidget::SingleChoice { style, .. } => assert_eq!(*style, ChoiceStyle::Combo),
            _ => panic!("expected single_choice"),
        }
        match &page.widgets[4] {
            PluginWidget::MultiChoice { default, .. } => assert_eq!(default, &["docs"]),
            _ => panic!("expected multi_choice"),
        }
    }

    /// A hand-written step (as a plugin's `installway_pages` would emit) parses,
    /// with `style`/`required`/`default`/`back` falling back correctly.
    #[test]
    fn page_step_parse() {
        let j = r#"{
          "step": "page",
          "page": {
            "id": "region",
            "title": "Select your country",
            "widgets": [
              { "kind": "single_choice", "id": "country",
                "options": [
                  { "label": "France", "value": "FR" },
                  { "label": "DOM-TOM", "value": "DOM" }
                ] },
              { "kind": "checkbox", "id": "accept", "label": "I agree" }
            ]
          }
        }"#;
        let PageStep::Page { page, back, notice } = serde_json::from_str(j).unwrap() else {
            panic!("expected page");
        };
        assert_eq!(page.id, "region");
        assert!(page.subtitle.is_empty());
        assert!(back); // missing -> true
        assert!(notice.is_empty());
        match &page.widgets[0] {
            PluginWidget::SingleChoice {
                style,
                required,
                default,
                options,
                ..
            } => {
                assert_eq!(*style, ChoiceStyle::Radio); // missing -> default
                assert!(*required); // missing -> true
                assert!(default.is_empty());
                assert_eq!(options.len(), 2);
            }
            _ => panic!("expected single_choice"),
        }
    }

    #[test]
    fn page_step_done_parses() {
        let s: PageStep = serde_json::from_str(r#"{ "step": "done" }"#).unwrap();
        assert!(matches!(s, PageStep::Done));
    }

    /// An unknown widget `kind` is rejected (the host must be able to render it).
    #[test]
    fn plugin_widget_unknown_kind_rejected() {
        let j = r#"{ "step":"page", "page":{ "id":"p","title":"t",
            "widgets":[{ "kind":"slider","id":"x" }] } }"#;
        assert!(serde_json::from_str::<PageStep>(j).is_err());
    }

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

    #[test]
    fn manifest_feature_mode_defaults_and_roundtrips() {
        use crate::model::feature_mode::FeatureMode;
        use crate::model::manifest::Manifest;
        // A manifest predating `feature_mode` deserializes to Sticky, so already
        // built payloads keep their original inherit-on-upgrade behavior.
        let m: Manifest = serde_json::from_str(r#"{"version":"1.0","files":{}}"#).unwrap();
        assert_eq!(m.feature_mode, FeatureMode::Sticky);
        // Sticky (the default) is omitted on serialize; Override is written and
        // round-trips as the lowercase string.
        assert!(!serde_json::to_string(&m).unwrap().contains("feature_mode"));
        let mut o = m;
        o.feature_mode = FeatureMode::Override;
        let s = serde_json::to_string(&o).unwrap();
        assert!(s.contains(r#""feature_mode":"override""#));
        assert_eq!(
            serde_json::from_str::<Manifest>(&s).unwrap().feature_mode,
            FeatureMode::Override
        );
    }
}
