// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_HIDE, ShowWindow, WM_CLOSE};

/// Manage a Reactor window's native handle and initial placement.
#[derive(Default)]
pub struct WindowHandle {
    raw: AtomicIsize,
    centered: AtomicBool,
}

#[cfg(test)]
mod tests;

impl WindowHandle {
    pub const fn new() -> Self {
        Self {
            raw: AtomicIsize::new(0),
            centered: AtomicBool::new(false),
        }
    }

    pub fn reset(&self) {
        self.raw.store(0, Ordering::Relaxed);
        self.centered.store(false, Ordering::Relaxed);
    }

    pub fn active(&self) -> isize {
        let cached = self.raw.load(Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        let raw = unsafe { GetActiveWindow() }.0 as isize;
        if raw != 0 {
            self.raw.store(raw, Ordering::Relaxed);
        }
        raw
    }

    /// Call after the first render has applied the window size. Later calls
    /// leave the window where the user moved it; no active window means retry.
    pub fn center_once(&self) {
        let raw = self.active();
        if raw != 0 && !self.centered.swap(true, Ordering::Relaxed) {
            common::window_placement::center(HWND(raw as *mut _));
        }
    }

    pub fn close(&self) {
        let raw = self.active();
        if raw != 0 {
            let hwnd = HWND(raw as *mut _);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}
