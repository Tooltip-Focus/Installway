// SPDX-License-Identifier: MIT
//! Example Installway plugin in Rust. Mirrors `sdk/installway_plugin.h`.
//! Logs a line via the host callback and succeeds.

use widestring::U16CString;

const INSTALLWAY_ABI_VERSION: u32 = 1;

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
    // Used only by custom-page plugins (see country_picker); unused here.
    inputs_json: *const u16,
    emit_pages: Option<extern "system" fn(*const u16)>,
}

/// Call the host log callback, if present.
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

#[no_mangle]
pub extern "system" fn installway_abi_version() -> u32 {
    INSTALLWAY_ABI_VERSION
}

#[no_mangle]
pub extern "system" fn installway_up(ctx: *const InstallwayContext) -> i32 {
    unsafe { log(ctx, "INFO", "example_plugin: up") };
    // ... do work here; return non-zero to fail the install ...
    0
}

#[no_mangle]
pub extern "system" fn installway_down(ctx: *const InstallwayContext) -> i32 {
    unsafe { log(ctx, "INFO", "example_plugin: down") };
    0
}
