// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Close running processes from the install directory before writing files.
//!
//! Pre-check (`ensure_closed`): scan the whole install dir for running
//! processes, then show a TaskDialog (Close and retry / Force close / Cancel).
//! Headless callers (silent install, elevated worker) get a silent force-kill
//! instead (hwnd_parent == 0). Reactive (`kill_if_running`): force-kill a
//! specific exe locking a file (used when the uninstaller write fails).

use anyhow::{Result, bail};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_USE_COMMAND_LINKS, TaskDialogIndirect,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    SW_RESTORE, SetForegroundWindow, ShowWindow, WM_CLOSE,
};
use windows::core::{PCWSTR, PWSTR};

/// Pause after processes exit so the OS / AV release file handles.
const SETTLE: Duration = Duration::from_millis(800);
/// Poll interval while waiting for graceful exit.
const POLL: Duration = Duration::from_millis(200);
/// How long to wait for graceful close before re-showing the dialog.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);
/// Max headless force-kill rounds before giving up (respawning process).
const MAX_HEADLESS_KILL_ROUNDS: u32 = 5;

enum BlockChoice {
    CloseAndRetry,
    ForceClose,
    Cancel,
}

const CLOSE_RETRY_ID: i32 = 101;
const FORCE_CLOSE_ID: i32 = 102;

/// Ensure no processes from `install_dir` are running before writing files.
///
/// `hwnd_parent != 0`: shows a TaskDialog (Close and retry / Force close /
/// Cancel) on each iteration until the dir is clear.
/// `hwnd_parent == 0` (headless): force-kills silently and continues.
///
/// Returns `Err` only if the user cancels or `cancel` is set.
pub fn ensure_closed(
    install_dir: &Path,
    hwnd_parent: isize,
    tr: common::i18n::Translator,
    cancel: &Arc<AtomicBool>,
    status: &dyn Fn(&str),
) -> Result<()> {
    if !install_dir.exists() {
        return Ok(());
    }
    let mut headless_kills = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("Installation cancelled.");
        }
        let pid_names = find_pids_in_dir(install_dir);
        if pid_names.is_empty() {
            return Ok(());
        }

        let mut seen = std::collections::BTreeSet::new();
        let mut app_names: Vec<String> = Vec::new();
        for (_, name) in &pid_names {
            if seen.insert(name.to_ascii_lowercase()) {
                app_names.push(name.clone());
            }
        }
        let pids: Vec<u32> = pid_names.iter().map(|(p, _)| *p).collect();
        let app_list = app_names.join(", ");

        common::log::info(format!(
            "{} process(es) running from install dir ({app_list})",
            pids.len()
        ));

        if hwnd_parent == 0 {
            // Bounded: a process that keeps respawning (service, watchdog)
            // would otherwise loop forever with no way to cancel.
            headless_kills += 1;
            if headless_kills > MAX_HEADLESS_KILL_ROUNDS {
                bail!(
                    "processes from the install folder keep restarting ({app_list}); \
                     stop them and retry"
                );
            }
            common::log::warn(format!("headless install: force-killing {app_list}"));
            kill_pids(&pids);
            thread::sleep(SETTLE);
            continue;
        }

        status(&tr.fmt("install.proc_blocked_waiting", &[("apps", &app_list)]));

        match show_blocking_dialog(hwnd_parent, tr, &app_names) {
            BlockChoice::CloseAndRetry => {
                focus_and_close(&pids);
                nudge_close(&pids);
                wait_for_exit(&pids, CLOSE_TIMEOUT, status, tr, &app_list);
                thread::sleep(SETTLE);
            }
            BlockChoice::ForceClose => {
                kill_pids(&pids);
                thread::sleep(SETTLE);
            }
            BlockChoice::Cancel => {
                bail!(
                    "{}",
                    tr.fmt("install.proc_blocked_cancelled", &[("apps", &app_list)])
                );
            }
        }
    }
}

/// Force-kill any process running `path`. Returns `true` if at least one was
/// found. Used when the uninstaller exe is locked by a leftover running
/// instance during re-install.
pub fn kill_if_running(path: &Path) -> bool {
    let target_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let pids = find_pids(path, &target_name);
    if pids.is_empty() {
        return false;
    }
    kill_pids(&pids);
    true
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

fn show_blocking_dialog(
    hwnd_parent: isize,
    tr: common::i18n::Translator,
    apps: &[String],
) -> BlockChoice {
    use common::utils::wide;

    let title = wide(&tr.get("install.proc_blocked_title"));
    let instruction = wide(&tr.get("install.proc_blocked_instruction"));
    let content_str = apps
        .iter()
        .map(|n| format!("\u{2022} {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content = wide(&content_str);
    let btn_close = wide(&tr.get("install.proc_blocked_btn_close"));
    let btn_force = wide(&tr.get("install.proc_blocked_btn_force"));

    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: CLOSE_RETRY_ID,
            pszButtonText: PCWSTR(btn_close.as_ptr()),
        },
        TASKDIALOG_BUTTON {
            nButtonID: FORCE_CLOSE_ID,
            pszButtonText: PCWSTR(btn_force.as_ptr()),
        },
    ];

    let mut clicked = 0i32;
    unsafe {
        let mut cfg: TASKDIALOGCONFIG = std::mem::zeroed();
        cfg.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
        cfg.hwndParent = HWND(hwnd_parent as *mut _);
        cfg.dwFlags = TDF_USE_COMMAND_LINKS | TDF_ALLOW_DIALOG_CANCELLATION;
        cfg.dwCommonButtons = TDCBF_CANCEL_BUTTON;
        cfg.pszWindowTitle = PCWSTR(title.as_ptr());
        // TD_WARNING_ICON = MAKEINTRESOURCEW(-1) = 0xFFFF as a resource-id pointer.
        cfg.Anonymous1.pszMainIcon = PCWSTR(0xFFFF_usize as *const u16);
        cfg.pszMainInstruction = PCWSTR(instruction.as_ptr());
        cfg.pszContent = PCWSTR(content.as_ptr());
        cfg.cButtons = buttons.len() as u32;
        cfg.pButtons = buttons.as_ptr();
        cfg.nDefaultButton = CLOSE_RETRY_ID;
        let _ = TaskDialogIndirect(&cfg, Some(&mut clicked), None, None);
    }

    match clicked {
        CLOSE_RETRY_ID => BlockChoice::CloseAndRetry,
        FORCE_CLOSE_ID => BlockChoice::ForceClose,
        _ => BlockChoice::Cancel,
    }
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

/// Find all processes running from inside `dir`. Excludes the current process.
fn find_pids_in_dir(dir: &Path) -> Vec<(u32, String)> {
    let canon_dir = std::fs::canonicalize(dir).ok();
    let self_pid = std::process::id();
    let mut out = Vec::new();
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
                let pid = entry.th32ProcessID;
                if pid != self_pid
                    && let Some(image) = process_image_path(pid)
                {
                    let in_dir = match (&canon_dir, std::fs::canonicalize(&image).ok()) {
                        (Some(d), Some(p)) => p.starts_with(d),
                        _ => image.starts_with(dir),
                    };
                    if in_dir {
                        let name = image
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| wide_to_string(&entry.szExeFile));
                        out.push((pid, name));
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

/// Snapshot all processes; return pids matching `target_path` exactly.
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
                    if let Some(path) = process_image_path(pid) {
                        let matches = match (&canon_target, std::fs::canonicalize(&path).ok()) {
                            (Some(a), Some(b)) => a == &b,
                            _ => path == *target_path,
                        };
                        if matches {
                            out.push(pid);
                        }
                    } else {
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

fn kill_pids(pids: &[u32]) {
    for &pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Restore and foreground all top-level windows of the given pids so the user
/// can see and interact with any save/confirm dialogs they trigger.
fn focus_and_close(pids: &[u32]) {
    for hwnd in windows_for_pids(pids) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Post WM_CLOSE to all top-level windows of the given pids.
fn nudge_close(pids: &[u32]) {
    for hwnd in windows_for_pids(pids) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Poll until all pids have exited or `timeout` elapses.
fn wait_for_exit(
    pids: &[u32],
    timeout: Duration,
    status: &dyn Fn(&str),
    tr: common::i18n::Translator,
    app_list: &str,
) {
    let mut remaining = pids.to_vec();
    let deadline = std::time::Instant::now() + timeout;
    while !remaining.is_empty() && std::time::Instant::now() < deadline {
        thread::sleep(POLL);
        remaining.retain(|&p| is_alive(p));
        if !remaining.is_empty() {
            status(&tr.fmt("install.proc_blocked_waiting", &[("apps", app_list)]));
        }
    }
}

fn process_image_path(pid: u32) -> Option<std::path::PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 32768];
        let mut len = buf.len() as u32;
        let res =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
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

fn is_alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) if !h.is_invalid() => {
                let r = WaitForSingleObject(h, 0);
                let _ = CloseHandle(h);
                r != WAIT_OBJECT_0
            }
            _ => false,
        }
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
