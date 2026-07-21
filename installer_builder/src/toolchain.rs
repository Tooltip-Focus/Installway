// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Cargo invocations for the sibling crates: build (or reuse) the installer
//! stub and the uninstaller from the workspace.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;

/// `cargo build --release -p <package>` in the workspace root, reusing the
/// existing artifact when `reuse` is set. `env` is applied to the build (e.g.
/// `INSTALLER_PUB_KEY` for the stub), and `feature` enables an optional Cargo
/// feature such as `hintway`. Returns the built exe path.
pub(crate) fn cargo_build_release(
    package: &str,
    exe_name: &str,
    env: Option<(&str, &str)>,
    feature: Option<&str>,
    reuse: bool,
) -> Result<PathBuf> {
    let workspace_root = find_workspace_root()?;
    let target_exe = workspace_root.join("target").join("release").join(exe_name);

    if reuse && target_exe.exists() {
        println!("Reusing existing {} at {}", exe_name, target_exe.display());
        return Ok(target_exe);
    }

    println!(
        "Building {package} (cargo build -p {package} --release{})...",
        feature
            .map(|name| format!(" --features {name}"))
            .unwrap_or_default()
    );
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["build", "-p", package, "--release"])
        .current_dir(&workspace_root);
    if let Some(feature) = feature {
        cmd.args(["--features", feature]);
    }
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .with_context(|| format!("invoke cargo build {package}"))?;
    if !status.success() {
        bail!("cargo build {package} failed");
    }
    if !target_exe.exists() {
        bail!(
            "expected {} not found at {}",
            exe_name,
            target_exe.display()
        );
    }
    Ok(target_exe)
}

/// Walk up from the current dir to the first `Cargo.toml` containing a
/// `[workspace]` table.
fn find_workspace_root() -> Result<PathBuf> {
    let mut p: PathBuf = std::env::current_dir()?;
    loop {
        let manifest = p.join("Cargo.toml");
        if manifest.exists() {
            let text = fs::read_to_string(&manifest).unwrap_or_default();
            if text.contains("[workspace]") {
                return Ok(p);
            }
        }
        if !p.pop() {
            bail!(
                "could not locate workspace root from {:?}",
                std::env::current_dir()
            );
        }
    }
}
