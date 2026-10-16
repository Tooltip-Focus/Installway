// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Initial placement shared by the installer, uninstaller and both UI backends.

use std::{fmt, str::FromStr, sync::OnceLock};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowPos,
};

static LAUNCH_MONITOR: OnceLock<LaunchMonitor> = OnceLock::new();

/// A point inside the launch monitor, transferable across process boundaries.
/// Monitor handles cannot be used as persistent launch coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaunchMonitor {
    x: i32,
    y: i32,
}

impl LaunchMonitor {
    /// Capture before opening UI or doing work that may change the foreground.
    pub fn capture() -> Self {
        unsafe {
            let foreground = GetForegroundWindow();
            let monitor = if foreground.is_invalid() {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY)
            } else {
                MonitorFromWindow(foreground, MONITOR_DEFAULTTOPRIMARY)
            };
            let mut info = monitor_info();
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let rc = info.rcMonitor;
                Self {
                    x: rc.left + (rc.right - rc.left) / 2,
                    y: rc.top + (rc.bottom - rc.top) / 2,
                }
            } else {
                Self::default()
            }
        }
    }
}

impl fmt::Display for LaunchMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

impl FromStr for LaunchMonitor {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (x, y) = value.split_once(',').ok_or("expected monitor point x,y")?;
        Ok(Self {
            x: x.parse().map_err(|_| "invalid monitor x coordinate")?,
            y: y.parse().map_err(|_| "invalid monitor y coordinate")?,
        })
    }
}

pub fn capture_launch_monitor() {
    remember_launch_monitor(LaunchMonitor::capture());
}

/// Initialize once, using the original launch point when this is a child process.
pub fn remember_launch_monitor(monitor: LaunchMonitor) {
    let _ = LAUNCH_MONITOR.set(monitor);
}

pub fn launch_monitor() -> LaunchMonitor {
    LAUNCH_MONITOR.get().copied().unwrap_or_default()
}

fn monitor_info() -> MONITORINFO {
    MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    }
}

fn centered_origin(work: RECT, width: i32, height: i32) -> (i32, i32) {
    (
        work.left + ((work.right - work.left - width) / 2).max(0),
        work.top + ((work.bottom - work.top - height) / 2).max(0),
    )
}

/// Center on the remembered monitor's work area; use primary if it disappeared.
pub fn center(hwnd: HWND) {
    center_on(hwnd, launch_monitor());
}

fn center_on(hwnd: HWND, point: LaunchMonitor) {
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: point.x,
                y: point.y,
            },
            MONITOR_DEFAULTTOPRIMARY,
        );
        let mut info = monitor_info();
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        // A cross-monitor move can synchronously resize for DPI. Re-read the
        // final size and center again, without activating or repeatedly pinning UI.
        for _ in 0..2 {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return;
            }
            let (x, y) =
                centered_origin(info.rcWork, rect.right - rect.left, rect.bottom - rect.top);
            if rect.left == x && rect.top == y {
                break;
            }
            let _ = SetWindowPos(
                hwnd,
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(test)]
mod tests;
