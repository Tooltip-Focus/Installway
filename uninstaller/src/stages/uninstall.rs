// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use crate::cleanup;
use crate::ui::{self, UninstallParams};
use anyhow::{Context, Result};
use common::i18n::Translator;
use common::model::install_info::InstallInfo;
use common::model::manifest::Manifest;
use common::model::plugin_ctx::PluginContext;
use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x00000008;

/// How long a temp log file is kept for this product, in days.
const TEMP_LOG_MAX_AGE_DAYS: u64 = 14;

fn assoc_id(info: &InstallInfo) -> &str {
    if info.product_id.is_empty() {
        &info.product
    } else {
        &info.product_id
    }
}

/// The data dir's folder name (the product_id). Used as the log-file key and as
/// the `--product` hint handed to the finalize step.
fn data_dir_key(data_dir: &Path) -> String {
    data_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Log in %TEMP% so it survives the rmdir of both the app and data dirs.
fn init_temp_log(data_dir: &Path) {
    let product_hint = data_dir_key(data_dir);
    common::log::init(common::log::log_path_uninstall_temp(
        &product_hint,
        std::process::id(),
    ));
    common::log::prune_temp_logs(&product_hint, TEMP_LOG_MAX_AGE_DAYS);
}

/// Windows 11's Settings app runs `UninstallString` inside its own package, and
/// this process inherited that identity - WinUI cannot start there. Hand the
/// uninstall to Explorer, which starts it as a plain desktop process.
///
/// `true` when the uninstall now belongs to the relaunched process and this one
/// should just exit.
fn relaunched_via_explorer(silent: bool) -> bool {
    if silent || !winui_support::inherited_package_identity() {
        return false;
    }
    match crate::relaunch::via_explorer() {
        Ok(()) => {
            common::log::info("launched from a packaged app; relaunched via Explorer");
            true
        }
        Err(e) => {
            common::log::warn(format!(
                "Explorer relaunch failed ({e:#}) - continuing here with the Win32 UI"
            ));
            false
        }
    }
}

pub fn run(silent: bool) -> Result<()> {
    // Runs from the data dir, not the app dir; the real app dir comes from
    // installer_info.json.
    let data_dir = cleanup::self_dir()?;
    init_temp_log(&data_dir);

    if relaunched_via_explorer(silent) {
        return Ok(());
    }

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

    let app_dir = PathBuf::from(&info.install_dir);

    // Manifest may be missing (partial delete). Fall back to empty: file
    // removal no-ops, but shortcuts/registry/dir cleanup still run.
    let manifest = cleanup::read_manifest(&data_dir).unwrap_or_else(|e| {
        common::log::warn(format!("manifest unreadable ({e:#}) - skipping file list"));
        Manifest::fallback(&info.version, info.exe.as_deref())
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

    ui::run(interactive_params(info, manifest, app_dir, data_dir));
    Ok(())
}

/// Build the interactive UI parameters, including the worker that does the
/// actual removal once the user confirms.
///
/// The elevation probe runs here, before the UI is shown - not inside the
/// worker - so the decision is made whether or not the user ever confirms.
fn interactive_params(
    info: InstallInfo,
    manifest: Manifest,
    app_dir: PathBuf,
    data_dir: PathBuf,
) -> UninstallParams {
    let tr = ui::tr();

    // Elevate when the install was recorded machine-wide, OR when the app/data
    // dir is actually permission-walled (an admin install to a custom ACL'd
    // folder that wasn't flagged machine-wide).
    let needs_elevation = !common::elevation::is_already_elevated()
        && (info.requires_admin
            || cleanup::perm_denied(&app_dir)
            || cleanup::perm_denied(&data_dir));

    UninstallParams {
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
        worker: ui::Worker::new(move |progress: ui::Progress| {
            if needs_elevation {
                if let Err(e) = run_elevated(progress) {
                    ui::fatal(&format!("{e:#}"));
                }
                return;
            }
            run_here(&info, &manifest, &app_dir, &data_dir, progress);
        }),
        auto_start: false,
    }
}

/// Worker body for the unelevated interactive path. Runs on the UI's background
/// thread, where `ui::tr()` resolves to the process-wide language.
fn run_here(
    info: &InstallInfo,
    manifest: &Manifest,
    app_dir: &Path,
    data_dir: &Path,
    progress: ui::Progress,
) {
    let tr = ui::tr();
    do_cleanup(info, manifest, app_dir, data_dir, |done, total, label| {
        progress(done, total, &progress_message(&tr, label));
    });
    common::log::info("spawning finalize step");
    if let Err(e) = spawn_finalize(
        Some(app_dir),
        data_dir,
        Some(&info.product),
        info.show_uninstall_complete,
    ) {
        common::log::error(format!("finalize spawn failed: {e:#}"));
        ui::fatal(&tr.fmt("uninstall.spawn_failed", &[("err", &format!("{e:#}"))]));
    }
}

/// Turn one `do_cleanup` label into the user-facing progress line: the three
/// fixed phases get their own string, anything else is a payload file path.
fn progress_message(tr: &Translator, label: &str) -> String {
    match label {
        "shortcuts" => tr.get("uninstall.removing_shortcuts"),
        "state" => tr.get("uninstall.removing_state"),
        "registry" => tr.get("uninstall.finalizing"),
        file => tr.fmt("uninstall.removing", &[("file", file)]),
    }
}

/// Core uninstall operations shared by the interactive and elevated-worker paths.
/// `on_step(done, total, label)` is called before each unit of work; label is a
/// file path for payload files, or "shortcuts" / "state" / "registry" for the
/// three fixed phases.
pub(crate) fn do_cleanup(
    info: &InstallInfo,
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
    info: &InstallInfo,
    manifest: &Manifest,
) -> Result<()> {
    // Same sequence as the interactive path; progress goes to the log.
    do_cleanup(info, manifest, app_dir, data_dir, |done, total, label| {
        common::log::info(format!("[{done}/{total}] {label}"));
    });
    spawn_finalize(Some(app_dir), data_dir, None, false)
}

fn run_elevated(progress: ui::Progress) -> anyhow::Result<()> {
    // No command to send: the elevated worker re-reads its own data dir.
    common::elevation::run_elevated_relay(None::<&()>, |done, total, name| {
        progress(done, total, name)
    })
    .map_err(|e| {
        if e.is::<common::elevation::UacCancelledError>() {
            anyhow::anyhow!("{}", ui::tr().get("uninstall.uac_cancelled"))
        } else {
            e
        }
    })
}

fn run_down_plugins(info: &InstallInfo, data_dir: &Path) {
    if info.plugins.is_empty() {
        return;
    }
    let items: Vec<_> = info
        .plugins
        .iter()
        .rev()
        .map(|p| (p.clone(), data_dir.join(&p.file), String::new()))
        .collect();
    let ctx = PluginContext::for_uninstall(info, data_dir);
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
    let self_exe = std::env::current_exe()?;
    let dest = staged_temp_path()?;
    // Retry past a transient AV scan of the freshly copied `.exe`; if this copy
    // fails outright, finalize never runs and the app/data dirs are never deleted.
    common::utils::copy_retry(&self_exe, &dest)
        .with_context(|| format!("copy finalize step to {}", dest.display()))?;

    Command::new(&dest)
        .args(finalize_args(
            app_dir,
            data_dir,
            &data_dir_key(data_dir),
            std::process::id(),
            display_name,
            show_complete,
        ))
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .with_context(|| format!("spawn {}", dest.display()))?;
    Ok(())
}

/// Command line for the finalize sub-command. `--display-name` and
/// `--show-complete` are only emitted together, when both a name and the flag
/// are present.
fn finalize_args(
    app_dir: Option<&Path>,
    data_dir: &Path,
    product: &str,
    parent_pid: u32,
    display_name: Option<&str>,
    show_complete: bool,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("finalize"),
        OsString::from("--data-dir"),
        data_dir.as_os_str().to_os_string(),
        OsString::from("--product"),
        OsString::from(product),
        OsString::from("--parent-pid"),
        OsString::from(parent_pid.to_string()),
    ];
    if let Some(dir) = app_dir {
        args.push(OsString::from("--app-dir"));
        args.push(dir.as_os_str().to_os_string());
    }
    if show_complete && let Some(name) = display_name {
        args.push(OsString::from("--display-name"));
        args.push(OsString::from(name));
        args.push(OsString::from("--show-complete"));
    }
    args
}

fn staged_temp_path() -> Result<PathBuf> {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "installway-uninstall-{}-{}.exe",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::i18n::Translator;

    fn args_of(v: &[OsString]) -> Vec<String> {
        v.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn finalize_args_full_form() {
        let a = finalize_args(
            Some(Path::new(r"C:\Apps\MyApp")),
            Path::new(r"C:\Data\MyApp"),
            "MyApp",
            42,
            Some("My App"),
            true,
        );
        assert_eq!(
            args_of(&a),
            vec![
                "finalize",
                "--data-dir",
                r"C:\Data\MyApp",
                "--product",
                "MyApp",
                "--parent-pid",
                "42",
                "--app-dir",
                r"C:\Apps\MyApp",
                "--display-name",
                "My App",
                "--show-complete",
            ]
        );
    }

    #[test]
    fn finalize_args_omits_app_dir_and_display_name() {
        let a = finalize_args(None, Path::new(r"C:\Data\MyApp"), "MyApp", 7, None, false);
        assert_eq!(
            args_of(&a),
            vec![
                "finalize",
                "--data-dir",
                r"C:\Data\MyApp",
                "--product",
                "MyApp",
                "--parent-pid",
                "7",
            ]
        );
    }

    /// `--show-complete` is never emitted without a name, and a name is never
    /// emitted without the flag.
    #[test]
    fn finalize_args_display_name_and_flag_travel_together() {
        let no_flag = finalize_args(
            None,
            Path::new(r"C:\Data\MyApp"),
            "MyApp",
            7,
            Some("My App"),
            false,
        );
        assert!(!args_of(&no_flag).iter().any(|a| a == "--display-name"));

        let no_name = finalize_args(None, Path::new(r"C:\Data\MyApp"), "MyApp", 7, None, true);
        assert!(!args_of(&no_name).iter().any(|a| a == "--show-complete"));
    }

    #[test]
    fn data_dir_key_is_the_folder_name() {
        assert_eq!(
            data_dir_key(Path::new(r"C:\Data\Pub\Uninstall\MyApp")),
            "MyApp"
        );
        assert_eq!(data_dir_key(Path::new(r"C:\")), "");
    }

    #[test]
    fn progress_message_maps_the_three_fixed_phases_and_files() {
        let tr = Translator::for_lang("en");
        // The three fixed phases each map to their own locale key.
        for (label, key) in [
            ("shortcuts", "uninstall.removing_shortcuts"),
            ("state", "uninstall.removing_state"),
            ("registry", "uninstall.finalizing"),
        ] {
            assert_eq!(progress_message(&tr, label), tr.get(key));
        }
        // Anything else is treated as a payload path and interpolated.
        assert_eq!(
            progress_message(&tr, "bin/a.exe"),
            tr.fmt("uninstall.removing", &[("file", "bin/a.exe")])
        );
    }
}
