// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Close a running copy of the app we are about to install over.
//!
//! Never force-terminates: find every process running the target exe, post
//! `WM_CLOSE` to its windows (so the app can prompt to save), and wait for the
//! user to close it, re-nudging periodically. No-op on a fresh install.

use anyhow::{Result, bail};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE, WM_CLOSE,
};
use windows::Win32::Foundation::BOOL;
use windows::core::PWSTR;

/// Poll cadence while waiting for exit.
const POLL: Duration = Duration::from_millis(200);
/// Re-focus + re-send WM_CLOSE every this often so the prompt stays in view.
const NUDGE: Duration = Duration::from_secs(5);
/// Pause after the app exits so the OS / AV release the file handle.
const SETTLE: Duration = Duration::from_millis(800);

/// Ensure no running instance of `exe_rel` (relative to `install_dir`) holds
/// our files. Blocks until every matching process has exited (user-driven) or
/// `cancel` is set. `status` receives short progress strings for the UI.
///
/// Returns `Err` only if the user cancelled while the app was still open.
pub fn ensure_closed(
    install_dir: &Path,
    exe_rel: &str,
    cancel: &Arc<AtomicBool>,
    status: &dyn Fn(&str),
) -> Result<()> {
    if exe_rel.trim().is_empty() {
        return Ok(());
    }
    let target = install_dir.join(exe_rel);
    let target_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let mut pids = find_pids(&target, &target_name);
    if pids.is_empty() {
        return Ok(());
    }

    let app = display_name(exe_rel);
    common::log::info(format!(
        "target app running ({} process(es)); requesting close (no force)",
        pids.len()
    ));
    status(&format!("Please close {} to continue...", app));

    // First close request: focus each window + WM_CLOSE.
    nudge(&pids);

    let started = Instant::now();
    let mut last_nudge = Instant::now();

    loop {
        pids.retain(|&p| is_alive(p));
        if pids.is_empty() {
            common::log::info(format!(
                "target app closed by user after {}s",
                started.elapsed().as_secs()
            ));
            thread::sleep(SETTLE);
            return Ok(());
        }

        if cancel.load(Ordering::Relaxed) {
            common::log::warn("user cancelled while target app still open");
            bail!("Installation cancelled: {} is still running.", app);
        }

        // Periodically re-focus + re-ask so the user keeps seeing the prompt.
        if last_nudge.elapsed() >= NUDGE {
            nudge(&pids);
            last_nudge = Instant::now();
            status(&format!("Waiting for {} to close...", app));
        }

        thread::sleep(POLL);
    }
}

/// Focus + WM_CLOSE every current top-level window of the given pids.
fn nudge(pids: &[u32]) {
    for hwnd in windows_for_pids(pids) {
        focus_and_close(hwnd);
    }
}

fn display_name(exe_rel: &str) -> String {
    Path::new(exe_rel)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| exe_rel.to_string())
}

/// Snapshot all processes; return pids matching the target exe.
/// Prefer full-path match (precise); fall back to file-name match.
fn find_pids(target_path: &Path, target_name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let canon_target = std::fs::canonicalize(target_path).ok();

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return out,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = wide_to_string(&entry.szExeFile).to_ascii_lowercase();
                if name == *target_name {
                    let pid = entry.th32ProcessID;
                    // Confirm by full path when we can read it.
                    if let Some(path) = process_image_path(pid) {
                        let matches = match (&canon_target, std::fs::canonicalize(&path).ok()) {
                            (Some(a), Some(b)) => a == &b,
                            _ => path == *target_path,
                        };
                        if matches {
                            out.push(pid);
                        }
                    } else {
                        // Couldn't read path (access); fall back to name match.
                        out.push(pid);
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

fn process_image_path(pid: u32) -> Option<std::path::PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 32768];
        let mut len = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(h);
        if res.is_ok() && len > 0 {
            Some(std::path::PathBuf::from(String::from_utf16_lossy(
                &buf[..len as usize],
            )))
        } else {
            None
        }
    }
}

thread_local! {
    static COLLECT: std::cell::RefCell<(Vec<u32>, Vec<isize>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// Top-level visible windows owned by any of `pids`.
fn windows_for_pids(pids: &[u32]) -> Vec<HWND> {
    COLLECT.with(|c| {
        *c.borrow_mut() = (pids.to_vec(), Vec::new());
    });
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(0));
    }
    COLLECT.with(|c| {
        c.borrow()
            .1
            .iter()
            .map(|h| HWND(*h as *mut core::ffi::c_void))
            .collect()
    })
}

unsafe extern "system" fn enum_cb(hwnd: HWND, _l: LPARAM) -> BOOL {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        // Top-level only: skip windows that have an owner.
        if !GetWindow(hwnd, GW_OWNER).unwrap_or_default().is_invalid() {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        COLLECT.with(|c| {
            let mut b = c.borrow_mut();
            if b.0.contains(&pid) {
                b.1.push(hwnd.0 as isize);
            }
        });
    }
    true.into()
}

fn focus_and_close(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn is_alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) if !h.is_invalid() => {
                // 0ms wait: WAIT_OBJECT_0 means it already exited (signaled).
                let r = WaitForSingleObject(h, 0);
                let _ = CloseHandle(h);
                r != WAIT_OBJECT_0
            }
            _ => false, // can't open → treat as gone
        }
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
