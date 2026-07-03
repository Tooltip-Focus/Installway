// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use common::model::file_assoc::FileAssoc;
use common::model::install_dir_restriction::InstallDirRestriction;
use common::model::launch_option::LaunchOption;
use common::model::plugin_phase::PluginPhase;
use common::model::reg_entry::RegEntry;
use common::model::reg_kind::RegKind;
use common::model::reg_value::RegValue;
use common::model::shortcut_entry::ShortcutEntry;
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
    Pack(Box<PackCli>),
}

#[derive(clap::Args, Debug)]
pub struct KeygenArgs {
    /// Output directory for `priv.key` + `pub.key`.
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

    /// Product display name. The human-facing name.
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

    /// Main executable path relative to product root (e.g. "app.exe").
    #[arg(short, long)]
    pub exe: Option<String>,

    /// Optional path to a  license text file shown on the License page.
    #[arg(long)]
    pub license: Option<PathBuf>,

    /// Optional PNG painted across the installer's header strip (replacing the
    /// default flat gray card).
    #[arg(long, value_name = "FILE.png")]
    pub banner: Option<PathBuf>,

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

    /// Non-empty-folder guard on the Choose-location page: `enforce` (block a
    /// fresh install into any non-empty folder), `default-dir-only` (allow only
    /// the build-time default dir to be non-empty) or `bypass` (allow any).
    /// Default `enforce`.
    #[arg(long, value_name = "enforce|default-dir-only|bypass")]
    pub install_dir_restriction: Option<String>,

    /// Use the compact minimal UI for upgrades. Optional.
    #[arg(long)]
    pub upgrade_minimal_ui: bool,

    /// Show the "uninstall complete" confirmation message box at the end of an
    /// interactive uninstall. Off by default.
    #[arg(long)]
    pub show_uninstall_complete: bool,

    /// Behaviour of the "launch now" checkbox on the installer's final page:
    /// `checked` (visible + ticked), `unchecked` (visible, not ticked) or
    /// `hidden` (no checkbox). Default `checked`.
    #[arg(long, value_name = "checked|unchecked|hidden")]
    pub launch_option: Option<String>,

    /// Default install dir the UI proposes (per-app). May contain `%VAR%` env
    /// tokens, e.g. `%LOCALAPPDATA%\Programs\MyApp` or `C:\Games\MyApp`.
    #[arg(long, value_name = "DIR")]
    pub default_install_dir: Option<String>,

    /// Path to the Ed25519 private key file.
    #[arg(long)]
    pub priv_key: Option<PathBuf>,

    /// Ed25519 private key as a hex string.
    /// Mutually exclusive with `--priv-key`.
    #[arg(long, conflicts_with = "priv_key")]
    pub priv_key_literal: Option<String>,

    /// Path to the Ed25519 public key file. Required only in toolchain mode.
    #[arg(long)]
    pub pub_key: Option<PathBuf>,

    /// Ed25519 public key as a hex string.
    /// Mutually exclusive with `--pub-key`.
    #[arg(long, conflicts_with = "pub_key")]
    pub pub_key_literal: Option<String>,

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

/// One `[[shortcut]]` table from the config file. Converted + validated into a
/// [`ShortcutEntry`] by [`build_shortcuts`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ShortcutFileEntry {
    pub dir: String,
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub args: String,
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
    /// Opt-in: this plugin contributes custom wizard pages.
    #[serde(default)]
    pub ui: bool,
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
    pub ui: bool,
}

/// One `[[feature]]` table from the config file. Maps a set of path globs to a
/// feature id; files matching are tagged in the manifest. Converted + validated
/// into a [`ResolvedFeature`] by [`build_features`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FeatureFileEntry {
    pub id: String,
    pub paths: Vec<String>,
    /// Enabled by default on a fresh install; a plugin can override at runtime.
    #[serde(default)]
    pub default: bool,
}

/// A validated feature pack: its id, the path globs that belong to it, and
/// whether it is enabled by default on a fresh install.
#[derive(Debug, Clone)]
pub struct ResolvedFeature {
    pub id: String,
    pub paths: Vec<String>,
    pub default_enabled: bool,
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
    pub banner: Option<PathBuf>,
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
    pub install_dir_restriction: Option<String>,
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    #[serde(default)]
    pub show_uninstall_complete: bool,
    pub launch_option: Option<String>,
    pub default_install_dir: Option<String>,
    pub priv_key: Option<PathBuf>,
    pub priv_key_literal: Option<String>,
    pub pub_key: Option<PathBuf>,
    pub pub_key_literal: Option<String>,
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
    /// Shortcuts to create (config-file only). `[[shortcut]]` tables.
    #[serde(default, rename = "shortcut")]
    pub shortcuts: Vec<ShortcutFileEntry>,
    /// Feature packs (config-file only). `[[feature]]` tables mapping path globs
    /// to a feature id; a plugin activates them at install time.
    #[serde(default, rename = "feature")]
    pub features: Vec<FeatureFileEntry>,
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
    pub banner: Option<PathBuf>,
    pub assoc: Vec<String>,
    pub min_installer_version: String,
    pub force_reinstall: bool,
    pub purge_unknown_files: bool,
    pub skip_license: bool,
    pub skip_path: bool,
    pub install_dir_restriction: InstallDirRestriction,
    pub upgrade_minimal_ui: bool,
    pub show_uninstall_complete: bool,
    pub launch_option: LaunchOption,
    pub default_install_dir: Option<String>,
    pub priv_key: Option<PathBuf>,
    pub priv_key_literal: Option<String>,
    pub pub_key: Option<PathBuf>,
    pub pub_key_literal: Option<String>,
    pub installer_stub: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub out: PathBuf,
    pub reuse_stub: bool,
    pub registry: Vec<RegEntry>,
    pub plugins: Vec<ResolvedPlugin>,
    pub shortcuts: Vec<ShortcutEntry>,
    pub features: Vec<ResolvedFeature>,
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

        let merged_priv_key = cli.priv_key.or(file.priv_key.clone());
        let merged_priv_key_literal = cli.priv_key_literal.or(file.priv_key_literal.clone());
        if merged_priv_key.is_none() && merged_priv_key_literal.is_none() {
            anyhow::bail!(
                "missing private key: pass --priv-key <path> or --priv-key-literal <hex>"
            );
        }

        let product_id = cli
            .product_id
            .or(file.product_id)
            .with_context(|| req("product-id"))?;
        validate_product_id(&product_id)?;
        let publisher = cli
            .publisher
            .or(file.publisher)
            .with_context(|| req("publisher"))?;
        if publisher.trim().is_empty() {
            bail!("--publisher must not be empty");
        }

        Ok(PackArgs {
            product: cli
                .product
                .or(file.product)
                .with_context(|| req("product"))?,
            product_id,
            publisher,
            to_version: cli
                .to_version
                .or(file.to_version)
                .with_context(|| req("to-version"))?,
            input: cli.input.or(file.input).with_context(|| req("input"))?,
            exe: cli.exe.or(file.exe).with_context(|| req("exe"))?,
            priv_key: merged_priv_key,
            priv_key_literal: merged_priv_key_literal,
            out: cli.out.or(file.out).with_context(|| req("out"))?,

            from_dir: cli.from_dir.or(file.from_dir),
            from_version: cli.from_version.or(file.from_version),
            license: cli.license.or(file.license),
            banner: cli.banner.or(file.banner),
            default_install_dir: cli.default_install_dir.or(file.default_install_dir),
            pub_key: cli.pub_key.or(file.pub_key),
            pub_key_literal: cli.pub_key_literal.or(file.pub_key_literal),
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
            install_dir_restriction: parse_install_dir_restriction(
                cli.install_dir_restriction.or(file.install_dir_restriction),
            )?,
            upgrade_minimal_ui: cli.upgrade_minimal_ui || file.upgrade_minimal_ui,
            show_uninstall_complete: cli.show_uninstall_complete || file.show_uninstall_complete,
            launch_option: parse_launch_option(cli.launch_option.or(file.launch_option))?,
            reuse_stub: cli.reuse_stub || file.reuse_stub,
            registry: build_registry(file.registry)?,
            plugins: build_plugins(file.plugins)?,
            shortcuts: build_shortcuts(file.shortcuts)?,
            features: build_features(file.features)?,
        })
    }
}

/// Convert + validate `[[feature]]` entries. Ids must be non-empty, unique
/// (case-insensitive) and ASCII letters/digits/`-`/`_`; each needs ≥1 path glob.
fn build_features(raw: Vec<FeatureFileEntry>) -> Result<Vec<ResolvedFeature>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut seen = std::collections::HashSet::new();
    for (i, f) in raw.into_iter().enumerate() {
        let n = i + 1;
        let id = f.id.trim().to_string();
        if id.is_empty() {
            bail!("feature #{n}: empty id");
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("feature #{n} ('{id}'): id must be ASCII letters, digits, '-' or '_'");
        }
        if !seen.insert(id.to_ascii_lowercase()) {
            bail!("feature #{n}: duplicate id '{id}'");
        }
        let paths: Vec<String> = f
            .paths
            .into_iter()
            .map(|p| p.trim().replace('\\', "/"))
            .filter(|p| !p.is_empty())
            .collect();
        if paths.is_empty() {
            bail!("feature #{n} ('{id}'): needs at least one non-empty path");
        }
        out.push(ResolvedFeature {
            id,
            paths,
            default_enabled: f.default,
        });
    }
    Ok(out)
}

/// Parse the optional `install_dir_restriction` value (CLI or config).
/// Accepts `enforce` / `default-dir-only` / `bypass` (case- and
/// `_`/`-`-insensitive). Absent → [`InstallDirRestriction::Enforce`].
fn parse_install_dir_restriction(v: Option<String>) -> Result<InstallDirRestriction> {
    let Some(s) = v else {
        return Ok(InstallDirRestriction::Enforce);
    };
    match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "enforce" => Ok(InstallDirRestriction::Enforce),
        "default-dir-only" => Ok(InstallDirRestriction::DefaultDirOnly),
        "bypass" => Ok(InstallDirRestriction::Bypass),
        other => {
            bail!("unknown install-dir-restriction '{other}' (enforce | default-dir-only | bypass)")
        }
    }
}

/// Parse the optional `launch_option` value (CLI or config). Accepts
/// `checked` / `unchecked` / `hidden` (case-insensitive). Absent →
/// [`LaunchOption::Checked`].
fn parse_launch_option(v: Option<String>) -> Result<LaunchOption> {
    let Some(s) = v else {
        return Ok(LaunchOption::Checked);
    };
    match s.trim().to_ascii_lowercase().as_str() {
        "checked" => Ok(LaunchOption::Checked),
        "unchecked" => Ok(LaunchOption::Unchecked),
        "hidden" => Ok(LaunchOption::Hidden),
        other => bail!("unknown launch-option '{other}' (checked | unchecked | hidden)"),
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
            ui: p.ui,
        });
    }
    Ok(out)
}

/// Convert + validate `[[shortcut]]` entries. `dir`, `name` and `target` must
/// be non-empty; `name` must be a single filename component (no path separators
/// or characters illegal on Windows), since it becomes `<name>.lnk`.
fn build_shortcuts(raw: Vec<ShortcutFileEntry>) -> Result<Vec<ShortcutEntry>> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, s) in raw.into_iter().enumerate() {
        let n = i + 1;
        let dir = s.dir.trim().to_string();
        let name = s.name.trim().to_string();
        let target = s.target.trim().to_string();
        if dir.is_empty() {
            bail!("shortcut #{n}: empty dir");
        }
        if name.is_empty() {
            bail!("shortcut #{n}: empty name");
        }
        if target.is_empty() {
            bail!("shortcut #{n} ('{name}'): empty target");
        }
        if name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            bail!(
                "shortcut #{n} ('{name}'): name must be a single filename \
                 (no \\ / : * ? \" < > |)"
            );
        }
        out.push(ShortcutEntry {
            dir,
            name,
            target,
            args: s.args,
        });
    }
    Ok(out)
}

/// Convert + validate `[[registry]]` entries. HKCU or HKLM (HKLM needs an
/// elevated / machine-wide install); type/value must agree; key non-empty and
/// not starting with `\`.
fn build_registry(raw: Vec<RegFileEntry>) -> Result<Vec<RegEntry>> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, e) in raw.into_iter().enumerate() {
        let n = i + 1;
        let hive = if e.hive.eq_ignore_ascii_case("HKCU") {
            "HKCU"
        } else if e.hive.eq_ignore_ascii_case("HKLM") {
            "HKLM"
        } else {
            bail!(
                "registry #{n}: hive '{}' unsupported (HKCU or HKLM)",
                e.hive
            );
        };
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
            anyhow!(
                "registry #{n} ('{key}'): value does not match type '{}'",
                e.kind
            )
        })?;
        out.push(RegEntry {
            hive: hive.to_string(),
            key,
            name: e.name,
            kind,
            value,
        });
    }
    Ok(out)
}

/// Validate the registry-safe internal id: starts with an ASCII letter, then
/// ASCII alphanumerics / `.` / `-` / `_`, length 1..=50. Keeps it usable as an
/// HKCU subkey name and an association ProgID prefix.
fn validate_product_id(id: &str) -> Result<()> {
    let ok_len = (1..=50).contains(&id.len());
    let mut chars = id.chars();
    let ok_first = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let ok_rest = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ok_len && ok_first && ok_rest {
        Ok(())
    } else {
        bail!(
            "invalid --product-id '{}': must be 1-50 chars, start with an ASCII \
             letter, and contain only ASCII letters, digits, '.', '-' or '_' \
             (registry- and ProgID-safe)",
            id
        );
    }
}

/// Parse `--assoc ".ext:Description"` entries into `FileAssoc`s.
/// Extension is normalized to a leading dot; description may contain colons.
pub(crate) fn parse_assocs(raw: &[String], product_id: &str) -> Result<Vec<FileAssoc>> {
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let (ext, desc) = s
            .split_once(':')
            .ok_or_else(|| anyhow!("bad --assoc '{}': expected \".ext:Description\"", s))?;
        let ext = common::assoc::normalize_ext(ext);
        if ext == "." {
            bail!("bad --assoc '{}': empty extension", s);
        }
        let description = desc.trim().to_string();
        let progid = common::assoc::progid_for(product_id, &ext);
        println!("Association: {} -> {} ({})", ext, progid, description);
        out.push(FileAssoc { ext, description });
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
                .then_some(RegValue::Int(n as u64))
        }
        RegKind::Qword => {
            let n = v.as_integer()?;
            (n >= 0).then_some(RegValue::Int(n as u64))
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
            banner: None,
            assoc: Vec::new(),
            min_installer_version: None,
            force_reinstall: false,
            purge_unknown_files: false,
            skip_license: false,
            skip_path: false,
            install_dir_restriction: None,
            upgrade_minimal_ui: false,
            show_uninstall_complete: false,
            launch_option: None,
            default_install_dir: None,
            priv_key: None,
            priv_key_literal: None,
            pub_key: None,
            pub_key_literal: None,
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
        let mut cli = empty_cli();
        cli.priv_key_literal = Some("aa".repeat(32)); // satisfy key check so product fires
        let err = PackArgs::resolve(cli).unwrap_err().to_string();
        assert!(err.contains("product"), "got: {err}");
    }

    #[test]
    fn product_id_validation() {
        for ok in ["MyApp", "Acme.App", "a-b_c", "App2", "x"] {
            assert!(validate_product_id(ok).is_ok(), "should accept {ok}");
        }
        for bad in [
            "",              // empty
            "1abc",          // starts with digit
            "_app",          // starts with non-letter
            "my app",        // space
            "a/b",           // slash
            "app:1",         // colon
            "café",          // non-ASCII
            &"a".repeat(51), // too long
        ] {
            assert!(validate_product_id(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn parse_assocs_valid_and_colon_in_desc() {
        let v = parse_assocs(&[".myx:My Doc".to_string(), ".a:b:c".to_string()], "Prod").unwrap();
        assert_eq!(v[0].ext, ".myx");
        assert_eq!(v[0].description, "My Doc");
        // split_once on the first ':' -> description keeps the rest.
        assert_eq!(v[1].ext, ".a");
        assert_eq!(v[1].description, "b:c");
    }

    #[test]
    fn parse_assocs_rejects_bad() {
        assert!(parse_assocs(&["noColon".to_string()], "P").is_err());
        assert!(parse_assocs(&[":nodesc".to_string()], "P").is_err()); // empty ext
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
        // unknown hive (only HKCU / HKLM allowed)
        assert!(
            resolve_with("\n[[registry]]\nhive='HKXX'\nkey='K'\ntype='sz'\nvalue='x'\n").is_err()
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
        // empty key
        assert!(
            resolve_with("\n[[registry]]\nhive='HKCU'\nkey=''\ntype='sz'\nvalue='x'\n").is_err()
        );
        // key starting with '\'
        assert!(
            resolve_with("\n[[registry]]\nhive='HKCU'\nkey='\\X'\ntype='sz'\nvalue='x'\n").is_err()
        );
    }

    #[test]
    fn registry_accepts_hklm() {
        let r =
            resolve_with("\n[[registry]]\nhive='hklm'\nkey='Software\\X'\ntype='sz'\nvalue='y'\n")
                .unwrap();
        assert_eq!(r.registry[0].hive, "HKLM");
    }

    #[test]
    fn plugins_parsed_and_validated() {
        let r = resolve_with(
            "\n[[plugin]]\nname='do-x'\ndll='plugins/x.dll'\nphase='pre-install'\n\
             [[plugin]]\nname='do_y'\ndll='plugins/y.dll'\nphase='post_install'\nrequired=false\n\
             [[plugin]]\nname='ask'\ndll='plugins/ask.dll'\nphase='pre-install'\nui=true\n",
        )
        .unwrap();
        assert_eq!(r.plugins.len(), 3);
        assert_eq!(r.plugins[0].name, "do-x");
        assert_eq!(r.plugins[0].phase, PluginPhase::PreInstall);
        assert!(r.plugins[0].required);
        assert!(!r.plugins[0].ui); // defaults false
        assert_eq!(r.plugins[1].phase, PluginPhase::PostInstall);
        assert!(!r.plugins[1].required);
        assert!(r.plugins[2].ui); // opt-in parsed
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

    #[test]
    fn features_parsed_and_validated() {
        let r = resolve_with(
            "\n[[feature]]\nid='D1'\npaths=['Dossier1']\ndefault=true\n\
             [[feature]]\nid='D2'\npaths=['data/**','extra/*.bin']\n",
        )
        .unwrap();
        assert_eq!(r.features.len(), 2);
        assert_eq!(r.features[0].id, "D1");
        assert_eq!(r.features[0].paths, vec!["Dossier1".to_string()]);
        assert!(r.features[0].default_enabled); // default=true parsed
        assert_eq!(r.features[1].paths.len(), 2);
        assert!(!r.features[1].default_enabled); // omitted → false
    }

    #[test]
    fn features_reject_bad() {
        // empty id
        assert!(resolve_with("\n[[feature]]\nid=''\npaths=['x']\n").is_err());
        // illegal id char
        assert!(resolve_with("\n[[feature]]\nid='a b'\npaths=['x']\n").is_err());
        // duplicate (case-insensitive)
        assert!(
            resolve_with("\n[[feature]]\nid='a'\npaths=['x']\n[[feature]]\nid='A'\npaths=['y']\n")
                .is_err()
        );
        // no paths
        assert!(resolve_with("\n[[feature]]\nid='a'\npaths=[]\n").is_err());
        // unknown field
        assert!(resolve_with("\n[[feature]]\nid='a'\npaths=['x']\nnope=1\n").is_err());
    }

    #[test]
    fn shortcuts_parsed_and_validated() {
        let r = resolve_with(
            "\n[[shortcut]]\ndir='%DESKTOP%'\nname='My App'\ntarget='%EXE%'\n\
             [[shortcut]]\ndir='%INSTALL_DIR%'\nname='Tools'\ntarget='bin/tool.exe'\nargs='--fast'\n",
        )
        .unwrap();
        assert_eq!(r.shortcuts.len(), 2);
        assert_eq!(r.shortcuts[0].dir, "%DESKTOP%");
        assert_eq!(r.shortcuts[0].name, "My App");
        assert_eq!(r.shortcuts[0].target, "%EXE%");
        assert_eq!(r.shortcuts[0].args, "");
        assert_eq!(r.shortcuts[1].args, "--fast");
    }

    #[test]
    fn shortcuts_reject_bad() {
        // empty name
        assert!(
            resolve_with("\n[[shortcut]]\ndir='%DESKTOP%'\nname=''\ntarget='a.exe'\n").is_err()
        );
        // empty target
        assert!(resolve_with("\n[[shortcut]]\ndir='%DESKTOP%'\nname='X'\ntarget=''\n").is_err());
        // empty dir
        assert!(resolve_with("\n[[shortcut]]\ndir=''\nname='X'\ntarget='a.exe'\n").is_err());
        // path separator in name
        assert!(
            resolve_with("\n[[shortcut]]\ndir='%DESKTOP%'\nname='a\\\\b'\ntarget='a.exe'\n")
                .is_err()
        );
        // unknown field
        assert!(
            resolve_with("\n[[shortcut]]\ndir='%DESKTOP%'\nname='X'\ntarget='a.exe'\nicon='x'\n")
                .is_err()
        );
    }

    #[test]
    fn install_dir_restriction_defaults_and_parses() {
        // Absent → Enforce.
        assert_eq!(
            resolve_with("").unwrap().install_dir_restriction,
            InstallDirRestriction::Enforce
        );
        // From file, accepting `-`/`_` and case variations.
        assert_eq!(
            resolve_with("\ninstall_dir_restriction = 'default_dir_only'\n")
                .unwrap()
                .install_dir_restriction,
            InstallDirRestriction::DefaultDirOnly
        );
        assert_eq!(
            resolve_with("\ninstall_dir_restriction = 'BYPASS'\n")
                .unwrap()
                .install_dir_restriction,
            InstallDirRestriction::Bypass
        );
        // Unknown value errors.
        assert!(resolve_with("\ninstall_dir_restriction = 'nope'\n").is_err());
    }

    #[test]
    fn launch_option_defaults_and_parses() {
        // Absent → Checked.
        assert_eq!(
            resolve_with("").unwrap().launch_option,
            LaunchOption::Checked
        );
        // From file, case-insensitive.
        assert_eq!(
            resolve_with("\nlaunch_option = 'Unchecked'\n")
                .unwrap()
                .launch_option,
            LaunchOption::Unchecked
        );
        assert_eq!(
            resolve_with("\nlaunch_option = 'hidden'\n")
                .unwrap()
                .launch_option,
            LaunchOption::Hidden
        );
        // Unknown value errors.
        assert!(resolve_with("\nlaunch_option = 'nope'\n").is_err());
    }

    // SAMPLE without a priv_key line — used to test the literal-key path.
    const SAMPLE_NO_KEY: &str = "\
product = 'myapp'
product_id = 'myapp'
publisher = 'Acme'
to_version = '1.0'
input = 'build/myapp'
exe = 'myapp.exe'
out = 'dist/setup.exe'
";

    #[test]
    fn priv_key_literal_via_cli() {
        let (_dir, cfg) = write_cfg(SAMPLE_NO_KEY);
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        cli.priv_key_literal = Some("ff".repeat(32));
        let r = PackArgs::resolve(cli).unwrap();
        assert!(r.priv_key.is_none());
        assert_eq!(r.priv_key_literal, Some("ff".repeat(32)));
    }

    #[test]
    fn priv_key_literal_via_file() {
        let hex = "aa".repeat(32);
        let (_dir, cfg) = write_cfg(&format!("{SAMPLE_NO_KEY}\npriv_key_literal = '{hex}'\n"));
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        let r = PackArgs::resolve(cli).unwrap();
        assert!(r.priv_key.is_none());
        assert_eq!(r.priv_key_literal, Some(hex));
    }

    #[test]
    fn missing_priv_key_errors() {
        let (_dir, cfg) = write_cfg(SAMPLE_NO_KEY);
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        let err = PackArgs::resolve(cli).unwrap_err().to_string();
        assert!(err.contains("private key"), "got: {err}");
    }

    #[test]
    fn pub_key_literal_via_cli() {
        let (_dir, cfg) = write_cfg(SAMPLE_NO_KEY);
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        cli.priv_key_literal = Some("aa".repeat(32));
        cli.pub_key_literal = Some("bb".repeat(32));
        let r = PackArgs::resolve(cli).unwrap();
        assert!(r.pub_key.is_none());
        assert_eq!(r.pub_key_literal, Some("bb".repeat(32)));
    }

    #[test]
    fn pub_key_literal_via_file() {
        let hex = "cc".repeat(32);
        let (_dir, cfg) = write_cfg(&format!(
            "{SAMPLE_NO_KEY}\npriv_key_literal = '{}'\npub_key_literal = '{hex}'\n",
            "aa".repeat(32)
        ));
        let mut cli = empty_cli();
        cli.config = Some(cfg);
        let r = PackArgs::resolve(cli).unwrap();
        assert!(r.pub_key.is_none());
        assert_eq!(r.pub_key_literal, Some(hex));
    }
}
