// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

pub mod assoc;
pub mod elevation;
pub mod i18n;
pub mod log;
pub mod model;
pub mod paths;
pub mod plugin;
pub mod registry;
pub mod shortcuts;
pub mod utils;

/// Progress callback shared by the installer and uninstaller UI/worker paths:
/// `(done_bytes, total_bytes, status_message)`. Aliased so the boxed-trait type
/// stays readable at its many call sites (and dodges clippy's `type_complexity`).
pub type ProgressFn = std::sync::Arc<dyn Fn(u64, u64, &str) + Send + Sync>;
