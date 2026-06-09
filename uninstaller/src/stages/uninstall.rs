// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Uninstall step: runs from `<install_dir>\uninstall.exe`. Shows confirm dialog,
//! then does the bulk of cleanup (files, shortcuts, registry, empty subdirs).
//! When done, copies itself into `%TEMP%` and spawns the finalize step, then exits
//! so finalize can delete `uninstall.exe` and the install_dir without lock issues.

use crate::cleanup;
use crate::ui::{self, StepCounter, UninstallParams};
use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

const DETACHED_PROCESS: u32 = 0x00000008;

pub fn run(silent: bool) -> Result<()> {
    // We run from the data dir (%LOCALAPPDATA%\<publisher>\Uninstall\<product>),
    // NOT the application dir. The real app dir comes from installer_info.json.
    let data_dir = cleanup::self_dir()?;

    // Uninstall log lives in %TEMP% so it survives the rmdir of both dirs.
    // Name it by product (the data-dir folder name is the sanitized product)
    // so support can tell which app this log is for.
    let product_hint = data_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    common::log::init(common::log::log_path_uninstall_temp(
        &product_hint,
        std::process::id(),
    ));
    // Self-clean: drop this product's stale %TEMP% logs (> 14 days).
    common::log::prune_temp_logs(&product_hint, 14);

    // If the metadata is gone, just remove leftovers quietly (no error dialog).
    let info = match cleanup::read_info(&data_dir) {
        Ok(i) => i,
        Err(e) => {
            common::log::warn(format!(
                "installer_info.json unreadable ({e:#}) - best-effort cleanup of leftovers"
            ));
            let product = data_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            spawn_finalize(None, &data_dir, &product)?;
            return Ok(());
        }
    };

    let app_dir = std::path::PathBuf::from(&info.install_dir);

    // Manifest may be missing (partial delete). Fall back to empty: file
    // removal no-ops, but shortcuts/registry/dir cleanup still run.
    let manifest = cleanup::read_manifest(&data_dir).unwrap_or_else(|e| {
        common::log::warn(format!("manifest unreadable ({e:#}) - skipping file list"));
        common::models::Manifest {
            version: info.version.clone(),
            exe: info.exe.clone(),
            files: Default::default(),
            deleted_files: Vec::new(),
            full_size: 0,
            total_patch_size: 0,
        }
    });

    common::log::info(format!(
        "uninstall start: product={} version={} app_dir={} data_dir={} silent={}",
        info.product,
        info.version,
        app_dir.display(),
        data_dir.display(),
        silent
    ));

    if silent {
        return run_silent(&app_dir, &data_dir, &info, &manifest);
    }

    let total_steps = manifest.files.len() as u64 + 3 /* shortcuts + state + registry */;

    let app_dir_owned = app_dir.clone();
    let data_dir_owned = data_dir.clone();
    let info_owned = info.clone();
    let manifest_owned = manifest.clone();
    let tr = ui::tr();

    let params = UninstallParams {
        title: tr.fmt("uninstall.title", &[("product", &info.product)]),
        subtitle: tr.fmt("uninstall.subtitle", &[("version", &info.version)]),
        confirm_text: tr.fmt(
            "uninstall.confirm",
            &[
                ("product", &info.product),
                ("version", &info.version),
                ("path", &info.install_dir),
            ],
        ),
        worker: Box::new(move |progress: Arc<dyn Fn(u64, u64, &str) + Send + Sync>| {
            let counter = StepCounter::new(total_steps, progress);
            let tr = ui::tr();

            // 1. Payload files - robust removal (retry locks, then reboot-delete).
            for rel in manifest_owned.files.keys() {
                let p = app_dir_owned.join(rel);
                counter.step(&tr.fmt("uninstall.removing", &[("file", rel)]));
                cleanup::remove_one_payload(&p);
            }

            // 2. Shortcuts + file associations
            counter.step(&tr.get("uninstall.removing_shortcuts"));
            cleanup::remove_shortcuts(&info_owned.product);
            common::assoc::unregister(&info_owned.product, &info_owned.associations);

            // 3. App-dir state files (version.json, installer_manifest.json)
            counter.step(&tr.get("uninstall.removing_state"));
            cleanup::remove_app_state_files(&app_dir_owned);

            // 4. Empty subdirectories in the app dir
            counter.report(&tr.get("uninstall.finalizing"));
            cleanup::remove_empty_subdirs(&app_dir_owned);

            // 5. Registry - last so the entry stays visible until cleanup ran.
            cleanup::unregister(&info_owned.registry_key);

            // 6. Finalize step deletes the app dir + the data dir (incl. us) + self.
            common::log::info("spawning finalize step to delete app dir + data dir + self");
            if let Err(e) = spawn_finalize(Some(&app_dir_owned), &data_dir_owned, &info_owned.product) {
                common::log::error(format!("finalize spawn failed: {e:#}"));
                ui::fatal(&tr.fmt("uninstall.spawn_failed", &[("err", &format!("{e:#}"))]));
            }
        }),
        auto_start: false,
    };

    if ui::run(params) {
        ui::info(
            &tr.fmt("uninstall.complete_message", &[("product", &info.product)]),
            &tr.get("uninstall.complete_caption"),
        );
    }
    Ok(())
}

fn run_silent(
    app_dir: &Path,
    data_dir: &Path,
    info: &common::models::InstallInfo,
    manifest: &common::models::Manifest,
) -> Result<()> {
    let n = cleanup::remove_payload_files(app_dir, manifest);
    common::log::info(format!("removed {} payload files", n));
    cleanup::remove_shortcuts(&info.product);
    common::assoc::unregister(&info.product, &info.associations);
    common::log::info("removed shortcuts + associations");
    let s = cleanup::remove_app_state_files(app_dir);
    common::log::info(format!("removed {} app state files", s));
    cleanup::remove_empty_subdirs(app_dir);
    cleanup::unregister(&info.registry_key);
    common::log::info(format!("unregistered HKCU Uninstall\\{}", info.registry_key));
    spawn_finalize(Some(app_dir), data_dir, &info.product)
}

/// Spawn the temp-copy finalize step that deletes the app dir + data dir + itself.
///
/// `app_dir` is `None` when the installer metadata was unreadable; the finalize
/// step will skip the app-dir removal in that case.
fn spawn_finalize(app_dir: Option<&Path>, data_dir: &Path, product: &str) -> Result<()> {
    let self_exe = std::env::current_exe()?;
    let dest = staged_temp_path()?;
    // Retry past a transient AV scan of the freshly copied `.exe`; if this copy
    // fails outright, finalize never runs and the app/data dirs are never deleted.
    common::utils::copy_retry(&self_exe, &dest)
        .with_context(|| format!("copy finalize step to {}", dest.display()))?;

    let mut cmd = Command::new(&dest);
    cmd.arg("finalize")
        .arg("--data-dir").arg(data_dir)
        .arg("--product").arg(product)
        .arg("--parent-pid").arg(std::process::id().to_string());
    if let Some(dir) = app_dir {
        cmd.arg("--app-dir").arg(dir);
    }
    cmd.creation_flags(DETACHED_PROCESS)
        .spawn()
        .with_context(|| format!("spawn {}", dest.display()))?;
    Ok(())
}

fn staged_temp_path() -> Result<std::path::PathBuf> {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustinst-uninstall-{}-{}.exe",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    Ok(p)
}
