// SPDX-License-Identifier: MIT
//! Example Installway plugin: silently uninstall a previous InstallShield
//! product before the new install. `up` finds the product in the registry by a
//! DisplayName substring and runs its `UninstallString` silently. `down` is a
//! no-op (not reversible).
//!
//! Declare it in pack.toml:
//!   [[plugin]]
//!   name  = "uninstall-old-installshield"
//!   dll   = "plugins/uninstall_old_is.dll"
//!   phase = "pre-install"

use std::process::Command;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

const ABI_VERSION: u32 = 1;
const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

/// EDIT THIS: a substring of the product's Add/Remove Programs DisplayName.
const DISPLAY_NAME_MATCH: &str = "My Legacy App";

#[repr(C)]
pub struct InstallwayContext {
    abi_version: u32,
    install_dir: *const u16,
    product: *const u16,
    product_id: *const u16,
    version: *const u16,
    exe: *const u16,
    log: Option<extern "system" fn(*const u16, *const u16)>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn log(ctx: *const InstallwayContext, level: &str, msg: &str) {
    if ctx.is_null() {
        return;
    }
    if let Some(cb) = unsafe { (*ctx).log } {
        cb(wide(level).as_ptr(), wide(msg).as_ptr());
    }
}

/// First `UninstallString` whose `DisplayName` contains `DISPLAY_NAME_MATCH`.
fn find_uninstall_string() -> Option<String> {
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Ok(base) = RegKey::predef(hive).open_subkey(UNINSTALL) else {
            continue;
        };
        for sub in base.enum_keys().flatten() {
            let Ok(k) = base.open_subkey(&sub) else {
                continue;
            };
            let Ok(name) = k.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if name.contains(DISPLAY_NAME_MATCH) {
                if let Ok(uninstall_str) = k.get_value::<String, _>("UninstallString") {
                    return Some(uninstall_str);
                }
            }
        }
    }
    None
}

#[no_mangle]
pub extern "system" fn installway_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "system" fn installway_up(ctx: *const InstallwayContext) -> i32 {
    let Some(uninstall_str) = find_uninstall_string() else {
        unsafe {
            log(
                ctx,
                "INFO",
                "InstallShield product not found - nothing to do",
            )
        };
        return 0; // nothing to remove is success
    };

    // InstallShield silent: append `/s` (and a quiet MSI transform). Real
    // UninstallStrings vary; adjust the flags for your product if needed.
    let cmd = format!("{uninstall_str} /s /v\"/qn\"");
    unsafe { log(ctx, "INFO", &cmd) };
    match Command::new("cmd").args(["/C", &cmd]).status() {
        Ok(s) => match s.code().unwrap_or(1) {
            0 | 3010 => 0,
            code => {
                unsafe { log(ctx, "ERROR", &format!("uninstaller exited {code}")) };
                code
            }
        },
        Err(e) => {
            unsafe { log(ctx, "ERROR", &format!("failed to start uninstaller: {e}")) };
            1
        }
    }
}

#[no_mangle]
pub extern "system" fn installway_down(_ctx: *const InstallwayContext) -> i32 {
    0 // not reversible
}
