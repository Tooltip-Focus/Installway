// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use super::*;
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowPos, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

#[test]
fn initial_placement_waits_for_a_window_runs_once_and_resets_for_the_next_run() {
    let handle = WindowHandle::new();
    handle.center_once();
    assert!(!handle.centered.load(Ordering::Relaxed));

    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Installway lifecycle test"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            700,
            540,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        struct HiddenWindow(HWND);
        impl Drop for HiddenWindow {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let _window = HiddenWindow(hwnd);
        handle.raw.store(hwnd.0 as isize, Ordering::Relaxed);
        handle.center_once();
        let mut initial = RECT::default();
        GetWindowRect(hwnd, &mut initial).unwrap();

        SetWindowPos(
            hwnd,
            None,
            initial.left + 13,
            initial.top + 17,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
        handle.center_once();
        let mut moved = RECT::default();
        GetWindowRect(hwnd, &mut moved).unwrap();
        assert_eq!(
            (moved.left, moved.top),
            (initial.left + 13, initial.top + 17)
        );

        handle.reset();
        assert_eq!(handle.raw.load(Ordering::Relaxed), 0);
        assert!(!handle.centered.load(Ordering::Relaxed));
        handle.raw.store(hwnd.0 as isize, Ordering::Relaxed);
        handle.center_once();
        GetWindowRect(hwnd, &mut moved).unwrap();
        assert_eq!((moved.left, moved.top), (initial.left, initial.top));
    }
}
