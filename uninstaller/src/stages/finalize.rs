// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Finalize step: runs from `%TEMP%` after the uninstall step spawned us. Waits for
//! the uninstall step to exit (releasing the `uninstall.exe` lock), removes the app
//! dir and data dir, then schedules its own removal via
//! `MoveFileExW(MOVEFILE_DELAY_UNTIL_REBOOT)`. No `cmd.exe`, no console flash.

use crate::ui::{self, StepCounter, UninstallParams};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn run(
    app_dir: Option<PathBuf>,
    data_dir: PathBuf,
    product: String,
    parent_pid: Option<u32>,
) -> Result<()> {
    // Continue the uninstall step's log file (keyed by its PID) so the whole
    // uninstall is in one %TEMP% file for support.
    let log_id = parent_pid.unwrap_or_else(std::process::id);
    common::log::init(common::log::log_path_uninstall_temp(&product, log_id));
    common::log::info(format!(
        "finalize start: product={} app_dir={} data_dir={} parent_pid={:?}",
        product,
        app_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".into()),
        data_dir.display(),
        parent_pid
    ));

    let tr = crate::ui::tr();
    let params = UninstallParams {
        title: tr.fmt("uninstall.finalize_title", &[("product", &product)]),
        subtitle: tr.get("uninstall.finalize_subtitle"),
        confirm_text: String::new(), // never shown - auto-advances to Progress
        worker: Box::new(move |progress: Arc<dyn Fn(u64, u64, &str) + Send + Sync>| {
            let counter = StepCounter::new(4, progress);
            counter.step(&tr.get("uninstall.waiting"));
            // Wait for the uninstall step to exit so file locks release.
            if let Some(pid) = parent_pid {
                wait_for_pid(pid, Duration::from_secs(10));
            }

            counter.step(&tr.get("uninstall.removing_install_dir"));
            // Remove the application dir; absent when metadata was unreadable.
            if let Some(ref dir) = app_dir {
                common::utils::remove_dir_retry(dir);
            }

            // Remove the data dir we launched from (we run from the %TEMP% copy).
            common::utils::remove_dir_retry(&data_dir);
            // Prune now-empty parent folders (Uninstall, publisher).
            if let Some(parent) = data_dir.parent() {
                let _ = fs::remove_dir(parent); // "Uninstall"
                if let Some(grand) = parent.parent() {
                    let _ = fs::remove_dir(grand); // "<publisher>"
                }
            }

            counter.step(&tr.get("uninstall.schedule_deletion"));
            // Schedule self for deletion on next reboot (no cmd, no flash).
            schedule_self_delete_on_reboot();
            common::log::info("finalize complete; self scheduled for delete-on-reboot");
            counter.step(&tr.get("uninstall.done"));

            // Brief pause so user sees the 100% bar.
            thread::sleep(Duration::from_millis(400));
        }),
        auto_start: true,
    };

    let _ = ui::run(params);
    Ok(())
}

fn wait_for_pid(pid: u32, timeout: Duration) {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) if !h.is_invalid() => {
                let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
                let r = WaitForSingleObject(h, ms);
                let _ = CloseHandle(h);
                if r == WAIT_OBJECT_0 {
                    return;
                }
            }
            _ => {}
        }
    }
    // Fallback: short sleep so locks at least likely released.
    thread::sleep(Duration::from_millis(500));
}

fn schedule_self_delete_on_reboot() {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
    use windows::core::PCWSTR;

    let Ok(self_exe) = std::env::current_exe() else {
        return;
    };
    let w: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(self_exe.as_os_str())
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = MoveFileExW(
            PCWSTR(w.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}
