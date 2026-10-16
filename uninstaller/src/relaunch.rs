// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Escape a packaged launcher's process tree without losing the launch monitor.

use anyhow::{Context, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IDispatch, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
    SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{SHOW_WINDOW_CMD, SW_SHOWNORMAL};
use windows::core::{BSTR, Interface};

/// Ask the existing desktop Explorer to launch us outside Settings' package.
/// Unlike `explorer.exe <path>`, desktop shell automation accepts arguments,
/// letting the child restore the original monitor even if focus has changed.
pub fn via_explorer() -> Result<()> {
    let exe = std::env::current_exe()?;
    execute_from_explorer(&exe, &launch_monitor_arg(), SW_SHOWNORMAL)
        .with_context(|| format!("relaunch {} through Explorer", exe.display()))
}

/// Shared by the Explorer relaunch and the detached finalize process.
pub fn launch_monitor_arg() -> String {
    format!(
        "--launch-monitor={}",
        common::window_placement::launch_monitor()
    )
}

fn execute_from_explorer(exe: &Path, args: &str, show: SHOW_WINDOW_CMD) -> Result<()> {
    let _apartment = ComApartment::sta()?;
    let shell = desktop_shell()?;
    let file = BSTR::from_wide(&exe.as_os_str().encode_wide().collect::<Vec<_>>());
    unsafe {
        shell
            .ShellExecute(
                &file,
                &VARIANT::from(args),
                &VARIANT::default(),
                &VARIANT::from("open"),
                &VARIANT::from(show.0),
            )
            .context("launch through Explorer")
    }
}

// Microsoft's desktop automation pattern: obtain Shell.Application from the
// desktop view hosted by Explorer, rather than creating a shell in our process.
// https://devblogs.microsoft.com/oldnewthing/20131118-00/?p=2643
fn desktop_shell() -> Result<IShellDispatch2> {
    unsafe {
        let shell_windows: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
                .context("connect to desktop Explorer")?;
        let mut desktop_hwnd = 0;
        let desktop = shell_windows
            .FindWindowSW(
                &VARIANT::from(CSIDL_DESKTOP as i32),
                &VARIANT::default(),
                SWC_DESKTOP,
                &mut desktop_hwnd,
                SWFO_NEEDDISPATCH,
            )
            .context("find Explorer desktop")?;
        let browser: IShellBrowser = desktop
            .cast::<IServiceProvider>()?
            .QueryService(&SID_STopLevelBrowser)
            .context("get desktop browser")?;
        let view = browser.QueryActiveShellView().context("get desktop view")?;
        // GetItemObject supports IDispatch here; query the dual interface from it.
        let folder: IShellFolderViewDual = view
            .GetItemObject::<IDispatch>(SVGIO_BACKGROUND)
            .context("get desktop automation")?
            .cast()
            .context("get desktop folder")?;
        folder.Application()?.cast().context("get Explorer shell")
    }
}

/// Balance every successful CoInitializeEx, including S_FALSE and error exits.
struct ComApartment;

impl ComApartment {
    fn sta() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the interactive desktop Explorer; launches only a harmless echo probe"]
    fn explorer_passes_negative_monitor_coordinates_to_child() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("launch monitor.txt");
        let cmd = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
        let args = format!(
            r#"/d /c echo --launch-monitor=-960,-540>"{}""#,
            output.display()
        );
        execute_from_explorer(
            &cmd,
            &args,
            windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(text) = std::fs::read_to_string(&output)
                && text.trim() == "--launch-monitor=-960,-540"
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Explorer did not pass the launch point to the probe"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
