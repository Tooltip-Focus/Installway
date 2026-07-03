// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "hintway")]
mod analytics;
mod elevation;
mod extract;
mod install;
mod payload;
mod proc;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::PCWSTR;

/// Exit code for a patch run against the wrong installed version. Distinct from
/// generic failure (1) so a launcher can tell the two apart.
const EXIT_VERSION_MISMATCH: i32 = 10;

#[derive(Parser)]
#[command(
    name = "installer",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct Cli {
    /// Target install directory (optional; used with `--silent` / `--minimal`).
    /// Falls back to `INSTALLWAY_PATH`, then the per-app default.
    install_dir: Option<String>,

    /// Silent (non-interactive) install.
    #[arg(long)]
    silent: bool,

    /// Internal: run as the elevated worker, streaming events back over the
    /// named pipe whose name is given here. Spawned via UAC by the main process.
    #[arg(long, hide = true, value_name = "PIPE")]
    elevated_worker: Option<String>,

    /// Internal: plugin-host child. Values are `<dll> <func> [pages_pipe]
    /// [progress_pipe]`; the context arrives on stdin. Re-launched as an
    /// isolated process so a crashing plugin can't stall the install.
    #[arg(long, hide = true, num_args = 2..=4, value_names = ["DLL", "FUNC", "PAGES", "PROGRESS"])]
    run_plugin: Option<Vec<String>>,

    /// Compact auto-update UI (icon + progress only).
    #[arg(long)]
    minimal: bool,

    /// Launch the product after a successful install.
    #[arg(long)]
    launch: bool,

    /// Verify the embedded payload + signature, print a summary, and exit.
    #[arg(long)]
    verify: bool,

    /// Re-hash the installed files against the recorded manifest, then exit.
    #[arg(long = "verify-install")]
    verify_install: bool,

    /// Override the UI language (2-letter ISO code, e.g. `fr`).
    #[arg(long)]
    lang: Option<String>,

    /// Dev-only: render one UI view with sample data, no payload needed
    /// (`license` | `choose` | `progress` | `done` | `error` | `minimal`).
    #[cfg(debug_assertions)]
    #[arg(long)]
    preview: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Diagnostic / headless modes report errors as text on the parent console
    // (+ a non-zero exit code) instead of a modal dialog, so CI and scripted
    // callers get a parseable result. Interactive (wizard / minimal) keeps the
    // modal.
    let console_mode = cli.silent || cli.verify || cli.verify_install;

    if let Err(e) = run(cli) {
        let code = if e.downcast_ref::<extract::VersionMismatch>().is_some() {
            EXIT_VERSION_MISMATCH
        } else {
            1
        };
        #[cfg(feature = "hintway")]
        {
            analytics::error(analytics::classify_error(&e));
            analytics::shutdown();
        }
        if console_mode {
            attach_console();
            eprintln!("Error: {e:#}");
        } else {
            report_fatal(&format!("{e:#}"));
        }
        std::process::exit(code);
    }
    #[cfg(feature = "hintway")]
    analytics::shutdown();
}

fn run(cli: Cli) -> Result<()> {
    // Elevated worker: connect to the main-process pipe, do the install +
    // finalize, stream events back. Spawned via UAC by the main process; needs
    // no payload/translator setup here (the worker resolves its own).
    if let Some(pipe_name) = cli.elevated_worker.as_deref() {
        let code = match crate::elevation::run_as_worker(pipe_name) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("elevated-worker error: {e:#}");
                1
            }
        };
        std::process::exit(code);
    }

    // Plugin-host child: load the DLL, call the function, exit with its code.
    // The context arrives on stdin; needs no payload. Values are
    // `<dll> <func> [pages_pipe] [progress_pipe]` (clap guarantees the first 2).
    if let Some(args) = cli.run_plugin.as_deref() {
        let code = common::plugin::host_main(
            Path::new(&args[0]),
            &args[1],
            args.get(2).filter(|s| !s.is_empty()).map(String::as_str),
            args.get(3).filter(|s| !s.is_empty()).map(String::as_str),
        );
        std::process::exit(code);
    }

    // `--lang` wins; otherwise env (`INSTALLWAY_LANG`) then OS locale.
    let translator = match &cli.lang {
        Some(code) => common::i18n::Translator::for_lang(code),
        None => common::i18n::Translator::detect(&[]),
    };
    // Record the resolved language process-wide so plugin contexts carry it.
    translator.set_global();

    // Dev-only: render a single UI view with sample data, no payload needed.
    // e.g. `installer --preview minimal`, `--preview license`.
    #[cfg(debug_assertions)]
    if let Some(view) = cli.preview.as_deref() {
        return if view == "minimal" {
            ui::minimal::preview(translator)
        } else {
            ui::win32::preview(view, translator)
        };
    }

    let loaded = payload::load_and_verify()?;
    let launch = cli.launch;

    // Determine analytics context now that payload is loaded (version + operation known).
    #[cfg(feature = "hintway")]
    let (hintway_version, hintway_operation, hintway_lang) = {
        let already = previous_install_dir(&loaded.payload).is_some();
        let op = match (&loaded.payload.kind, already) {
            (common::model::payload_kind::PayloadKind::Patch, _)
            | (common::model::payload_kind::PayloadKind::Full, true) => "update",
            _ => "install",
        };
        (loaded.payload.to_version.as_str(), op, translator.lang())
    };

    // Compact auto-start update UI (app-triggered self-update): no license,
    // path picker or buttons - just icon + progress.
    if cli.minimal {
        // Path resolved before `loaded` is moved into the UI: CLI positional,
        // then `INSTALLWAY_PATH`, then the per-app default.
        let path = resolve_install_path(cli.install_dir.as_deref(), &loaded.payload);
        #[cfg(feature = "hintway")]
        analytics::init(
            hintway_version,
            hintway_operation,
            "minimal",
            if common::paths::is_machine_location(&path) {
                "admin"
            } else {
                "user"
            },
            hintway_lang,
        );
        return ui::minimal::run(loaded, path, launch, translator);
    }

    if cli.silent {
        let path = resolve_install_path(cli.install_dir.as_deref(), &loaded.payload);
        #[cfg(feature = "hintway")]
        analytics::init(
            hintway_version,
            hintway_operation,
            "silent",
            if common::paths::is_machine_location(&path) {
                "admin"
            } else {
                "user"
            },
            hintway_lang,
        );
        return run_silent(loaded, path, launch, translator);
    }

    // Diagnostic: re-hash installed files against the manifest in the data dir.
    // The metadata lives in the machine-wide (%ProgramData%) or per-user
    // (%LOCALAPPDATA%) dir depending on how it was installed; check both.
    if cli.verify_install {
        attach_console();
        let data_dir = [true, false]
            .into_iter()
            .filter_map(|machine| {
                common::paths::uninstall_dir_for(
                    &loaded.payload.publisher,
                    &loaded.payload.product_id,
                    machine,
                )
            })
            .find(|d| d.join("installer_info.json").exists())
            .context("resolve data dir - is this product installed?")?;
        return extract::verify_install(&data_dir);
    }

    if cli.verify {
        attach_console();
        let license = match &loaded.payload.license_text {
            Some(t) => format!("custom ({} bytes)", t.len()),
            None => "built-in placeholder".to_string(),
        };
        println!(
            "OK: {} {} -> {} (payload {} bytes verified)\nLicense: {}",
            match loaded.payload.kind {
                common::model::payload_kind::PayloadKind::Full => "FULL",
                common::model::payload_kind::PayloadKind::Patch => "PATCH",
            },
            loaded
                .payload
                .from_version
                .clone()
                .unwrap_or_else(|| "(fresh)".to_string()),
            loaded.payload.to_version,
            loaded.zip().len(),
            license,
        );
        return Ok(());
    }

    let prior = previous_install_dir(&loaded.payload);
    let already_installed = prior.is_some();
    // cli.install_dir is set when the wizard re-launches itself as admin so
    // the elevated process pre-fills the path the user originally chose.
    let default_path = cli
        .install_dir
        .as_deref()
        .map(PathBuf::from)
        .or(prior)
        .unwrap_or_else(|| default_install_path(&loaded.payload));

    // Opt-in: an upgrade over an existing install uses the compact UI when the
    // new payload asks for it. First install always uses the full wizard.
    if already_installed && loaded.payload.upgrade_minimal_ui {
        return ui::minimal::run(loaded, default_path, launch, translator);
    }

    #[cfg(feature = "hintway")]
    analytics::init(
        hintway_version,
        hintway_operation,
        "interactive",
        "unknown",
        hintway_lang,
    );

    // Save before `loaded` is moved into the UI call below.
    #[cfg(feature = "hintway")]
    let loaded_publisher = loaded.payload.publisher.clone();
    #[cfg(feature = "hintway")]
    let loaded_product_id = loaded.payload.product_id.clone();

    // Extract any `ui = true` plugin DLLs for the wizard to query step by step.
    let self_exe = std::env::current_exe()?;
    let ui_plugins =
        extract::extract_ui_plugins(&loaded.payload, &default_path, &self_exe, loaded.zip());

    ui::win32::run(
        loaded,
        default_path,
        launch,
        already_installed,
        translator,
        ui_plugins,
    )?;

    // Privilege is determined inside the UI (path chosen by user + UAC).
    // Read it from the written installer_info.json so app_exit carries the real value.
    #[cfg(feature = "hintway")]
    if let Some(info_path) = [true, false].into_iter().find_map(|machine| {
        common::paths::uninstall_dir_for(&loaded_publisher, &loaded_product_id, machine)
            .map(|d| d.join("installer_info.json"))
            .filter(|p| p.exists())
    }) {
        if let Ok(text) = std::fs::read_to_string(&info_path) {
            if let Ok(info) =
                serde_json::from_str::<common::model::install_info::InstallInfo>(&text)
            {
                analytics::set_privilege(info.requires_admin);
            }
        }
    }

    Ok(())
}

/// Attach to the parent console so output from this GUI-subsystem binary is
/// visible.
fn attach_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn run_silent(
    mut loaded: payload::LoadedPayload,
    install_dir: PathBuf,
    launch: bool,
    translator: common::i18n::Translator,
) -> Result<()> {
    attach_console();
    println!(
        "Silent install: {} {} -> {}",
        loaded.payload.product,
        loaded.payload.to_version,
        install_dir.display()
    );
    let progress = Arc::new(|done: u64, total: u64, name: &str| {
        if let Some(pct) = (done * 100).checked_div(total) {
            eprintln!("[{:>3}%] {}", pct, name);
        }
    }) as common::ProgressFn;

    // No interactive UI: plugin pages fall back to their declared defaults.
    let plugin_inputs = ui::headless_plugin_inputs(&loaded, &install_dir)?;

    // Machine-wide iff the target is a shared location (Program Files, etc.).
    // Silent runs have no auto-elevation, so location is the only signal.
    let requires_admin = common::paths::is_machine_location(&install_dir);
    // Resolve feature packs from the headless answers and filter the manifest.
    extract::resolve_and_filter(&mut loaded, &install_dir, requires_admin, &plugin_inputs);
    let ctx = extract::InstallCtx {
        install_dir: install_dir.clone(),
        payload: &loaded.payload,
        zip_bytes: loaded.zip(),
        cancel: Arc::new(AtomicBool::new(false)),
        on_progress: progress,
        plugin_inputs: plugin_inputs.clone(),
        requires_admin,
        hwnd_parent: 0,
        translator,
    };
    #[cfg(feature = "hintway")]
    analytics::stage("extract");
    // Lock held across finalize so a concurrent run can't interleave.
    let _install_lock = extract::install(ctx)?;
    #[cfg(feature = "hintway")]
    analytics::stage("finalize");
    install::finalize(
        &install_dir,
        &loaded.payload,
        &loaded.uninstaller_bytes,
        loaded.zip(),
        &plugin_inputs,
        requires_admin,
    )?;
    #[cfg(feature = "hintway")]
    analytics::stage("done");

    if launch && !loaded.payload.manifest.exe.is_empty() {
        install::launch_product(&install_dir, &loaded.payload.manifest.exe)?;
        println!("Launched {}", loaded.payload.manifest.exe);
    }
    println!("Done.");
    Ok(())
}

/// Resolve the target dir for headless / compact modes: explicit CLI value,
/// then `INSTALLWAY_PATH`, then the per-app default.
fn resolve_install_path(
    cli_path: Option<&str>,
    payload: &common::model::installer_payload::InstallerPayload,
) -> PathBuf {
    if let Some(p) = cli_path {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("INSTALLWAY_PATH") {
        return PathBuf::from(p);
    }
    default_install_path(payload)
}

fn default_install_path(payload: &common::model::installer_payload::InstallerPayload) -> PathBuf {
    // Already installed? Propose the same folder so a reinstall/update lands in
    // place (the user can still change it on the Choose page).
    if let Some(prev) = previous_install_dir(payload) {
        return prev;
    }
    // Per-app default from the build (env tokens expanded), if set.
    if let Some(dir) = payload.default_install_dir.as_deref() {
        let expanded = common::utils::expand_env(dir);
        let trimmed = expanded.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    // Else a user-local path, no admin needed.
    let product = &payload.product;
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Programs").join(product);
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home).join(product);
    }
    PathBuf::from(format!(r"C:\Users\Public\{}", product))
}

/// The folder this product was last installed to, or `None` if never installed
/// or the recorded path is empty.
fn previous_install_dir(
    payload: &common::model::installer_payload::InstallerPayload,
) -> Option<PathBuf> {
    let (_, info) = extract::prior_install_info(payload)?;
    (!info.install_dir.trim().is_empty()).then(|| PathBuf::from(info.install_dir))
}

fn report_fatal(msg: &str) {
    let text = common::utils::wide(msg);
    let cap = common::utils::wide("Installer error");
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(cap.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
