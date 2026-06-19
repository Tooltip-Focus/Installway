// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

pub mod choice_option;
pub mod choice_style;
pub mod file_assoc;
pub mod file_entry;
pub mod install_dir_restriction;
pub mod install_info;
pub mod installer_payload;
pub mod manifest;
pub mod page_step;
pub mod patch_info;
pub mod payload_kind;
pub mod plugin_entry;
pub mod plugin_page;
pub mod plugin_phase;
pub mod plugin_widget;
pub mod reg_entry;
pub mod reg_kind;
pub mod reg_value;
pub mod shortcut_entry;
pub mod signed_payload;

#[cfg(test)]
mod tests {
    use super::choice_option::ChoiceOption;
    use super::choice_style::ChoiceStyle;
    use super::file_assoc::FileAssoc;
    use super::install_dir_restriction::InstallDirRestriction;
    use super::install_info::InstallInfo;
    use super::installer_payload::InstallerPayload;
    use super::manifest::Manifest;
    use super::page_step::PageStep;
    use super::payload_kind::PayloadKind;
    use super::plugin_entry::PluginEntry;
    use super::plugin_page::PluginPage;
    use super::plugin_phase::PluginPhase;
    use super::plugin_widget::PluginWidget;
    use super::reg_entry::RegEntry;
    use super::reg_kind::RegKind;
    use super::reg_value::RegValue;
    use super::shortcut_entry::ShortcutEntry;

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
        assert!(!p.force_reinstall);
        assert!(!p.purge_unknown_files);
        assert_eq!(p.install_dir_restriction, InstallDirRestriction::Enforce);
        assert!(!p.upgrade_minimal_ui);
        assert!(!p.show_uninstall_complete);
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
        assert!(i.associations.is_empty());
        assert!(i.shortcuts.is_empty());
    }

    #[test]
    fn payload_roundtrips() {
        let p = InstallerPayload {
            kind: PayloadKind::Patch,
            product: "P".into(),
            product_id: "P_id".into(),
            publisher: "Pub".into(),
            from_version: Some("1.0".into()),
            to_version: "1.1".into(),
            min_installer_version: "1.0.0".into(),
            payload_blake3: "abc".into(),
            created_at_unix: 123,
            manifest: Manifest {
                version: "1.1".into(),
                exe: "a.exe".into(),
                files: Default::default(),
                deleted_files: vec![],
                full_size: 0,
                total_patch_size: 0,
            },
            license_text: None,
            associations: vec![FileAssoc {
                ext: ".x".into(),
                description: "X".into(),
            }],
            shortcuts: vec![ShortcutEntry {
                dir: r"%DESKTOP%".into(),
                name: "P".into(),
                target: "a.exe".into(),
                args: "--flag".into(),
            }],
            force_reinstall: true,
            purge_unknown_files: true,
            skip_license: true,
            skip_path: false,
            install_dir_restriction: InstallDirRestriction::DefaultDirOnly,
            default_install_dir: Some(r"%LOCALAPPDATA%\Programs\P".into()),
            upgrade_minimal_ui: true,
            registry: vec![RegEntry {
                hive: "HKCU".into(),
                key: r"Software\Acme\App".into(),
                name: "Build".into(),
                kind: RegKind::Dword,
                value: RegValue::Int(42),
            }],
            plugins: vec![PluginEntry {
                name: "p1".into(),
                file: "plugins/p1.dll".into(),
                blake3: "abc".into(),
                phase: PluginPhase::PreInstall,
                required: true,
                ui: true,
            }],
            show_uninstall_complete: true,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: InstallerPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.publisher, "Pub");
        assert_eq!(back.product_id, "P_id");
        assert_eq!(back.registry.len(), 1);
        assert_eq!(back.registry[0].kind, RegKind::Dword);
        assert_eq!(back.registry[0].value, RegValue::Int(42));
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
}
