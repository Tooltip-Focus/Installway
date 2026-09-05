// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_HIDE, ShowWindow, WM_CLOSE};

/// Cache a Reactor window handle for access from its frontend and workers.
#[derive(Default)]
pub struct WindowHandle(AtomicIsize);

impl WindowHandle {
    pub const fn new() -> Self {
        Self(AtomicIsize::new(0))
    }

    pub fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }

    pub fn active(&self) -> isize {
        let cached = self.0.load(Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        let raw = unsafe { GetActiveWindow() }.0 as isize;
        if raw != 0 {
            self.0.store(raw, Ordering::Relaxed);
        }
        raw
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
