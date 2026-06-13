// SPDX-License-Identifier: MIT
//! Example Installway plugin: silently uninstall a previous MSI before the new

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::process::Command;

const ABI_VERSION: u32 = 1;

/// EDIT THIS: The stable UpgradeCode of your legacy MSI application family.
/// It must include the curly braces `{}`.
const UPGRADE_CODE: &str = "{12021922-0000-0000-F000-1202192222AA}";

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

// Link directly against the Windows Installer system DLL
#[link(name = "msi")]
extern "system" {
    /// Native Windows API to enumerate products sharing an UpgradeCode.
    /// Returns 0 (ERROR_SUCCESS) if a product is found, or 259 (ERROR_NO_MORE_ITEMS).
    fn MsiEnumRelatedProductsW(
        lpUpgradeCode: *const u16,
        dwReserved: u32,
        iProductIndex: u32,
        lpProductBuf: *mut u16,
    ) -> u32;
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

unsafe fn log(ctx: *const InstallwayContext, level: &str, msg: &str) {
    if ctx.is_null() {
        return;
    }
    if let Some(cb) = unsafe { (*ctx).log } {
        cb(wide(level).as_ptr(), wide(msg).as_ptr());
    }
}

/// Uses raw FFI to query msi.dll and find the ProductCode associated with the UpgradeCode
fn find_product_code_by_upgrade_code(upgrade_code: &str) -> Option<String> {
    let wide_upgrade = wide(upgrade_code);

    // An MSI ProductCode GUID string is exactly 38 characters long.
    // We allocate 39 elements to accommodate the mandatory null-terminator.
    let mut product_buf = vec![0u16; 39];

    // Call the raw function from msi.dll
    let result = unsafe {
        MsiEnumRelatedProductsW(
            wide_upgrade.as_ptr(),
            0, // Reserved, must be 0
            0, // Index 0 to retrieve the first matching installed product
            product_buf.as_mut_ptr(),
        )
    };

    // 0 means ERROR_SUCCESS
    if result == 0 {
        if let Some(end) = product_buf.iter().position(|&c| c == 0) {
            if let Ok(code) = String::from_utf16(&product_buf[..end]) {
                return Some(code);
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
    unsafe {
        log(
            ctx,
            "INFO",
            &format!("Searching for existing installation via UpgradeCode: {UPGRADE_CODE}"),
        )
    };

    // Step 1: Dynamically resolve the ProductCode via raw FFI
    let product_code = match find_product_code_by_upgrade_code(UPGRADE_CODE) {
        Some(code) => code,
        None => {
            unsafe {
                log(
                    ctx,
                    "INFO",
                    "No previous MSI version detected on the system.",
                )
            };
            return 0; // Nothing to uninstall, proceed safely
        }
    };

    unsafe {
        log(
            ctx,
            "INFO",
            &format!("Previous version detected! Found ProductCode: {product_code}"),
        )
    };
    unsafe {
        log(
            ctx,
            "INFO",
            &format!("Running: msiexec /x {product_code} /qn"),
        )
    };

    // Step 2: Execute the uninstallation with the resolved ProductCode
    match Command::new("msiexec")
        .args(["/x", &product_code, "/qn", "/norestart"])
        .status()
    {
        Ok(s) => match s.code().unwrap_or(1) {
            // 0 = ok, 1605 = already uninstalled, 3010 = ok but reboot required.
            0 | 1605 | 3010 => {
                unsafe { log(ctx, "INFO", "Successfully uninstalled the legacy MSI.") };
                0
            }
            code => {
                unsafe {
                    log(
                        ctx,
                        "ERROR",
                        &format!("msiexec failed with exit code: {code}"),
                    )
                };
                code
            }
        },
        Err(e) => {
            unsafe { log(ctx, "ERROR", &format!("Failed to launch msiexec: {e}")) };
            1
        }
    }
}

#[no_mangle]
pub extern "system" fn installway_down(_ctx: *const InstallwayContext) -> i32 {
    0 // Not reversible
}
