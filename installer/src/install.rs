// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result};
use common::models::{FileAssoc, InstallInfo, InstallerPayload, PluginPhase};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use common::utils::days_to_ymd;

/// Write the uninstaller + metadata to a per-user data folder outside the app
/// directory and register the product under HKCU Uninstall.
///
/// Keeping the uninstaller + metadata in `%LOCALAPPDATA%\<publisher>\Uninstall\
/// <product>` means deleting the app folder by hand never orphans the
/// Add/Remove entry.
pub fn finalize(
    install_dir: &Path,
    payload: &InstallerPayload,
    uninstaller_bytes: &[u8],
    zip_bytes: &[u8],
) -> Result<()> {
    // Data dir is keyed by the registry-safe product_id (stable across
    // versions). Fall back to the app dir only if %LOCALAPPDATA% can't resolve.
    let data_dir = common::paths::uninstall_dir(&payload.publisher, &payload.product_id)
        .unwrap_or_else(|| install_dir.to_path_buf());
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("create uninstall data dir {}", data_dir.display()))?;

    // Prior install record, read BEFORE we overwrite installer_info.json, so we
    // can drop the associations / registry entries this version no longer
    // declares (otherwise they orphan and even survive uninstall).
    let prior: Option<InstallInfo> = fs::read_to_string(data_dir.join("installer_info.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let prior_assocs: Vec<FileAssoc> = prior
        .as_ref()
        .map(|i| i.associations.clone())
        .unwrap_or_default();
    let prior_reg = prior.map(|i| i.registry).unwrap_or_default();

    // Resolve the registry token templates against this install.
    let registry = expand_registry(payload, install_dir);

    // Atomic + retrying write: a fresh `.exe` is the prime Defender trigger
    // (it locks the new file to scan it), so a bare write could fail the
    // install after every product file is already in place.
    let uninstaller_path = data_dir.join("uninstall.exe");
    common::utils::write_atomic(&uninstaller_path, uninstaller_bytes)
        .with_context(|| format!("write {}", uninstaller_path.display()))?;

    // The HKCU Uninstall subkey IS the product_id (validated registry-safe at
    // build time) — no on-the-fly sanitization of the display name.
    let key = payload.product_id.clone();
    let info = InstallInfo {
        product: payload.product.clone(),
        product_id: payload.product_id.clone(),
        publisher: payload.publisher.clone(),
        version: payload.to_version.clone(),
        install_dir: install_dir.to_string_lossy().into_owned(),
        installed_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default(),
        registry_key: key.clone(),
        exe: payload.manifest.exe.clone(),
        associations: payload.associations.clone(),
        registry: registry.clone(),
        plugins: payload.plugins.clone(),
    };

    // Extract the plugin DLLs into the data dir so the uninstaller (and the
    // post-install phase below) can run them.
    write_plugin_dlls(&data_dir, payload, zip_bytes)?;

    // Register the Add/Remove Programs entry. Uses the in-memory `info`, not
    // the json file, so the json can safely be written last.
    register_uninstall(&info, &uninstaller_path)?;

    // Registry / shortcut side effects, performed BEFORE the state files are
    // written. Reconcile associations: drop any the previous install
    // registered that this version no longer declares (so a changed list never
    // orphans ProgIDs / extension handlers), then (re)register the current set.
    // Runs unconditionally (even if the new set is empty or there's no exe).
    let stale = common::assoc::stale(&prior_assocs, &payload.associations);
    if !stale.is_empty() {
        common::assoc::unregister(&payload.product_id, &stale);
    }
    if !payload.manifest.exe.is_empty() {
        let target = install_dir.join(&payload.manifest.exe);
        // Shortcut label is the display name.
        create_shortcuts(&payload.product, install_dir, &target);

        if !payload.associations.is_empty() {
            // ProgIDs are keyed by product_id. Normalize separators so the
            // registry command reads cleanly.
            let exe_str = target.to_string_lossy().replace('/', "\\");
            common::assoc::register(&payload.product_id, &exe_str, &payload.associations);
        }
    }

    // Free-form registry: drop entries the previous install wrote but this
    // version no longer declares, then (re)write the current set.
    for e in common::registry::stale(&prior_reg, &registry) {
        common::registry::remove_if_ours(&e);
    }
    for e in &registry {
        common::registry::write(e);
    }

    // State files written LAST. `installer_info.json` is the durable completion
    // marker: until it is (re)written it still holds the PREVIOUS association
    // set, so a crash anywhere above leaves a re-run able to recompute the stale
    // set correctly and self-heal. Atomic writes: a half-written file would
    // break uninstall / version checks.
    common::utils::write_atomic(
        &data_dir.join("installer_manifest.json"),
        serde_json::to_string_pretty(&payload.manifest)?.as_bytes(),
    )?;
    common::utils::write_atomic(
        &data_dir.join("version.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "version": payload.to_version }))?
            .as_bytes(),
    )?;
    common::utils::write_atomic(
        &data_dir.join("installer_info.json"),
        serde_json::to_string_pretty(&info)?.as_bytes(),
    )?;
    // Post-install plugins run last, from the data dir, with everything in
    // place and recorded.
    run_post_install_plugins(&data_dir, payload, install_dir)?;

    // Copy the live %TEMP% log next to the uninstaller for support.
    if let Some(src) = common::log::current_path() {
        let _ = fs::copy(&src, data_dir.join("install.log"));
    }

    Ok(())
}

/// Extract every plugin DLL from the payload zip into `<data_dir>/plugins/`.
fn write_plugin_dlls(data_dir: &Path, payload: &InstallerPayload, zip_bytes: &[u8]) -> Result<()> {
    if payload.plugins.is_empty() {
        return Ok(());
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .context("open payload zip for plugins")?;
    for p in &payload.plugins {
        let mut buf = Vec::new();
        {
            let mut f = archive
                .by_name(&p.file)
                .with_context(|| format!("plugin {} missing from payload", p.file))?;
            std::io::Read::read_to_end(&mut f, &mut buf)?;
        }
        // `p.file` is `plugins/<name>.dll`, relative to the data dir.
        common::utils::write_atomic(&data_dir.join(&p.file), &buf)?;
    }
    Ok(())
}

/// Run the post-install plugins (from the data dir) in isolated child processes.
fn run_post_install_plugins(
    data_dir: &Path,
    payload: &InstallerPayload,
    install_dir: &Path,
) -> Result<()> {
    let items: Vec<_> = payload
        .plugins
        .iter()
        .filter(|p| p.phase == PluginPhase::PostInstall)
        .map(|p| (p.clone(), data_dir.join(&p.file)))
        .collect();
    if items.is_empty() {
        return Ok(());
    }
    let pctx = crate::extract::plugin_ctx(payload, install_dir);
    let ctx_path = common::plugin::write_ctx(&pctx)?;
    let self_exe = std::env::current_exe()?;
    let res = common::plugin::run_each(&self_exe, &ctx_path, &items, "up", true);
    let _ = fs::remove_file(&ctx_path);
    res
}

/// Resolve token templates in each registry entry against this install.
/// Tokens: `%APP_KEY%` (= `Software\<publisher>\<product_id>`), `%INSTALL_DIR%`,
/// `%EXE%`, `%VERSION%`, `%PRODUCT%`, `%PRODUCT_ID%`, `%PUBLISHER%`. The
/// publisher is sanitized so it stays a single registry-key component.
fn expand_registry(
    payload: &InstallerPayload,
    install_dir: &Path,
) -> Vec<common::models::RegEntry> {
    use common::models::{RegEntry, RegValue};
    let pub_s = common::paths::sanitize_component(&payload.publisher);
    let app_key = format!(r"Software\{}\{}", pub_s, payload.product_id);
    let exe = install_dir
        .join(&payload.manifest.exe)
        .to_string_lossy()
        .replace('/', "\\");
    let install = install_dir.to_string_lossy().replace('/', "\\");
    let sub = |s: &str| {
        s.replace("%APP_KEY%", &app_key)
            .replace("%INSTALL_DIR%", &install)
            .replace("%EXE%", &exe)
            .replace("%VERSION%", &payload.to_version)
            .replace("%PRODUCT_ID%", &payload.product_id)
            .replace("%PRODUCT%", &payload.product)
            .replace("%PUBLISHER%", &pub_s)
    };
    payload
        .registry
        .iter()
        .map(|e| RegEntry {
            hive: e.hive.clone(),
            key: sub(&e.key),
            name: sub(&e.name),
            kind: e.kind,
            value: match &e.value {
                RegValue::Text(s) => RegValue::Text(sub(s)),
                RegValue::List(v) => RegValue::List(v.iter().map(|s| sub(s)).collect()),
                RegValue::Int(n) => RegValue::Int(*n),
            },
        })
        .collect()
}

/// Drop a desktop and Start Menu shortcut pointing at the installed exe.
/// Best effort: a failed shortcut must not fail the install, but failures are
/// logged so support can tell why a shortcut is missing.
pub fn create_shortcuts(product: &str, install_dir: &Path, target: &Path) {
    for path in common::shortcuts::paths_for(product) {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                common::log::warn(format!(
                    "shortcut: could not create folder {}: {e}",
                    parent.display()
                ));
                continue;
            }
        }
        match mslnk::ShellLink::new(target.to_string_lossy().as_ref()) {
            Ok(mut lnk) => {
                lnk.set_working_dir(Some(install_dir.to_string_lossy().into_owned()));
                match lnk.create_lnk(&path) {
                    Ok(()) => common::log::info(format!("shortcut created: {}", path.display())),
                    Err(e) => common::log::warn(format!(
                        "shortcut: could not write {}: {e}",
                        path.display()
                    )),
                }
            }
            Err(e) => common::log::warn(format!(
                "shortcut: could not build link to {}: {e}",
                target.display()
            )),
        }
    }
}

fn register_uninstall(info: &InstallInfo, uninstaller_path: &Path) -> Result<()> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, RegCloseKey, RegCreateKeyExW,
    };
    use windows::core::PCWSTR;

    let sub = format!(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}",
        info.registry_key
    );
    let sub_w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let mut hkey = HKEY::default();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if rc.is_err() {
            anyhow::bail!("RegCreateKeyEx failed: {:?}", rc);
        }

        set_sz_logged(hkey, "DisplayName", &info.product);
        set_sz_logged(hkey, "DisplayVersion", &info.version);
        set_sz_logged(
            hkey,
            "UninstallString",
            &format!("\"{}\"", uninstaller_path.display()),
        );
        set_sz_logged(
            hkey,
            "QuietUninstallString",
            &format!("\"{}\" --silent", uninstaller_path.display()),
        );
        set_sz_logged(hkey, "InstallLocation", &info.install_dir);
        set_sz_logged(hkey, "Publisher", &info.publisher);
        set_sz_logged(
            hkey,
            "InstallDate",
            &install_date_yyyymmdd(info.installed_at_unix),
        );
        set_sz_logged(hkey, "DisplayIcon", &uninstaller_path.to_string_lossy());
        set_sz_logged(hkey, "NoModify", "1");
        set_sz_logged(hkey, "NoRepair", "1");

        let _ = RegCloseKey(hkey);
    }
    Ok(())
}

unsafe fn set_sz(
    hkey: windows::Win32::System::Registry::HKEY,
    name: &str,
    value: &str,
) -> Result<()> {
    use windows::Win32::System::Registry::{REG_SZ, RegSetValueExW};
    use windows::core::PCWSTR;
    let n: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let v: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    let rc = unsafe { RegSetValueExW(hkey, PCWSTR(n.as_ptr()), None, REG_SZ, Some(bytes)) };
    if rc.is_err() {
        anyhow::bail!("RegSetValueEx({}) failed: {:?}", name, rc);
    }
    Ok(())
}

/// `set_sz`, but logs (instead of silently dropping) a failure to write one
/// Add/Remove Programs field. One missing field shouldn't abort registration -
/// but a support engineer staring at a half-empty entry needs to know why.
fn set_sz_logged(hkey: windows::Win32::System::Registry::HKEY, name: &str, value: &str) {
    if let Err(e) = unsafe { set_sz(hkey, name, value) } {
        common::log::warn(format!("registry: could not set {name}: {e:#}"));
    }
}

fn install_date_yyyymmdd(unix: i64) -> String {
    // crude UTC date conversion (no chrono dependency).
    let days = unix / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{:04}{:02}{:02}", y, m, d)
}

pub fn launch_product(install_dir: &Path, exe_rel: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    if exe_rel.trim().is_empty() {
        return Ok(());
    }
    let full = install_dir.join(exe_rel);
    let path_w: Vec<u16> = std::ffi::OsStr::new(&full)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dir_w: Vec<u16> = std::ffi::OsStr::new(install_dir)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let op: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR(dir_w.as_ptr()),
            SW_SHOWNORMAL,
        );
    }
    Ok(())
}
