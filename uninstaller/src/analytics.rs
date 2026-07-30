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
pub fn init(tenant_id: Option<&str>, mode: &str, privilege: &str) {
    let Some(tenant_id) = tenant_id else { return };

    let mgr = AnalyticsManager::instance();

    let mut custom = HashMap::new();
    custom.insert("operation".to_string(), json!("uninstall"));
    custom.insert("mode".to_string(), json!(mode));
    custom.insert("privilege".to_string(), json!(privilege));
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
