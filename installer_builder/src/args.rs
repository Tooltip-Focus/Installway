// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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

    /// Product name (key).
    #[arg(short, long)]
    pub product: Option<String>,

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

/// `pack` options as read from a TOML file. Flat keys matching the CLI long
/// names (snake_case). Unknown keys are rejected to catch typos.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub product: Option<String>,
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
    pub skip_license: bool,
    #[serde(default)]
    pub skip_path: bool,
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    pub default_install_dir: Option<String>,
    pub priv_key: Option<PathBuf>,
    pub pub_key: Option<PathBuf>,
    pub installer_stub: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub out: Option<PathBuf>,
    #[serde(default)]
    pub reuse_stub: bool,
}

/// Fully resolved `pack` options consumed by `pack::run`. CLI > TOML > default.
#[derive(Debug, Clone)]
pub struct PackArgs {
    pub product: String,
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
    pub skip_license: bool,
    pub skip_path: bool,
    pub upgrade_minimal_ui: bool,
    pub default_install_dir: Option<String>,
    pub priv_key: PathBuf,
    pub pub_key: Option<PathBuf>,
    pub installer_stub: Option<PathBuf>,
    pub uninstaller: Option<PathBuf>,
    pub out: PathBuf,
    pub reuse_stub: bool,
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
            product: cli.product.or(file.product).with_context(|| req("product"))?,
            publisher: cli.publisher.or(file.publisher).with_context(|| req("publisher"))?,
            to_version: cli.to_version.or(file.to_version).with_context(|| req("to-version"))?,
            input: cli.input.or(file.input).with_context(|| req("input"))?,
            exe: cli.exe.or(file.exe).with_context(|| req("exe"))?,
            priv_key: cli.priv_key.or(file.priv_key).with_context(|| req("priv-key"))?,
            out: cli.out.or(file.out).with_context(|| req("out"))?,

            from_dir: cli.from_dir.or(file.from_dir),
            from_version: cli.from_version.or(file.from_version),
            license: cli.license.or(file.license),
            default_install_dir: cli.default_install_dir.or(file.default_install_dir),
            pub_key: cli.pub_key.or(file.pub_key),
            installer_stub: cli.installer_stub.or(file.installer_stub),
            uninstaller: cli.uninstaller.or(file.uninstaller),

            // CLI list replaces the file list when present.
            assoc: if cli.assoc.is_empty() { file.assoc } else { cli.assoc },
            min_installer_version: cli
                .min_installer_version
                .or(file.min_installer_version)
                .unwrap_or_else(|| "1.0.0".to_string()),
            // Boolean flags: either source can turn them on.
            force_reinstall: cli.force_reinstall || file.force_reinstall,
            skip_license: cli.skip_license || file.skip_license,
            skip_path: cli.skip_path || file.skip_path,
            upgrade_minimal_ui: cli.upgrade_minimal_ui || file.upgrade_minimal_ui,
            reuse_stub: cli.reuse_stub || file.reuse_stub,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> PackCli {
        PackCli {
            config: None,
            product: None,
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
            skip_license: false,
            skip_path: false,
            upgrade_minimal_ui: false,
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
}
