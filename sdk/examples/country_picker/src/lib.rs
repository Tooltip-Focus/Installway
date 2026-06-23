// SPDX-License-Identifier: MIT
//! Example Installway plugin. Contributes a "choose your country" wizard page;
//! the installer renders it, and `installway_up` acts on the answer. Mirrors
//! `sdk/installway_plugin.h`.

use widestring::{U16CStr, U16CString};

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
    inputs_json: *const u16,
    emit_pages: Option<extern "system" fn(*const u16)>,
}

/// File where we remember the choice across upgrades — inside `data_dir`, next to
/// installer_info.json. The uninstaller deletes that folder, so it auto-cleans.
/// Returns `None` when `data_dir` is empty or null (no persistent storage available).
fn state_path(ctx: *const InstallwayContext) -> Option<std::path::PathBuf> {
    let data_dir = unsafe { from_wide((*ctx).data_dir) };
    if data_dir.is_empty() {
        return None;
    }
    Some(std::path::Path::new(&data_dir).join("country_picker.json"))
}

/// Copy a null-terminated wide string from the host into a `String`.
/// Returns an empty string for a null pointer.
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { U16CStr::from_ptr_str(p) }.to_string_lossy()
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

/// One wizard step. `installway_pages` is a step function: the host calls it with
/// the answers so far in `ctx->inputs_json`; we return the next page (or done).
///
/// This shows a branch: after the country page, if the user picked DOM-TOM we ask
/// a second, dependent page; otherwise we're done. The plugin stays stateless —
/// the host carries the answers.
#[no_mangle]
pub extern "system" fn installway_pages(ctx: *const InstallwayContext) -> i32 {
    if ctx.is_null() {
        return 1;
    }
    // Remembered from a previous install? Skip the whole flow (`up` reuses it).
    if state_path(ctx).and_then(|p| std::fs::read_to_string(p).ok()).is_some() {
        if let Some(emit) = unsafe { (*ctx).emit_pages } {
            emit(U16CString::from_str_truncate(r#"{ "step": "done" }"#).as_ptr());
        }
        return 0;
    }
    let answers = unsafe { from_wide((*ctx).inputs_json) };
    let answers: serde_json::Value = serde_json::from_str(&answers).unwrap_or_default();
    let country = answers["region.country"].as_str();
    let territory = answers["dom.territory"].as_str();

    let step = if country.is_none() {
        // First page: pick a country.
        r#"{ "step": "page", "page": {
              "id": "region",
              "title": "Choose your country",
              "subtitle": "This tailors the install to your region.",
              "widgets": [
                { "kind": "single_choice", "id": "country", "label": "Country",
                  "style": "radio", "default": "FR", "required": true,
                  "options": [
                    { "label": "France (metropolitan)", "value": "FR" },
                    { "label": "DOM-TOM", "value": "DOM" },
                    { "label": "Other", "value": "XX" }
                  ] }
              ] } }"#
    } else if country == Some("DOM") && territory.is_none() {
        // Dependent page: only shown when DOM-TOM was chosen on the first page.
        r#"{ "step": "page", "page": {
              "id": "dom",
              "title": "Which territory?",
              "subtitle": "Pick your DOM-TOM.",
              "widgets": [
                { "kind": "single_choice", "id": "territory", "label": "Territory",
                  "style": "combo", "required": true,
                  "options": [
                    { "label": "Guadeloupe", "value": "GP" },
                    { "label": "Martinique", "value": "MQ" },
                    { "label": "Reunion", "value": "RE" }
                  ] }
              ] } }"#
    } else {
        r#"{ "step": "done" }"#
    };

    match unsafe { (*ctx).emit_pages } {
        Some(emit) => {
            emit(U16CString::from_str_truncate(step).as_ptr());
            0
        }
        None => 2,
    }
}

/// Act on the answers at install. `ctx->inputs_json` carries this run's answers;
/// on an upgrade where the page was skipped it's empty, so we fall back to the
/// remembered `state_path`. We then (re)write that state so the next upgrade can
/// skip, and drop a `selected-country.json` at the install root to show the result.
/// Declare the plugin `phase = "post-install"` so `data_dir`/`install_dir` exist.
#[no_mangle]
pub extern "system" fn installway_up(ctx: *const InstallwayContext) -> i32 {
    if ctx.is_null() {
        return 1;
    }
    let inputs = unsafe { from_wide((*ctx).inputs_json) };
    let inputs: serde_json::Value = serde_json::from_str(&inputs).unwrap_or_default();
    let saved = state_path(ctx)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap_or_default();

    // This run's answer, else the remembered one, else the default.
    let country = inputs["region.country"]
        .as_str()
        .or_else(|| saved["country"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "FR".to_string());
    let territory = inputs["dom.territory"]
        .as_str()
        .or_else(|| saved["territory"].as_str())
        .map(str::to_owned);
    let body = serde_json::json!({ "country": country, "territory": territory }).to_string();

    // Remember it (data_dir) for next time, and show the result (install root).
    if let Some(p) = state_path(ctx) {
        let _ = std::fs::write(p, &body);
    }
    let install_dir = unsafe { from_wide((*ctx).install_dir) };
    let _ = std::fs::create_dir_all(&install_dir);
    let out = std::path::Path::new(&install_dir).join("selected-country.json");
    if let Err(e) = std::fs::write(&out, &body) {
        unsafe { log(ctx, "ERROR", &format!("country_picker: write {} failed: {e}", out.display())) };
        return 4;
    }
    unsafe { log(ctx, "INFO", &format!("country_picker: country={country}")) };
    0
}

#[no_mangle]
pub extern "system" fn installway_down(ctx: *const InstallwayContext) -> i32 {
    unsafe { log(ctx, "INFO", "country_picker: down (nothing to undo)") };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal context whose `data_dir` points at `dir` (a NUL-terminated
    /// wide buffer the caller keeps alive), all other pointers null.
    fn ctx_with_data_dir(dir: *const u16) -> InstallwayContext {
        InstallwayContext {
            abi_version: INSTALLWAY_ABI_VERSION,
            install_dir: std::ptr::null(),
            data_dir: dir,
            product: std::ptr::null(),
            product_id: std::ptr::null(),
            version: std::ptr::null(),
            exe: std::ptr::null(),
            log: None,
            inputs_json: std::ptr::null(),
            emit_pages: None,
        }
    }

    #[test]
    fn state_path_none_on_null_data_dir() {
        let ctx = ctx_with_data_dir(std::ptr::null());
        assert!(state_path(&ctx).is_none());
    }

    #[test]
    fn state_path_none_on_empty_data_dir() {
        let empty: Vec<u16> = "\0".encode_utf16().collect();
        let ctx = ctx_with_data_dir(empty.as_ptr());
        assert!(state_path(&ctx).is_none());
    }

    #[test]
    fn state_path_joins_filename_when_data_dir_set() {
        let dir: Vec<u16> = "C:\\data\0".encode_utf16().collect();
        let ctx = ctx_with_data_dir(dir.as_ptr());
        let p = state_path(&ctx).expect("data_dir set -> Some");
        assert_eq!(p.file_name().unwrap(), "country_picker.json");
        assert!(p.starts_with("C:\\data"));
    }

    #[test]
    fn from_wide_round_trips_and_handles_null() {
        let w: Vec<u16> = "Côte\0".encode_utf16().collect();
        assert_eq!(unsafe { from_wide(w.as_ptr()) }, "Côte");
        assert_eq!(unsafe { from_wide(std::ptr::null()) }, "");
    }
}
