// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use hintway_analytics::AnalyticsManager;
use serde_json::json;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

const URL: &str = "https://in.hintway.app";

thread_local! {
    static CURRENT_STAGE: Cell<&'static str> = const { Cell::new("startup") };
}

// Local copy of custom fields so individual keys can be updated after init.
static CUSTOM: OnceLock<Mutex<HashMap<String, serde_json::Value>>> = OnceLock::new();

fn custom_map() -> &'static Mutex<HashMap<String, serde_json::Value>> {
    CUSTOM.get_or_init(|| Mutex::new(HashMap::new()))
}

fn apply_custom(mgr: &AnalyticsManager) {
    let data = custom_map().lock().unwrap().clone();
    mgr.set_custom_data(if data.is_empty() { None } else { Some(data) });
}

/// Initialize analytics for one installer run.
///
/// Custom data sent with every event (GDPR-safe, no personal data):
/// - `operation`: `"install"` | `"update"`
/// - `mode`:      `"silent"` | `"minimal"` | `"interactive"`
/// - `privilege`: `"admin"` | `"user"` (updated post-install for interactive via `set_privilege`)
/// - `lang`:      detected ISO-639-1 code (e.g. `"en"`, `"fr"`)
pub fn init(
    tenant_id: Option<&str>,
    app_version: &str,
    operation: &str,
    mode: &str,
    privilege: &str,
    lang: &str,
) {
    let Some(tenant_id) = tenant_id else { return };

    {
        let mut c = custom_map().lock().unwrap();
        c.insert("operation".to_string(), json!(operation));
        c.insert("mode".to_string(), json!(mode));
        c.insert("privilege".to_string(), json!(privilege));
        c.insert("lang".to_string(), json!(lang));
    }

    let mgr = AnalyticsManager::instance();
    // Apply custom data BEFORE init so app_started captures it.
    apply_custom(mgr);

    let identity = Uuid::new_v4().to_string();
    mgr.init(
        tenant_id,
        URL,
        "windows",
        app_version,
        true,
        30,
        Some(&identity),
    );
}

/// Update privilege after the install completes (interactive mode: path chosen inside UI).
/// Affects all events created after this call, including app_exit.
pub fn set_privilege(is_admin: bool) {
    custom_map().lock().unwrap().insert(
        "privilege".to_string(),
        json!(if is_admin { "admin" } else { "user" }),
    );
    apply_custom(AnalyticsManager::instance());
}

/// Record a named install stage. Also remembered for error attribution.
///
/// Stages (in order): `"extract"` → `"finalize"` → `"done"`
pub fn stage(name: &'static str) {
    CURRENT_STAGE.with(|s| s.set(name));
    AnalyticsManager::instance().track_event_string("stage_reached", name);
}

/// Record an install error. Stage is read from thread-local.
///
/// `category`: `"version_mismatch"` | `"permission_denied"` | `"signature_failed"` |
/// `"disk_full"` | `"extract_failed"` | `"finalize_failed"` | `"unknown"`
pub fn error(category: &str) {
    let at_stage = CURRENT_STAGE.with(|s| s.get());
    AnalyticsManager::instance().track_event_string(
        "install_error",
        &json!({ "category": category, "stage": at_stage }).to_string(),
    );
}

/// Flush all queued events (blocking, ≤ 5 s). Fires `app_exit` automatically.
pub fn shutdown() {
    AnalyticsManager::instance().shutdown();
}

/// Classify an anyhow error into a safe category string (no paths, no personal data).
pub(crate) fn classify_error(e: &anyhow::Error) -> &'static str {
    if e.downcast_ref::<crate::extract::VersionMismatch>()
        .is_some()
    {
        return "version_mismatch";
    }
    if e.downcast_ref::<crate::extract::PermissionDeniedError>()
        .is_some()
    {
        return "permission_denied";
    }
    let msg = format!("{e:#}").to_lowercase();
    if msg.contains("full") || msg.contains("no space") || msg.contains("disk") {
        return "disk_full";
    }
    if msg.contains("signature") || msg.contains("hash") || msg.contains("verify") {
        return "signature_failed";
    }
    if msg.contains("permission") || msg.contains("access") || msg.contains("denied") {
        return "permission_denied";
    }
    // Walk the anyhow chain for an io::Error — its kind is a safe enum (no paths).
    for cause in e.chain() {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return match io.kind() {
                std::io::ErrorKind::NotFound => "io_not_found",
                std::io::ErrorKind::PermissionDenied => "permission_denied",
                std::io::ErrorKind::AlreadyExists => "io_already_exists",
                std::io::ErrorKind::OutOfMemory => "io_out_of_memory",
                std::io::ErrorKind::TimedOut => "io_timed_out",
                std::io::ErrorKind::StorageFull => "disk_full",
                _ => "io_other",
            };
        }
    }
    "unknown"
}
