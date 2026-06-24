// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use crate::cleanup;
use crate::ui::{self, UninstallParams};
use anyhow::{Context, Result};
use common::model::manifest::Manifest;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x00000008;

fn assoc_id(info: &common::model::install_info::InstallInfo) -> &str {
    if info.product_id.is_empty() {
        &info.product
    } else {
        &info.product_id
    }
}

pub fn run(silent: bool) -> Result<()> {
    // Runs from the data dir, not the app dir; the real app dir comes from
    // installer_info.json.
    let data_dir = cleanup::self_dir()?;

    // Log in %TEMP% so it survives the rmdir of both dirs. Hint = data-dir
    // folder name (the product_id).
    let product_hint = data_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    common::log::init(common::log::log_path_uninstall_temp(
        &product_hint,
        std::process::id(),
    ));
    common::log::prune_temp_logs(&product_hint, 14);

    // If the metadata is gone, just remove leftovers quietly (no error dialog).
    let info = match cleanup::read_info(&data_dir) {
        Ok(i) => i,
        Err(e) => {
            common::log::warn(format!(
                "installer_info.json unreadable ({e:#}) - best-effort cleanup of leftovers"
            ));
            spawn_finalize(None, &data_dir, None, false)?;
            return Ok(());
        }
    };

    let app_dir = std::path::PathBuf::from(&info.install_dir);

    // Manifest may be missing (partial delete). Fall back to empty: file
    // removal no-ops, but shortcuts/registry/dir cleanup still run.
    let manifest = cleanup::read_manifest(&data_dir).unwrap_or_else(|e| {
        common::log::warn(format!("manifest unreadable ({e:#}) - skipping file list"));
        Manifest {
            version: info.version.clone(),
            exe: info.exe.clone(),
            files: Default::default(),
            deleted_files: Vec::new(),
            full_size: 0,
            total_patch_size: 0,
            features: Vec::new(),
            default_features: Vec::new(),
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

    let app_dir_owned = app_dir.clone();
    let data_dir_owned = data_dir.clone();
    let info_owned = info.clone();
    let manifest_owned = manifest.clone();
    let tr = ui::tr();

    // Elevate when the install was recorded machine-wide, OR when the app/data
    // dir is actually permission-walled (an admin install to a custom ACL'd
    // folder that wasn't flagged machine-wide). The probe distinguishes a real
    // ACL wall from a transient AV lock, which the retry loop handles instead.
    let needs_elevation = !common::elevation::is_already_elevated()
        && (info_owned.requires_admin
            || cleanup::perm_denied(&app_dir)
            || cleanup::perm_denied(&data_dir));

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
        worker: Box::new(move |progress: ui::Progress| {
            if needs_elevation {
                if let Err(e) = run_elevated(progress) {
                    ui::fatal(&format!("{e:#}"));
                }
                return;
            }
            let tr = ui::tr();
            do_cleanup(
                &info_owned,
                &manifest_owned,
                &app_dir_owned,
                &data_dir_owned,
                |done, total, label| {
                    let msg = match label {
                        "shortcuts" => tr.get("uninstall.removing_shortcuts"),
                        "state" => tr.get("uninstall.removing_state"),
                        "registry" => tr.get("uninstall.finalizing"),
                        file => tr.fmt("uninstall.removing", &[("file", file)]),
                    };
                    progress(done, total, &msg);
                },
            );
            common::log::info("spawning finalize step");
            if let Err(e) = spawn_finalize(
                Some(&app_dir_owned),
                &data_dir_owned,
                Some(&info_owned.product),
                info_owned.show_uninstall_complete,
            ) {
                common::log::error(format!("finalize spawn failed: {e:#}"));
                ui::fatal(&tr.fmt("uninstall.spawn_failed", &[("err", &format!("{e:#}"))]));
            }
        }),
        auto_start: false,
    };

    ui::run(params);
    Ok(())
}

/// Core uninstall operations shared by the interactive and elevated-worker paths.
/// `on_step(done, total, label)` is called before each unit of work; label is a
/// file path for payload files, or "shortcuts" / "state" / "registry" for the
/// three fixed phases.
pub(crate) fn do_cleanup(
    info: &common::model::install_info::InstallInfo,
    manifest: &Manifest,
    app_dir: &Path,
    data_dir: &Path,
    mut on_step: impl FnMut(u64, u64, &str),
) {
    let total = manifest.files.len() as u64 + 3;
    let mut done = 0u64;
    let mut step = |label: &str| {
        done += 1;
        on_step(done, total, label);
    };

    run_down_plugins(info, data_dir);

    for rel in manifest.files.keys() {
        step(rel);
        cleanup::remove_one_payload(&app_dir.join(rel));
    }

    step("shortcuts");
    cleanup::remove_shortcuts(info);
    common::assoc::unregister(assoc_id(info), &info.associations, info.requires_admin);
    for e in &info.registry {
        common::registry::remove_if_ours(e);
    }

    step("state");
    cleanup::remove_app_state_files(app_dir);
    cleanup::remove_empty_subdirs(app_dir);

    step("registry");
    cleanup::unregister(&info.registry_key, info.requires_admin);
}

fn run_silent(
    app_dir: &Path,
    data_dir: &Path,
    info: &common::model::install_info::InstallInfo,
    manifest: &Manifest,
) -> Result<()> {
    run_down_plugins(info, data_dir);
    let n = cleanup::remove_payload_files(app_dir, manifest);
    common::log::info(format!("removed {} payload files", n));
    cleanup::remove_shortcuts(info);
    common::assoc::unregister(assoc_id(info), &info.associations, info.requires_admin);
    for e in &info.registry {
        common::registry::remove_if_ours(e);
    }
    common::log::info("removed shortcuts + associations");
    let s = cleanup::remove_app_state_files(app_dir);
    common::log::info(format!("removed {} app state files", s));
    cleanup::remove_empty_subdirs(app_dir);
    cleanup::unregister(&info.registry_key, info.requires_admin);
    common::log::info(format!(
        "unregistered {}\\Uninstall\\{}",
        if info.requires_admin { "HKLM" } else { "HKCU" },
        info.registry_key
    ));
    spawn_finalize(Some(app_dir), data_dir, None, false)
}

fn run_elevated(progress: ui::Progress) -> anyhow::Result<()> {
    use anyhow::bail;
    use common::elevation::{WorkerEvent, recv};

    let pipe_name = common::elevation::pipe_name(std::process::id());
    let pipe_handle = common::elevation::create_pipe_server(&pipe_name)?;

    if common::elevation::spawn_elevated_worker(&pipe_name).is_err() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
        }
        bail!("{}", ui::tr().get("uninstall.uac_cancelled"));
    }

    common::elevation::wait_for_client(pipe_handle)?;
    let pipe_file = common::elevation::open_pipe_handle(pipe_handle);
    let mut reader = common::elevation::event_reader(pipe_file);

    loop {
        match recv::<WorkerEvent, _>(&mut reader) {
            Ok(Some(WorkerEvent::Progress { done, total, name })) => {
                progress(done, total, &name);
            }
            Ok(Some(WorkerEvent::Done)) => break,
            Ok(Some(WorkerEvent::Error { msg })) => bail!("{msg}"),
            Ok(None) => bail!("elevated worker exited unexpectedly"),
            Err(e) => bail!("pipe read error: {e:#}"),
        }
    }
    Ok(())
}

fn run_down_plugins(info: &common::model::install_info::InstallInfo, data_dir: &Path) {
    if info.plugins.is_empty() {
        return;
    }
    let items: Vec<_> = info
        .plugins
        .iter()
        .rev()
        .map(|p| (p.clone(), data_dir.join(&p.file), String::new()))
        .collect();
    let ctx = common::plugin::PluginCtx {
        install_dir: info.install_dir.clone(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        product: info.product.clone(),
        product_id: info.product_id.clone(),
        version: info.version.clone(),
        exe: Path::new(&info.install_dir)
            .join(&info.exe)
            .to_string_lossy()
            .replace('/', "\\"),
        log_path: common::log::current_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        lang: common::i18n::current_lang().to_string(),
        ..Default::default()
    };
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = common::plugin::run_each(&self_exe, &ctx, &items, "down", false);
    }
}

/// Copies this exe to %TEMP% and spawns the finalize step detached. `app_dir`
/// is `None` when metadata was unreadable (skips app-dir removal).
pub(crate) fn spawn_finalize(
    app_dir: Option<&Path>,
    data_dir: &Path,
    display_name: Option<&str>,
    show_complete: bool,
) -> Result<()> {
    let product = data_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let self_exe = std::env::current_exe()?;
    let dest = staged_temp_path()?;
    // Retry past a transient AV scan of the freshly copied `.exe`; if this copy
    // fails outright, finalize never runs and the app/data dirs are never deleted.
    common::utils::copy_retry(&self_exe, &dest)
        .with_context(|| format!("copy finalize step to {}", dest.display()))?;

    let mut cmd = Command::new(&dest);
    cmd.arg("finalize")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--product")
        .arg(product)
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    if let Some(dir) = app_dir {
        cmd.arg("--app-dir").arg(dir);
    }
    if show_complete && let Some(name) = display_name {
        cmd.arg("--display-name").arg(name).arg("--show-complete");
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
