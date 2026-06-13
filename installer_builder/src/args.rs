// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use common::models::{PluginPhase, RegEntry, RegKind, RegValue};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "Build installer .exe with embedded payload")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Generate an Ed25519 signing keypair.
    Keygen(KeygenArgs),
    /// Build an installer .exe with an embedded payload.
    Pack(PackCli),
}

#[derive(clap::Args, Debug)]
pub struct KeygenArgs {
    /// Output directory for `priv.key` + `pub.key` (hex-encoded).
    #[arg(short, long)]
    pub out: PathBuf,
}

/// Raw `pack` CLI. Every value is optional here; a `--config <file.toml>` can
/// supply any of them, and a CLI value always wins over the file. Required
/// fields are checked once after merging, in [`PackArgs::resolve`].
#[derive(clap::Args, Debug)]
pub struct PackCli {
    /// TOML config file supplying any of the options below. CLI args override it.
    #[arg(long, value_name = "FILE.toml")]
    pub config: Option<PathBuf>,

    /// Product display name (ARP "DisplayName", version-info ProductName, UI
    /// text, shortcut label). The human-facing name.
    #[arg(short, long)]
    pub product: Option<String>,

    /// Registry-safe internal id, distinct from the display `--product`. Drives
    /// the HKCU Uninstall key, association ProgIDs, the per-user data folder and
    /// upgrade detection. Must match `^[A-Za-z][A-Za-z0-9._-]{0,49}$`; keep it
    /// stable across versions.
    #[arg(long)]
    pub product_id: Option<String>,

    /// Publisher / vendor name. Used for the per-user uninstall data folder
    /// %LOCALAPPDATA%\<publisher>\Uninstall\<product> and the Add/Remove
    /// Programs "Publisher" field.
    #[arg(long)]
    pub publisher: Option<String>,

    /// New version string (e.g. "1.0.1").
    #[arg(long)]
    pub to_version: Option<String>,

    /// Source dir containing the new version files.
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Previous version dir (for patch mode).
    #[arg(long)]
    pub from_dir: Option<PathBuf>,

    /// Previous version string (for patch mode).
    #[arg(long)]
    pub from_version: Option<String>,

    /// Main executable path relative to product root (e.g. "game.exe").
    #[arg(short, long)]
    pub exe: Option<String>,

    /// Optional path to a UTF-8 license text file shown on the License page.
    #[arg(long)]
    pub license: Option<PathBuf>,

    /// File association, format `.ext:Description`. Repeatable. Replaces (not
    /// merges with) any `assoc` list from the config file when given.
    #[arg(long = "assoc", value_name = ".ext:Description")]
    pub assoc: Vec<String>,

    /// Minimum installer binary version allowed to install this payload.
    #[arg(long)]
    pub min_installer_version: Option<String>,

    /// Dev: reinstall from scratch (skip from-version check, rewrite all files,
    /// remove orphans).
    #[arg(long)]
    pub force_reinstall: bool,

    /// Remove unknown/leftover files (not in this build) on a Full install, so
    /// an upgrade or reinstall from a full version leaves a clean directory.
    /// Known files are still hash-skipped. Ignored for patch payloads.
    #[arg(long)]
    pub purge_unknown_files: bool,

    /// Hide the License page in the interactive installer.
    #[arg(long)]
    pub skip_license: bool,

    /// Hide the Choose-location page; install straight to the default path.
    #[arg(long)]
    pub skip_path: bool,

    /// Use the compact minimal UI for upgrades (a run over an already-installed
    /// copy). The first install still uses the full wizard. Optional.
    #[arg(long)]
    pub upgrade_minimal_ui: bool,

    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default.
    #[arg(long)]
    pub show_uninstall_complete: bool,

    /// Default install dir the UI proposes (per-app). May contain `%VAR%` env
    /// tokens, e.g. `%LOCALAPPDATA%\Programs\MyApp` or `C:\Games\MyApp`.
    #[arg(long, value_name = "DIR")]
    pub default_install_dir: Option<String>,

    /// Path to the Ed25519 private key file.
    #[arg(long)]
    pub priv_key: Option<PathBuf>,

    /// Path to the Ed25519 public key file. Required only in toolchain mode.
    #[arg(long)]
    pub pub_key: Option<PathBuf>,

    /// Prebuilt installer stub (`installer.exe`) with the key already compiled
    /// in. Requires `--uninstaller`; no Rust toolchain needed.
    #[arg(long)]
    pub installer_stub: Option<PathBuf>,

    /// Prebuilt uninstaller (`uninstall.exe`), paired with `--installer-stub`.
    #[arg(long)]
    pub uninstaller: Option<PathBuf>,

    /// Output installer .exe path.
    #[arg(short, long)]
    pub out: Option<PathBuf>,

    /// Skip rebuilding the installer crate if the stub already exists.
    #[arg(long)]
    pub reuse_stub: bool,
}

/// One `[[registry]]` table from the config file. Converted + validated into a
/// [`RegEntry`] by [`build_registry`]. `value` is left as a raw `toml::Value`
/// so a string / integer / array can all be accepted per `type`.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RegFileEntry {
    pub hive: String,
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: toml::Value,
}

/// One `[[plugin]]` table from the config file. Converted + validated into a
/// [`ResolvedPlugin`] by [`build_plugins`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PluginFileEntry {
    pub name: String,
    pub dll: PathBuf,
    pub phase: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// A validated plugin: its source DLL path + parsed phase.
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub name: String,
    pub src: PathBuf,
    pub phase: PluginPhase,
    pub required: bool,
}

/// `pack` options as read from a TOML file. Flat keys matching the CLI long
/// names (snake_case). Unknown keys are rejected to catch typos.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub product: Option<String>,
    pub product_id: Option<String>,
    pub publisher: Option<String>,
    pub to_version: Option<String>,
    pub input: Option<PathBuf>,
    pub from_dir: Option<PathBuf>,
    pub from_version: Option<String>,
    pub exe: Option<String>,
    pub license: Option<PathBuf>,
    #[serde(default)]
    pub assoc: Vec<String>,
    pub min_installer_version: Option<String>,
    #[serde(default)]
    pub force_reinstall: bool,
    #[serde(default)]
    pub purge_unknown_files: bool,
    #[serde(default)]
    pub skip_license: bool,
    #[serde(default)]
    pub skip_path: bool,
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    #[serde(default)]
    pub show_uninstall_complete: bool,
    pub default_install_dir: Option<String>,
    pub priv_key: Option<PathBuf>,
    pub pub_key: Option<PathBuf>,
    pub installer_stub: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub out: Option<PathBuf>,
    #[serde(default)]
    pub reuse_stub: bool,
    /// Free-form registry entries (config-file only).
    #[serde(default)]
    pub registry: Vec<RegFileEntry>,
    /// Native DLL plugins (config-file only). `[[plugin]]` tables.
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<PluginFileEntry>,
}

/// Fully resolved `pack` options consumed by `pack::run`. CLI > TOML > default.
#[derive(Debug, Clone)]
pub struct PackArgs {
    pub product: String,
    pub product_id: String,
    pub publisher: String,
    pub to_version: String,
    pub input: PathBuf,
    pub from_dir: Option<PathBuf>,
    pub from_version: Option<String>,
    pub exe: String,
    pub license: Option<PathBuf>,
    pub assoc: Vec<String>,
    pub min_installer_version: String,
    pub force_reinstall: bool,
    pub purge_unknown_files: bool,
    pub skip_license: bool,
    pub skip_path: bool,
    pub upgrade_minimal_ui: bool,
    pub show_uninstall_complete: bool,
    pub default_install_dir: Option<String>,
    pub priv_key: PathBuf,
    pub pub_key: Option<PathBuf>,
    pub installer_stub: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub out: PathBuf,
    pub reuse_stub: bool,
    pub registry: Vec<RegEntry>,
    pub plugins: Vec<ResolvedPlugin>,
}

impl PackArgs {
    /// Merge the CLI over an optional TOML config and validate required fields.
    pub fn resolve(cli: PackCli) -> Result<PackArgs> {
        let file: PackFile = match &cli.config {
            Some(p) => {
                let text = std::fs::read_to_string(p)
                    .with_context(|| format!("read config {}", p.display()))?;
                toml::from_str(&text).with_context(|| format!("parse config {}", p.display()))?
            }
            None => PackFile::default(),
        };

        // A missing required value (neither CLI nor config) is a clear error.
        let req = |name: &str| format!("missing '{name}' (pass --{name} or set it in --config)");

        Ok(PackArgs {
            product: cli
                .product
                .or(file.product)
                .with_context(|| req("product"))?,
            product_id: cli
                .product_id
                .or(file.product_id)
                .with_context(|| req("product-id"))?,
            publisher: cli
                .publisher
                .or(file.publisher)
                .with_context(|| req("publisher"))?,
            to_version: cli
                .to_version
                .or(file.to_version)
                .with_context(|| req("to-version"))?,
            input: cli.input.or(file.input).with_context(|| req("input"))?,
            exe: cli.exe.or(file.exe).with_context(|| req("exe"))?,
            priv_key: cli
                .priv_key
                .or(file.priv_key)
                .with_context(|| req("priv-key"))?,
            out: cli.out.or(file.out).with_context(|| req("out"))?,

            from_dir: cli.from_dir.or(file.from_dir),
            from_version: cli.from_version.or(file.from_version),
            license: cli.license.or(file.license),
            default_install_dir: cli.default_install_dir.or(file.default_install_dir),
            pub_key: cli.pub_key.or(file.pub_key),
            installer_stub: cli.installer_stub.or(file.installer_stub),
            uninstaller: cli.uninstaller.or(file.uninstaller),

            // CLI list replaces the file list when present.
            assoc: if cli.assoc.is_empty() {
                file.assoc
            } else {
                cli.assoc
            },
            min_installer_version: cli
                .min_installer_version
                .or(file.min_installer_version)
                .unwrap_or_else(|| "1.0.0".to_string()),
            // Boolean flags: either source can turn them on.
            force_reinstall: cli.force_reinstall || file.force_reinstall,
            purge_unknown_files: cli.purge_unknown_files || file.purge_unknown_files,
            skip_license: cli.skip_license || file.skip_license,
            skip_path: cli.skip_path || file.skip_path,
            upgrade_minimal_ui: cli.upgrade_minimal_ui || file.upgrade_minimal_ui,
            show_uninstall_complete: cli.show_uninstall_complete || file.show_uninstall_complete,
            reuse_stub: cli.reuse_stub || file.reuse_stub,
            registry: build_registry(file.registry)?,
            plugins: build_plugins(file.plugins)?,
        })
    }
}

/// Convert + validate `[[plugin]]` entries. Names must be safe filename
/// components and unique; phase is `pre-install` or `post-install`.
fn build_plugins(raw: Vec<PluginFileEntry>) -> Result<Vec<ResolvedPlugin>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut seen = std::collections::HashSet::new();
    for (i, p) in raw.into_iter().enumerate() {
        let n = i + 1;
        let name = p.name.trim().to_string();
        if name.is_empty() {
            bail!("plugin #{n}: empty name");
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("plugin #{n} ('{name}'): name must be ASCII letters, digits, '-' or '_'");
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            bail!("plugin #{n}: duplicate name '{name}'");
        }
        let phase = match p.phase.to_ascii_lowercase().replace('_', "-").as_str() {
            "pre-install" => PluginPhase::PreInstall,
            "post-install" => PluginPhase::PostInstall,
            other => bail!(
                "plugin #{n} ('{name}'): unknown phase '{other}' (pre-install | post-install)"
            ),
        };
        out.push(ResolvedPlugin {
            name,
            src: p.dll,
            phase,
            required: p.required,
        });
    }
    Ok(out)
}

/// Convert + validate `[[registry]]` entries. HKCU only; type/value must agree;
/// key non-empty and not starting with `\`.
fn build_registry(raw: Vec<RegFileEntry>) -> Result<Vec<RegEntry>> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, e) in raw.into_iter().enumerate() {
        let n = i + 1;
        if !e.hive.eq_ignore_ascii_case("HKCU") {
            bail!("registry #{n}: hive '{}' unsupported (HKCU only)", e.hive);
        }
        let key = e.key.trim().to_string();
        if key.is_empty() {
            bail!("registry #{n}: empty key");
        }
        if key.starts_with('\\') {
            bail!("registry #{n}: key must not start with '\\'");
        }
        let kind = match e.kind.to_ascii_lowercase().as_str() {
            "sz" => RegKind::Sz,
            "expand_sz" => RegKind::ExpandSz,
            "dword" => RegKind::Dword,
            "qword" => RegKind::Qword,
            "multi_sz" => RegKind::MultiSz,
            "binary" => RegKind::Binary,
            other => bail!("registry #{n}: unknown type '{other}'"),
        };
        let value = convert_reg_value(kind, &e.value).ok_or_else(|| {
            anyhow::anyhow!(
                "registry #{n} ('{key}'): value does not match type '{}'",
                e.kind
            )
        })?;
        out.push(RegEntry {
            hive: "HKCU".to_string(),
            key,
            name: e.name,
            kind,
            value,
        });
    }
    Ok(out)
}

fn convert_reg_value(kind: RegKind, v: &toml::Value) -> Option<RegValue> {
    match kind {
        RegKind::Sz | RegKind::ExpandSz => Some(RegValue::Text(v.as_str()?.to_string())),
        RegKind::Binary => {
            let s = v.as_str()?;
            (s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| RegValue::Text(s.to_string()))
        }
        RegKind::Dword => {
            let n = v.as_integer()?;
            (0..=u32::MAX as i64)
                .contains(&n)
                .then(|| RegValue::Int(n as u64))
        }
        RegKind::Qword => {
            let n = v.as_integer()?;
            (n >= 0).then(|| RegValue::Int(n as u64))
        }
        RegKind::MultiSz => {
            let arr = v.as_array()?;
            arr.iter()
                .map(|it| it.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .map(RegValue::List)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> PackCli {
        PackCli {
            config: None,
            product: None,
            product_id: None,
            publisher: None,
            to_version: None,
            input: None,
            from_dir: None,
            from_version: None,
            exe: None,
            license: None,
            assoc: Vec::new(),
            min_installer_version: None,
            force_reinstall: false,
            purge_unknown_files: false,
            skip_license: false,
            skip_path: false,
            upgrade_minimal_ui: false,
            show_uninstall_complete: false,
            default_install_dir: None,
            priv_key: None,
            pub_key: None,
            installer_stub: None,
            uninstaller: None,
            out: None,
            reuse_stub: false,
        }
    }

    const SAMPLE: &str = "\
product = 'myapp'
product_id = 'myapp'
publisher = 'Acme'
to_version = '1.0'
input = 'build/myapp'
exe = 'myapp.exe'
priv_key = 'keys/priv.key'
out = 'dist/setup.exe'
assoc = ['.myx:Doc']
force_reinstall = true
";

    fn write_cfg(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pack.toml");
        std::fs::write(&p, body).unwrap();
        (dir, p)
    }

    /// TOML fills everything; resolve succeeds with file values + default min version.
    #[test]
    fn resolves_from_file() {
        let (_dir, cfg) = write_cfg(SAMPLE);
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        let r = PackArgs::resolve(cli).unwrap();
        assert_eq!(r.product, "myapp");
        assert_eq!(r.product_id, "myapp");
        assert_eq!(r.publisher, "Acme");
        assert_eq!(r.assoc, vec![".myx:Doc".to_string()]);
        assert_eq!(r.min_installer_version, "1.0.0"); // default, absent in file
        assert!(r.force_reinstall); // from file
    }

    /// CLI value wins over the file value.
    #[test]
    fn cli_overrides_file() {
        let (_dir, cfg) = write_cfg(SAMPLE);
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        cli.product = Some("override".to_string());
        cli.assoc = vec![".zzz:Other".to_string()];
        let r = PackArgs::resolve(cli).unwrap();
        assert_eq!(r.product, "override"); // CLI over file
        assert_eq!(r.assoc, vec![".zzz:Other".to_string()]); // CLI list replaces file list
        assert_eq!(r.publisher, "Acme"); // untouched, from file
    }

    /// Missing a required field (no CLI, no file) errors naming the field.
    #[test]
    fn missing_required_errors() {
        let err = PackArgs::resolve(empty_cli()).unwrap_err().to_string();
        assert!(err.contains("product"), "got: {err}");
    }

    /// Unknown keys in the config are rejected (typo guard).
    #[test]
    fn unknown_key_rejected() {
        let (_dir, cfg) = write_cfg("produdct = 'oops'\n");
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        assert!(PackArgs::resolve(cli).is_err());
    }

    fn resolve_with(extra: &str) -> Result<PackArgs> {
        let (_dir, cfg) = write_cfg(&format!("{SAMPLE}{extra}"));
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        let r = PackArgs::resolve(cli);
        // keep tempdir alive until resolve has read the file
        drop(_dir);
        r
    }

    #[test]
    fn registry_parsed_and_typed() {
        let r = resolve_with(
            "\n[[registry]]\nhive='HKCU'\nkey='Software\\\\X'\nname='Build'\ntype='dword'\nvalue=7\n\
             [[registry]]\nhive='HKCU'\nkey='%APP_KEY%'\ntype='multi_sz'\nvalue=['a','b']\n",
        )
        .unwrap();
        assert_eq!(r.registry.len(), 2);
        assert_eq!(r.registry[0].kind, RegKind::Dword);
        assert_eq!(r.registry[0].value, RegValue::Int(7));
        assert_eq!(r.registry[1].kind, RegKind::MultiSz);
        assert_eq!(
            r.registry[1].value,
            RegValue::List(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn registry_rejects_bad_inputs() {
        // unknown type
        assert!(
            resolve_with("\n[[registry]]\nhive='HKCU'\nkey='K'\ntype='nope'\nvalue=1\n").is_err()
        );
        // HKLM not allowed
        assert!(
            resolve_with("\n[[registry]]\nhive='HKLM'\nkey='K'\ntype='sz'\nvalue='x'\n").is_err()
        );
        // dword given a string
        assert!(
            resolve_with("\n[[registry]]\nhive='HKCU'\nkey='K'\ntype='dword'\nvalue='x'\n")
                .is_err()
        );
        // bad hex for binary
        assert!(
            resolve_with("\n[[registry]]\nhive='HKCU'\nkey='K'\ntype='binary'\nvalue='XY'\n")
                .is_err()
        );
    }

    #[test]
    fn plugins_parsed_and_validated() {
        let r = resolve_with(
            "\n[[plugin]]\nname='do-x'\ndll='plugins/x.dll'\nphase='pre-install'\n\
             [[plugin]]\nname='do_y'\ndll='plugins/y.dll'\nphase='post_install'\nrequired=false\n",
        )
        .unwrap();
        assert_eq!(r.plugins.len(), 2);
        assert_eq!(r.plugins[0].name, "do-x");
        assert_eq!(r.plugins[0].phase, PluginPhase::PreInstall);
        assert!(r.plugins[0].required);
        assert_eq!(r.plugins[1].phase, PluginPhase::PostInstall);
        assert!(!r.plugins[1].required);
    }

    #[test]
    fn plugins_reject_bad() {
        // unknown phase
        assert!(resolve_with("\n[[plugin]]\nname='a'\ndll='a.dll'\nphase='nope'\n").is_err());
        // illegal name
        assert!(
            resolve_with("\n[[plugin]]\nname='a b'\ndll='a.dll'\nphase='pre-install'\n").is_err()
        );
        // duplicate (case-insensitive)
        assert!(
            resolve_with(
                "\n[[plugin]]\nname='a'\ndll='a.dll'\nphase='pre-install'\n\
                 [[plugin]]\nname='A'\ndll='b.dll'\nphase='pre-install'\n"
            )
            .is_err()
        );
    }
}
