// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result};
use common::models::{InstallInfo, InstallerPayload};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
) -> Result<()> {
    // Fall back to the app dir only if %LOCALAPPDATA% can't be resolved.
    let data_dir = common::paths::uninstall_dir(&payload.publisher, &payload.product)
        .unwrap_or_else(|| install_dir.to_path_buf());
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("create uninstall data dir {}", data_dir.display()))?;

    // Atomic + retrying write: a fresh `.exe` is the prime Defender trigger
    // (it locks the new file to scan it), so a bare write could fail the
    // install after every product file is already in place.
    let uninstaller_path = data_dir.join("uninstall.exe");
    common::utils::write_atomic(&uninstaller_path, uninstaller_bytes)
        .with_context(|| format!("write {}", uninstaller_path.display()))?;

    let key = registry_key_for(&payload.product);
    let info = InstallInfo {
        product: payload.product.clone(),
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
    };

    // Atomic writes: a half-written file would break uninstall / version checks.
    common::utils::write_atomic(
        &data_dir.join("installer_info.json"),
        serde_json::to_string_pretty(&info)?.as_bytes(),
    )?;
    common::utils::write_atomic(
        &data_dir.join("installer_manifest.json"),
        serde_json::to_string_pretty(&payload.manifest)?.as_bytes(),
    )?;
    common::utils::write_atomic(
        &data_dir.join("version.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "version": payload.to_version }))?
            .as_bytes(),
    )?;
    // Copy the live %TEMP% log next to the uninstaller for support.
    if let Some(src) = common::log::current_path() {
        let _ = fs::copy(&src, data_dir.join("install.log"));
    }

    register_uninstall(&info, &uninstaller_path)?;

    if !payload.manifest.exe.is_empty() {
        let target = install_dir.join(&payload.manifest.exe);
        create_shortcuts(&payload.product, install_dir, &target);

        if !payload.associations.is_empty() {
            // Normalize separators so the registry command reads cleanly.
            let exe_str = target.to_string_lossy().replace('/', "\\");
            common::assoc::register(&payload.product, &exe_str, &payload.associations);
        }
    }

    Ok(())
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

/// Sanitize product name for registry-key use.
fn registry_key_for(product: &str) -> String {
    product
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect()
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
        set_sz_logged(hkey, "InstallDate", &install_date_yyyymmdd(info.installed_at_unix));
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
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2)
    };
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

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // Days since 1970-01-01. Algorithm from civil_from_days (Howard Hinnant).
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
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
