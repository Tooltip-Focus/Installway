// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shared Win32 helpers used by both installer UIs (full wizard + minimal updater).

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
};
use windows::Win32::UI::Shell::ExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, DispatchMessageW, GetDlgItem, GetMessageW, GetSystemMetrics, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, HICON, MSG, PostMessageW, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos, SetWindowTextW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_SETFONT,
};
use windows::core::PCWSTR;

/// App-defined window messages posted from the worker thread to the UI thread.
pub const WM_APP_PROGRESS: u32 = WM_APP + 1;
pub const WM_APP_DONE: u32 = WM_APP + 2;
pub const WM_APP_ERROR: u32 = WM_APP + 3;

/// Null-terminated UTF-16 from a `&str`, for Win32 `W` APIs.
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Register the progress-bar common control class.
pub fn init_progress_class() {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_PROGRESS_CLASS,
    };
    let _ = unsafe { InitCommonControlsEx(&icc) };
}

pub fn create_font(name: &str, height: i32, weight: i32) -> HFONT {
    let name_w = wide(name);
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            ((DEFAULT_PITCH.0 as u32) | ((FF_DONTCARE.0 as u32) << 4)) as u32,
            PCWSTR(name_w.as_ptr()),
        )
    }
}

/// This exe's own primary icon (the packaged app's, embedded at build time) for
/// the window/taskbar. Default `HICON` if absent.
pub unsafe fn own_icon() -> HICON {
    let Ok(exe) = std::env::current_exe() else {
        return HICON::default();
    };
    let w: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        ExtractIconW(Some(HINSTANCE(hmod.0)), PCWSTR(w.as_ptr()), 0)
    }
}

/// Total window size whose *client area* is `client_w` × `client_h` for the
/// given styles. Control layout uses client coords, so pass this to
/// `CreateWindowExW` to get the intended margins (the raw size would be the
/// outer rect, leaving the client ~16 px narrower / ~39 px shorter).
pub fn window_size_for_client(
    client_w: i32,
    client_h: i32,
    style: WINDOW_STYLE,
    ex: WINDOW_EX_STYLE,
) -> (i32, i32) {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    let _ = unsafe { AdjustWindowRectEx(&mut r, style, false, ex) };
    (r.right - r.left, r.bottom - r.top)
}

/// Center a top-level window on the primary monitor.
pub unsafe fn center(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    };
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            (sw - w) / 2,
            (sh - h) / 2,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

/// Set the font of a child control by dialog id.
pub unsafe fn set_font(parent: HWND, id: usize, font: HFONT) {
    unsafe {
        let h = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
        if !h.is_invalid() {
            SendMessageW(
                h,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

/// Set the text of a control by its own `HWND`.
pub unsafe fn set_window_text(ctrl: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        let _ = SetWindowTextW(ctrl, PCWSTR(w.as_ptr()));
    };
}

/// Set the text of a child control by dialog id.
pub unsafe fn set_dlg_text(parent: HWND, id: usize, s: &str) {
    let h = unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() };
    unsafe { set_window_text(h, s) };
}

/// Read a control's text by its own `HWND`.
pub unsafe fn get_window_text(ctrl: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(ctrl) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    unsafe { GetWindowTextW(ctrl, &mut buf) };
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end])
        .to_string_lossy()
        .into_owned()
}

/// `done/total` as a 0..=10000 fixed-point value for `PBM_SETPOS`.
pub fn scale_progress(done: u64, total: u64) -> i32 {
    let total = if total == 0 { 1 } else { total };
    ((done as u128 * 10000u128) / total as u128) as i32
}

/// Set a progress bar (by dialog id) to a 0..=10000 scaled value.
pub unsafe fn set_progress(parent: HWND, id: usize, scaled: i32) {
    unsafe {
        let bar = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
        SendMessageW(bar, PBM_SETRANGE32, Some(WPARAM(0)), Some(LPARAM(10000)));
        SendMessageW(
            bar,
            PBM_SETPOS,
            Some(WPARAM(scaled as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// Post a no-payload app message to a window thread (thread-safe FFI).
pub fn post(hwnd_isize: isize, msg: u32) {
    let _ = unsafe { PostMessageW(Some(HWND(hwnd_isize as *mut _)), msg, WPARAM(0), LPARAM(0)) };
}

/// Standard blocking message pump until `WM_QUIT`.
pub unsafe fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
