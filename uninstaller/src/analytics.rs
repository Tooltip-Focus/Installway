// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use hintway_analytics::AnalyticsManager;
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

const URL: &str = "https://in.hintway.app";

/// Initialize analytics for one uninstaller run.
///
/// Identity is a random UUID generated here.
pub fn init(tenant_id: Option<&str>, mode: &str, privilege: &str, ui: &str) {
    let Some(tenant_id) = tenant_id else { return };

    let mgr = AnalyticsManager::instance();

    let mut custom = HashMap::new();
    custom.insert("operation".to_string(), json!("uninstall"));
    custom.insert("mode".to_string(), json!(mode));
    custom.insert("privilege".to_string(), json!(privilege));
    custom.insert("ui".to_string(), json!(ui));
    mgr.set_custom_data(Some(custom));

    let identity = Uuid::new_v4().to_string();
    mgr.init(
        tenant_id,
        URL,
        "windows",
        env!("CARGO_PKG_VERSION"),
        true,
        30,
        Some(&identity),
    );
}

/// Initialize analytics for the uninstall entry point, reading the tenant id
/// from the installed metadata. A missing or unreadable `installer_info.json`
/// leaves analytics off.
pub fn init_for_run(silent: bool) {
    let mode = if silent { "silent" } else { "interactive" };
    let data_dir = crate::cleanup::self_dir().unwrap_or_default();
    let privilege = match std::env::var("ProgramData") {
        Ok(pd) if !pd.is_empty() && data_dir.starts_with(&pd) => "admin",
        _ => "user",
    };
    let tenant_id = crate::cleanup::read_info(&data_dir)
        .ok()
        .and_then(|info| info.hintway_tenant_id);
    let backend = if silent {
        "none"
    } else {
        crate::ui::backend_name()
    };
    init(tenant_id.as_deref(), mode, privilege, backend);
}

/// Record an uninstall error.
pub fn error(category: &str) {
    AnalyticsManager::instance().track_event_string(
        "uninstall_error",
        &json!({ "category": category }).to_string(),
    );
}

/// Record an uninstall failure, bucketed by [`category`].
pub fn error_from(err: &anyhow::Error) {
    error(category(err));
}

/// Coarse bucket for a failure: enough to tell a permission wall from a missing
/// file or corrupt metadata, without shipping the message itself (it carries
/// user paths).
fn category(err: &anyhow::Error) -> &'static str {
    if let Some(io) = err.chain().find_map(|c| c.downcast_ref::<std::io::Error>()) {
        return match io.kind() {
            std::io::ErrorKind::NotFound => "not_found",
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            _ => "io",
        };
    }
    if err.chain().any(|c| c.is::<serde_json::Error>()) {
        return "parse";
    }
    "unknown"
}

/// Flush all queued events (blocking, ≤ 5 s). Fires `app_exit` automatically.
pub fn shutdown() {
    AnalyticsManager::instance().shutdown();
}

#[cfg(test)]
mod tests {
    use super::category;
    use anyhow::Context;

    /// The bucket comes from the root cause, through the `anyhow` context the
    /// call sites add.
    #[test]
    fn category_buckets_by_root_cause() {
        let missing = std::fs::read_to_string(r"Z:\nope\installer_info.json")
            .context("read installer_info.json")
            .unwrap_err();
        assert_eq!(category(&missing), "not_found");

        let parse = serde_json::from_str::<serde_json::Value>("{ nope")
            .context("parse installer_info.json")
            .unwrap_err();
        assert_eq!(category(&parse), "parse");

        assert_eq!(
            category(&anyhow::anyhow!("locate uninstaller parent dir")),
            "unknown"
        );
    }
}
