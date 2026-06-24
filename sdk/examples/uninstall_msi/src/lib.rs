// SPDX-License-Identifier: MIT
//! Example Installway plugin: silently uninstall a previous MSI before the new
//! install. Emits a `buttons: false` page with a marquee progress bar;
//! the host auto-runs `installway_up` while that screen is shown.

use std::process::Command;
use widestring::U16CString;

const ABI_VERSION: u32 = 1;

/// EDIT THIS: The stable UpgradeCode of your legacy MSI application family.
/// It must include the curly braces `{}`.
const UPGRADE_CODE: &str = "{12021922-0000-0000-F000-1202192222AA}";

#[repr(C)]
pub struct InstallwayContext {
    abi_version: u32,
    install_dir: *const u16,
    data_dir: *const u16,
    product: *const u16,
    product_id: *const u16,
    version: *const u16,
    exe: *const u16,
    log: Option<extern "system" fn(*const u16, *const u16)>,
    inputs_json: *const u16,
    emit_pages: Option<extern "system" fn(*const u16)>,
    /// Call with a 0–100 value to drive a deterministic progress bar.
    /// Null when the page uses `marquee: true` (infinite bar).
    emit_progress: Option<extern "system" fn(u32)>,
    /// Host UI language code (e.g. L"en", L"fr"). Never null; empty when the host
    /// resolved none. Use it to localize the page strings to match the installer.
    lang: *const u16,
}

#[link(name = "msi")]
extern "system" {
    /// Enumerate installed products sharing an UpgradeCode.
    /// Returns 0 (ERROR_SUCCESS) or 259 (ERROR_NO_MORE_ITEMS).
    fn MsiEnumRelatedProductsW(
        upgrade_code: *const u16,
        reserved: u32,
        product_index: u32,
        product_buf: *mut u16,
    ) -> u32;
}

/// Read a null-terminated wide string from the context; empty on null.
unsafe fn from_wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

/// The host UI language code (e.g. "fr"). A custom i18n: each plugin ships its
/// own strings and selects them by this code, so the page matches the installer
/// (including its `--lang` / `INSTALLWAY_LANG` overrides). Unknown → English.
unsafe fn ctx_lang(ctx: *const InstallwayContext) -> String {
    if ctx.is_null() {
        return String::new();
    }
    unsafe { from_wide((*ctx).lang) }
}

unsafe fn log(ctx: *const InstallwayContext, level: &str, msg: &str) {
    if ctx.is_null() {
        return;
    }
    if let Some(cb) = unsafe { (*ctx).log } {
        cb(
            U16CString::from_str_truncate(level).as_ptr(),
            U16CString::from_str_truncate(msg).as_ptr(),
        );
    }
}

fn find_product_code_by_upgrade_code(upgrade_code: &str) -> Option<String> {
    let upgrade_code_w = U16CString::from_str_truncate(upgrade_code);
    // MSI ProductCode GUID is 38 chars + null terminator = 39 elements.
    let mut product_buf = vec![0u16; 39];
    let rc =
        unsafe { MsiEnumRelatedProductsW(upgrade_code_w.as_ptr(), 0, 0, product_buf.as_mut_ptr()) };
    if rc == 0 {
        let end = product_buf.iter().position(|&c| c == 0)?;
        String::from_utf16(&product_buf[..end]).ok()
    } else {
        None
    }
}

#[no_mangle]
pub extern "system" fn installway_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "system" fn installway_pages(ctx: *const InstallwayContext) -> i32 {
    if ctx.is_null() {
        return 1;
    }
    // Skip the uninstall page entirely when no previous MSI is installed.
    if find_product_code_by_upgrade_code(UPGRADE_CODE).is_none() {
        return 0;
    }
    // Localize the page to the host language. Strings contain no `"` or `\`, so
    // they need no JSON escaping when interpolated below.
    let (title, subtitle) = match unsafe { ctx_lang(ctx) }.as_str() {
        "fr" => (
            "Suppression de la version précédente",
            "Désinstallation de l'ancien MSI — cela peut prendre un moment.",
        ),
        _ => (
            "Removing previous version",
            "Uninstalling the legacy MSI — this may take a moment.",
        ),
    };
    match unsafe { (*ctx).emit_pages } {
        Some(emit) => {
            let json = format!(
                r#"{{"step":"page","page":{{"id":"uninstall","title":"{title}","subtitle":"{subtitle}","buttons":false,"widgets":[{{"kind":"progress"}}]}}}}"#
            );
            emit(U16CString::from_str_truncate(&json).as_ptr());
            0
        }
        None => 2,
    }
}

#[no_mangle]
pub extern "system" fn installway_up(ctx: *const InstallwayContext) -> i32 {
    unsafe {
        log(
            ctx,
            "INFO",
            &format!("Searching for UpgradeCode: {UPGRADE_CODE}"),
        )
    };

    let Some(product_code) = find_product_code_by_upgrade_code(UPGRADE_CODE) else {
        unsafe { log(ctx, "INFO", "No previous MSI version detected.") };
        return 0;
    };

    unsafe { log(ctx, "INFO", &format!("Found ProductCode: {product_code}")) };
    unsafe {
        log(
            ctx,
            "INFO",
            &format!("Running: msiexec /x {product_code} /qn"),
        )
    };

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