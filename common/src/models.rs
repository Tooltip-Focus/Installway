// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub exe: String,
    pub files: HashMap<String, FileEntry>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    #[serde(default)]
    pub full_size: u64,
    #[serde(default)]
    pub total_patch_size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    pub hash: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<PatchInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchInfo {
    pub file: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    Full,
    Patch,
}

/// Whether the *interactive* installer lets a fresh install target a non-empty
/// folder. A first install normally blocks a non-empty destination (a guard
/// against picking the wrong folder). These relax it for apps that must install
/// over an existing layout - e.g. replacing a legacy InstallShield or MSI
/// install in its own directory, where a (pre-install) plugin validates the old
/// install and a `purge_unknown_files` + uninstall `down` plugin clean it up.
/// Only affects the Choose-location page; silent/headless installs never run
/// this guard.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InstallDirRestriction {
    /// Default: block a fresh install into a non-empty folder.
    #[default]
    Enforce,
    /// Allow a non-empty folder only when it is the build-time default install
    /// dir (the known legacy location). Any other non-empty folder is blocked.
    DefaultDirOnly,
    /// Allow any non-empty folder.
    Bypass,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallerPayload {
    pub kind: PayloadKind,
    /// Human-facing display name: ARP `DisplayName`, version-info ProductName,
    /// installer/uninstaller UI text, and the shortcut label.
    pub product: String,
    /// Registry-safe internal identifier, distinct from the display `product`.
    /// Drives the HKCU Uninstall key, association ProgIDs, the per-user data
    /// folder (`%LOCALAPPDATA%\<publisher>\Uninstall\<product_id>`) and upgrade
    /// detection. Validated at build time. `#[serde(default)]` so payloads
    /// predating the split still parse (empty → fall back to a sanitized
    /// `product`).
    #[serde(default)]
    pub product_id: String,
    /// Publisher / vendor name. Used for the per-user uninstall data folder
    /// and the Add/Remove Programs "Publisher" field. Mandatory at build time.
    #[serde(default)]
    pub publisher: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub min_installer_version: String,
    pub payload_blake3: String,
    pub created_at_unix: i64,
    pub manifest: Manifest,
    /// Optional EULA text shown on the License page of the installer UI.
    /// `None` (or missing field on older payloads) falls back to a built-in placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
    /// File-type associations to register under `HKCU\Software\Classes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts (`.lnk`) to create at install; nothing is created unless
    /// declared here. `dir`/`target`/`args` are templates expanded at install
    /// time (see the shortcut docs for the token list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Dev flag: ignore the installed version and reinstall from scratch
    /// (skip patch from-version check, rewrite all files, remove orphans).
    #[serde(default)]
    pub force_reinstall: bool,
    /// Remove existing files not in this build's manifest (unknown / leftover
    /// files) during a Full install. Opt-in at build time so an upgrade or
    /// reinstall from a full version leaves a clean directory. Ignored for
    /// patch payloads. Unlike [`force_reinstall`], known files are still
    /// hash-skipped (not rewritten) and the version check is unaffected.
    #[serde(default)]
    pub purge_unknown_files: bool,
    /// Hide the License page in the interactive UI.
    #[serde(default)]
    pub skip_license: bool,
    /// Hide the Choose-location page; install straight to the default path.
    #[serde(default)]
    pub skip_path: bool,
    /// Whether a fresh interactive install may target a non-empty folder.
    /// Defaults to [`InstallDirRestriction::Enforce`]; see that type's docs.
    #[serde(default)]
    pub install_dir_restriction: InstallDirRestriction,
    /// Default install directory the UI proposes, set per-app at build time.
    /// May contain `%VAR%` env tokens (e.g. `%LOCALAPPDATA%\Programs\MyApp`).
    /// `None` falls back to `%LOCALAPPDATA%\Programs\<product>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_install_dir: Option<String>,
    /// When set, an *upgrade* (a run over an already-installed copy) uses the
    /// compact minimal UI instead of the full wizard. The first install always
    /// uses the full wizard. Decided by this (the new installer's) payload.
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    /// Free-form registry entries (HKCU) written at install and removed at
    /// uninstall. Key/value strings are templates expanded at install time
    /// (see the registry docs for the token list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegEntry>,
    /// Native DLL plugins bundled in the payload zip, run at install.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default; enable per-app at build time.
    #[serde(default)]
    pub show_uninstall_complete: bool,
}

/// Registry value type for a [`RegEntry`].
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegKind {
    Sz,
    ExpandSz,
    Dword,
    Qword,
    MultiSz,
    Binary,
}

/// A registry value's data. The variant is paired with a [`RegKind`]:
/// `Text` for sz/expand_sz (and the hex string of binary), `Int` for
/// dword/qword, `List` for multi_sz.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum RegValue {
    Text(String),
    Int(u64),
    List(Vec<String>),
}

/// One free-form registry entry. In the payload the strings are templates; in
/// `InstallInfo` they are the resolved values actually written (so the
/// uninstaller can match + remove exactly what it wrote).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegEntry {
    /// Hive — `"HKCU"` only (the installer never elevates).
    pub hive: String,
    /// Subkey path under the hive, e.g. `Software\Acme\App`.
    pub key: String,
    /// Value name; empty = the key's `(Default)` value.
    #[serde(default)]
    pub name: String,
    pub kind: RegKind,
    pub value: RegValue,
}

/// One shortcut (`.lnk`) the installer creates.
///
/// In the payload the strings are templates; in `InstallInfo` they are the
/// resolved values actually written (absolute `dir`/`target`), so the
/// uninstaller removes exactly the files it created and an upgrade can
/// reconcile a changed list by resolved `.lnk` path.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShortcutEntry {
    /// Directory the `.lnk` is placed in. Tokens: `%DESKTOP%`, `%START_MENU%`
    /// (per-user Programs), `%INSTALL_DIR%`, plus `%VAR%` env vars.
    pub dir: String,
    /// Shortcut file name, without the `.lnk` extension (also the label).
    pub name: String,
    /// Shortcut target. A relative path resolves against the install dir (the
    /// product exe); same tokens as `dir` are expanded.
    pub target: String,
    /// Free-form command-line arguments appended to the shortcut. Empty = none.
    #[serde(default)]
    pub args: String,
}

/// One file-type association: extension + a human description.
/// The shell `open` verb is wired to the product's main exe with `"%1"`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileAssoc {
    /// Extension including the leading dot, e.g. ".myx".
    pub ext: String,
    /// Friendly type description shown in Explorer, e.g. "My App Document".
    pub description: String,
}

/// What gets embedded in the installer .exe as RCDATA id=2.
///
/// `payload_json` is the exact UTF-8 byte sequence the signature was computed over.
/// The verifier verifies the signature against those bytes, *then* parses
/// `InstallerPayload` from them. This avoids any serializer-determinism trap.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SignedPayload {
    pub payload_json: String,
    pub signature_hex: String,
}

/// Persisted to `<install_dir>/installer_info.json` by the installer.
/// Read by the uninstaller (and any tooling) to locate registry entries
/// and walk the manifest for cleanup.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallInfo {
    pub product: String,
    /// Registry-safe internal id (see `InstallerPayload::product_id`). Empty on
    /// records written before the split — readers fall back to `registry_key` /
    /// a sanitized `product`.
    #[serde(default)]
    pub product_id: String,
    #[serde(default)]
    pub publisher: String,
    pub version: String,
    pub install_dir: String,
    pub installed_at_unix: i64,
    /// HKCU subkey under `Software\Microsoft\Windows\CurrentVersion\Uninstall`.
    pub registry_key: String,
    /// Optional path (relative to install_dir) of the product's main exe.
    pub exe: String,
    /// File associations registered at install time - the uninstaller removes
    /// exactly these.
    #[serde(default)]
    pub associations: Vec<FileAssoc>,
    /// Shortcuts created at install (resolved absolute paths) - the uninstaller
    /// removes exactly these, and an upgrade reconciles a changed set.
    #[serde(default)]
    pub shortcuts: Vec<ShortcutEntry>,
    /// Resolved registry entries written at install - the uninstaller removes
    /// exactly these (anti-stomp by value).
    #[serde(default)]
    pub registry: Vec<RegEntry>,
    /// Plugins recorded at install - the uninstaller runs their `down`.
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default; set per-app at build time.
    #[serde(default)]
    pub show_uninstall_complete: bool,
}

fn default_true() -> bool {
    true
}

/// When a [`PluginEntry`] runs at install.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginPhase {
    /// Before any file is staged/committed. A required failure aborts cleanly.
    #[default]
    PreInstall,
    /// After the install is finalized (files in place, product registered).
    PostInstall,
}

/// A native DLL plugin (migration-style `up`/`down`). The DLL lives in the
/// signed payload zip at `file` and is copied to the per-user data dir for the
/// uninstall `down`. `blake3` is verified before the DLL is loaded.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginEntry {
    pub name: String,
    /// In-zip / data-dir-relative path, e.g. `plugins/<name>.dll`.
    pub file: String,
    pub blake3: String,
    pub phase: PluginPhase,
    /// A required plugin's `up` failure fails the install. Default `true`.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Opt-in: this plugin contributes custom wizard pages. When set, the host
    /// queries its `installway_pages` before showing the wizard.
    #[serde(default)]
    pub ui: bool,
}

// ---- Plugin custom wizard pages -----------------------------------------

/// One step in a `ui = true` plugin's wizard flow. `installway_pages` is a pure
/// step function: the host calls it with the answers so far (`ctx.inputs_json`)
/// and the plugin returns the next [`PageStep`]. The installer renders pages with
/// its own Win32 controls — the plugin never draws UI.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum PageStep {
    /// Show this page next.
    Page {
        page: PluginPage,
        /// Optional banner shown above the page (e.g. a validation error so the
        /// plugin can re-ask).
        #[serde(default)]
        notice: String,
        /// Allow the Back button on this page (when there's somewhere to go back
        /// to). The plugin can set `false` to pin the user here.
        #[serde(default = "default_true")]
        back: bool,
    },
    /// No more pages — proceed to install.
    Done,
}

/// One contributed wizard page.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginPage {
    /// Unique within the contributing plugin; namespaces collected values
    /// (`"<page_id>.<widget_id>"`).
    pub id: String,
    /// Banner title (final text — the plugin localizes it, the host renders it
    /// verbatim).
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub widgets: Vec<PluginWidget>,
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
        /// Option `value`s checked initially.
        #[serde(default)]
        default: Vec<String>,
        /// Require at least one selection.
        #[serde(default)]
        required: bool,
    },
}

/// One choice in a [`PluginWidget::SingleChoice`].
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChoiceOption {
    pub label: String,
    pub value: String,
}

/// How a [`PluginWidget::SingleChoice`] is drawn.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceStyle {
    #[default]
    Radio,
    Combo,
}

/// Collected page answers, keyed `"<page_id>.<widget_id>"`. `BTreeMap` keeps a
/// deterministic order (stable logs/tests). Serialized into `PluginCtx.inputs_json`.
/// `MultiChoice` answers join selected values with `,`; option `value` strings must
/// not themselves contain `,` (no escaping is applied).
pub type PluginInputs = BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

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
