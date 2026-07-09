// SPDX-License-Identifier: MIT
//! Example Installway plugin: an interactive feature-pack picker.
//!
//! Declared `ui = true`. `installway_pages` shows a checkbox list of every
//! feature the build declares (from `ctx.features_json`), pre-checked to the
//! current set (`active`) — the base the host picked per `feature_mode`: the build
//! defaults on a fresh install, then either the prior install's set (`sticky`) or
//! the build defaults again (`override`) on an upgrade. `installway_features` then
//! turns the user's checked set into the `{ enable, disable }` delta the host applies. The
//! host persists the result itself (in `installer_info.json`); the plugin keeps
//! no state. In silent/compact mode the page falls back to its defaults, so the
//! base set installs unattended. Mirrors `sdk/installway_plugin.h`.

use serde_json::Value;
use widestring::{U16CStr, U16CString};

const INSTALLWAY_ABI_VERSION: u32 = 1;

/// Page id + widget id; together they key the answer as `"features.packs"`.
const PAGE_ID: &str = "features";
const WIDGET_ID: &str = "packs";

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
    emit_progress: Option<extern "system" fn(u32)>,
    lang: *const u16,
    features_json: *const u16,
}

/// Copy a null-terminated wide string from the host. Empty for a null pointer.
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { U16CStr::from_ptr_str(p) }.to_string_lossy()
}

unsafe fn emit(ctx: *const InstallwayContext, json: &str) -> i32 {
    match unsafe { (*ctx).emit_pages } {
        Some(cb) => {
            cb(U16CString::from_str_truncate(json).as_ptr());
            0
        }
        None => 2,
    }
}

/// `(all, active)` from the host's `features_json` catalog.
fn parse_catalog(json: &str) -> (Vec<String>, Vec<String>) {
    let v: Value = serde_json::from_str(json).unwrap_or_default();
    let list = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    (list("all"), list("active"))
}

/// Split a multi_choice answer (`"A,B"`) into the checked ids.
fn parse_checked(answer: &str) -> Vec<String> {
    answer
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Turn the checked set into a delta: enable the checked, disable the rest.
fn delta(all: &[String], checked: &[String]) -> (Vec<String>, Vec<String>) {
    let disable = all
        .iter()
        .filter(|id| !checked.contains(id))
        .cloned()
        .collect();
    (checked.to_vec(), disable)
}

#[no_mangle]
pub extern "system" fn installway_abi_version() -> u32 {
    INSTALLWAY_ABI_VERSION
}

/// Step function: emit the checkbox page once, then `done` once it's answered.
#[no_mangle]
pub extern "system" fn installway_pages(ctx: *const InstallwayContext) -> i32 {
    if ctx.is_null() {
        return 1;
    }
    let answers: Value =
        serde_json::from_str(&unsafe { from_wide((*ctx).inputs_json) }).unwrap_or_default();
    let answer_key = format!("{PAGE_ID}.{WIDGET_ID}");
    let (all, active) = parse_catalog(&unsafe { from_wide((*ctx).features_json) });

    // Already answered, or nothing to ask: finish.
    if all.is_empty() || answers.get(&answer_key).is_some() {
        return unsafe { emit(ctx, r#"{ "step": "done" }"#) };
    }

    let options: Vec<Value> = all
        .iter()
        .map(|id| serde_json::json!({ "label": id, "value": id }))
        .collect();
    let step = serde_json::json!({
        "step": "page",
        "page": {
            "id": PAGE_ID,
            "title": "Choose components",
            "subtitle": "Select which feature packs to install.",
            "widgets": [{
                "kind": "multi_choice",
                "id": WIDGET_ID,
                "label": "Components",
                "options": options,
                "default": active,
            }],
        },
    });
    unsafe { emit(ctx, &step.to_string()) }
}

/// Convert the checked set into the enable/disable delta the host applies.
#[no_mangle]
pub extern "system" fn installway_features(ctx: *const InstallwayContext) -> i32 {
    if ctx.is_null() {
        return 1;
    }
    let (all, _) = parse_catalog(&unsafe { from_wide((*ctx).features_json) });
    let answers: Value =
        serde_json::from_str(&unsafe { from_wide((*ctx).inputs_json) }).unwrap_or_default();

    let (enable, disable) = match answers
        .get(format!("{PAGE_ID}.{WIDGET_ID}"))
        .and_then(Value::as_str)
    {
        // No answer (no page shown) → empty delta, build defaults stand.
        None => (Vec::new(), Vec::new()),
        Some(ans) => delta(&all, &parse_checked(ans)),
    };
    let body = serde_json::json!({ "enable": enable, "disable": disable }).to_string();
    unsafe { emit(ctx, &body) }
}

/// Nothing to do at install — the host already staged the chosen features.
#[no_mangle]
pub extern "system" fn installway_up(_ctx: *const InstallwayContext) -> i32 {
    0
}

#[no_mangle]
pub extern "system" fn installway_down(_ctx: *const InstallwayContext) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_catalog_reads_all_and_active() {
        let (all, active) = parse_catalog(r#"{"all":["A","B","C"],"active":["B"]}"#);
        assert_eq!(all, vec!["A", "B", "C"]);
        assert_eq!(active, vec!["B"]);
    }

    #[test]
    fn parse_catalog_empty_on_garbage() {
        let (all, active) = parse_catalog("");
        assert!(all.is_empty() && active.is_empty());
    }

    #[test]
    fn delta_enables_checked_disables_rest() {
        let all = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let (enable, disable) = delta(&all, &parse_checked("A, C"));
        assert_eq!(enable, vec!["A", "C"]);
        assert_eq!(disable, vec!["B"]);
    }

    #[test]
    fn delta_none_checked_disables_all() {
        let all = vec!["A".to_string(), "B".to_string()];
        let (enable, disable) = delta(&all, &parse_checked(""));
        assert!(enable.is_empty());
        assert_eq!(disable, vec!["A", "B"]);
    }
}
