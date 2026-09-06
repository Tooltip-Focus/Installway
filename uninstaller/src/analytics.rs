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

/// Flush all queued events (blocking, ≤ 5 s). Fires `app_exit` automatically.
pub fn shutdown() {
    AnalyticsManager::instance().shutdown();
}
