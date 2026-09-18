// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Win32 helpers used by both installer UIs (full wizard + minimal updater):
//! the app-defined window messages, plus the helpers shared with the
//! uninstaller.

pub use common::win32::*;
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// App-defined window messages posted from the worker thread to the UI thread.
pub const WM_APP_DONE: u32 = WM_APP + 2;
pub const WM_APP_ERROR: u32 = WM_APP + 3;
/// Posted by the background plugin-step query thread when a result is ready.
pub const WM_APP_PLUGIN_STEP: u32 = WM_APP + 4;
/// Posted by the progress pipe reader thread; WPARAM carries the 0–100 value.
pub const WM_APP_PLUGIN_PROGRESS: u32 = WM_APP + 5;
/// Permission denied on the install dir; elevation may help. LPARAM is a
/// `Box<PermErrorPayload>` (path + plugin_inputs).
pub const WM_APP_PERM_ERROR: u32 = WM_APP + 6;
/// UAC was cancelled or the elevated worker failed to start. LPARAM is a
/// `Box<PathBuf>` (the rejected install dir, for go-back-to-Choose).
pub const WM_APP_PERM_DENIED: u32 = WM_APP + 7;
/// The user confirmed cancellation from the window's close button; the worker
/// has rolled the install back, so the window can close cleanly (no error page).
pub const WM_APP_CANCELLED: u32 = WM_APP + 8;
