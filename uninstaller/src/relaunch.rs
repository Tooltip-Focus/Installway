// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Escape hatch out of a packaged launcher's process tree.

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Re-launch this executable through Explorer and let the caller exit.
///
/// Windows 11's Settings app runs `UninstallString` from inside its own package
/// (`windows.immersivecontrolpanel`), and children inherit that identity; XAML
/// then resolves against the launcher's package graph and crashes on startup.
/// `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY` cannot help - the breakaway policy
/// is ignored for UWP launchers - but Explorer is a plain desktop process, so
/// what it starts on our behalf has no package identity at all.
///
/// Explorer takes a path and passes no arguments, so this only covers the
/// argument-free interactive uninstall; every other entry point stays in-process
/// and falls back to the Win32 UI instead.
pub fn via_explorer() -> Result<()> {
    let exe = std::env::current_exe()?;
    Command::new("explorer.exe")
        .arg(&exe)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .with_context(|| format!("relaunch {} through Explorer", exe.display()))?;
    Ok(())
}
