// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod extract;
mod install;
mod payload;
mod proc;
mod ui;

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Exit code for a patch run against the wrong installed version. Distinct from
/// generic failure (1) so a launcher can tell the two apart.
const EXIT_VERSION_MISMATCH: i32 = 10;

fn main() {
    if let Err(e) = run() {
        let code = if e.downcast_ref::<extract::VersionMismatch>().is_some() {
            EXIT_VERSION_MISMATCH
        } else {
            1
        };
        report_fatal(&format!("{e:#}"));
        std::process::exit(code);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let translator = common::i18n::Translator::detect(&args);

    // Dev-only: render a single UI view with sample data, no payload needed.
    // e.g. `installer --preview minimal`, `--preview license`, `--preview progress`.
    #[cfg(debug_assertions)]
    if let Some(idx) = args.iter().position(|a| a == "--preview") {
        let view = args.get(idx + 1).map(|s| s.as_str()).unwrap_or("license");
        return if view == "minimal" {
            ui::minimal::preview(translator)
        } else {
            ui::win32::preview(view, translator)
        };
    }

    let loaded = payload::load_and_verify()?;
    let launch = args.iter().any(|a| a == "--launch");

    // Compact auto-start update UI (app-triggered self-update): no license,
    // path picker or buttons - just icon + progress.
    if let Some(idx) = args.iter().position(|a| a == "--minimal" || a == "/minimal") {
        let path = path_arg(&args, idx)
            .or_else(|| std::env::var("INSTALLWAY_PATH").ok())
            .unwrap_or_else(|| default_install_path(&loaded.payload).to_string_lossy().into_owned());
        return ui::minimal::run(loaded, PathBuf::from(path), launch, translator);
    }

    if let Some(idx) = args.iter().position(|a| a == "--silent" || a == "/S") {
        let path = path_arg(&args, idx)
            .or_else(|| std::env::var("INSTALLWAY_PATH").ok())
            .unwrap_or_else(|| default_install_path(&loaded.payload).to_string_lossy().into_owned());
        return run_silent(&loaded, PathBuf::from(path), launch);
    }
    // Diagnostic: re-hash installed files against the manifest in the data dir.
    if args.iter().any(|a| a == "--verify-install") {
        attach_console();
        let data_dir = common::paths::uninstall_dir(
            &loaded.payload.publisher,
            &loaded.payload.product,
        )
        .context("resolve data dir")?;
        return extract::verify_install(&data_dir);
    }

    if args.iter().any(|a| a == "--verify") {
        attach_console();
        let license = match &loaded.payload.license_text {
            Some(t) => format!("custom ({} bytes)", t.len()),
            None => "built-in placeholder".to_string(),
        };
        println!(
            "OK: {} {} -> {} (payload {} bytes verified)\nLicense: {}",
            match loaded.payload.kind {
                common::models::PayloadKind::Full => "FULL",
                common::models::PayloadKind::Patch => "PATCH",
            },
            loaded.payload.from_version.clone().unwrap_or_else(|| "(fresh)".to_string()),
            loaded.payload.to_version,
            loaded.zip().len(),
            license,
        );
        return Ok(());
    }

    let prior = previous_install_dir(&loaded.payload);
    let already_installed = prior.is_some();
    let default_path = prior.unwrap_or_else(|| default_install_path(&loaded.payload));
    ui::win32::run(loaded, default_path, launch, already_installed, translator)?;
    Ok(())
}

/// Attach to the parent console so output from this GUI-subsystem binary is
/// visible.
fn attach_console() {
    unsafe {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn run_silent(
    loaded: &payload::LoadedPayload,
    install_dir: PathBuf,
    launch: bool,
) -> Result<()> {
    attach_console();
    println!(
        "Silent install: {} {} -> {}",
        loaded.payload.product, loaded.payload.to_version, install_dir.display()
    );
    let progress = Arc::new(|done: u64, total: u64, name: &str| {
        if total > 0 {
            let pct = (done * 100) / total;
            eprintln!("[{:>3}%] {}", pct, name);
        }
    }) as Arc<dyn Fn(u64, u64, &str) + Send + Sync>;

    let ctx = extract::InstallCtx {
        install_dir: install_dir.clone(),
        payload: &loaded.payload,
        zip_bytes: loaded.zip(),
        cancel: Arc::new(AtomicBool::new(false)),
        on_progress: progress,
    };
    extract::install(ctx)?;
    install::finalize(&install_dir, &loaded.payload, &loaded.uninstaller_bytes)?;

    if launch && !loaded.payload.manifest.exe.is_empty() {
        install::launch_product(&install_dir, &loaded.payload.manifest.exe)?;
        println!("Launched {}", loaded.payload.manifest.exe);
    }
    println!("Done.");
    Ok(())
}

/// The value right after a flag, unless it's another flag.
fn path_arg(args: &[String], flag_idx: usize) -> Option<String> {
    args.get(flag_idx + 1)
        .filter(|s| !s.starts_with("--") && !s.starts_with('/'))
        .cloned()
}

fn default_install_path(payload: &common::models::InstallerPayload) -> PathBuf {
    // Already installed? Propose the same folder so a reinstall/update lands in
    // place (the user can still change it on the Choose page).
    if let Some(prev) = previous_install_dir(payload) {
        return prev;
    }
    // Per-app default from the build (env tokens expanded), if set.
    if let Some(dir) = payload.default_install_dir.as_deref() {
        let expanded = expand_env(dir);
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

/// The folder this product was last installed to, read from `installer_info.json`
/// in the per-user data dir. `None` if never installed or the record is missing
/// / empty.
fn previous_install_dir(payload: &common::models::InstallerPayload) -> Option<PathBuf> {
    let data_dir = common::paths::uninstall_dir(&payload.publisher, &payload.product)?;
    let text = std::fs::read_to_string(data_dir.join("installer_info.json")).ok()?;
    let info: common::models::InstallInfo = serde_json::from_str(&text).ok()?;
    if info.install_dir.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(info.install_dir))
    }
}

/// Expand `%VAR%` tokens via Win32 (handles `%LOCALAPPDATA%` etc.). Returns the
/// input unchanged on failure.
fn expand_env(s: &str) -> String {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::PCWSTR;
    let src: Vec<u16> = std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect();
    let needed = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), None) };
    if needed == 0 {
        return s.to_string();
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) };
    if written == 0 {
        return s.to_string();
    }
    let n = (written as usize).saturating_sub(1).min(buf.len()); // drop trailing null
    String::from_utf16_lossy(&buf[..n])
}

fn report_fatal(msg: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::PCWSTR;
    let text: Vec<u16> = OsStr::new(msg).encode_wide().chain(std::iter::once(0)).collect();
    let cap: Vec<u16> = OsStr::new("Installer error")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(cap.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
