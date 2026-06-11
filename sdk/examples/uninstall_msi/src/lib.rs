// SPDX-License-Identifier: MIT
//! Example Installway plugin: silently uninstall a previous MSI before the new
//! install (tech switch). `up` runs `msiexec /x <code> /qn`. Uninstalling an
//! MSI isn't reversible, so `down` is a no-op.
//!
//! Declare it in pack.toml:
//!   [[plugin]]
//!   name  = "uninstall-old-msi"
//!   dll   = "plugins/uninstall_old_msi.dll"
//!   phase = "pre-install"

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::process::Command;

const ABI_VERSION: u32 = 1;

/// EDIT THIS: the ProductCode (or {UpgradeCode}) of the MSI to remove.
const PRODUCT_CODE: &str = "{00000000-0000-0000-0000-000000000000}";

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
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

unsafe fn log(ctx: *const InstallwayContext, level: &str, msg: &str) {
    if ctx.is_null() {
        return;
    }
    if let Some(cb) = unsafe { (*ctx).log } {
        cb(wide(level).as_ptr(), wide(msg).as_ptr());
    }
}

#[no_mangle]
pub extern "system" fn installway_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "system" fn installway_up(ctx: *const InstallwayContext) -> i32 {
    unsafe { log(ctx, "INFO", &format!("msiexec /x {PRODUCT_CODE} /qn")) };
    match Command::new("msiexec")
        .args(["/x", PRODUCT_CODE, "/qn", "/norestart"])
        .status()
    {
        Ok(s) => match s.code().unwrap_or(1) {
            // 0 = ok, 1605 = not installed (fine), 3010 = ok but reboot needed.
            0 | 1605 | 3010 => 0,
            code => {
                unsafe { log(ctx, "ERROR", &format!("msiexec exited {code}")) };
                code
            }
        },
        Err(e) => {
            unsafe { log(ctx, "ERROR", &format!("failed to run msiexec: {e}")) };
            1
        }
    }
}

#[no_mangle]
pub extern "system" fn installway_down(_ctx: *const InstallwayContext) -> i32 {
    0 // not reversible
}
