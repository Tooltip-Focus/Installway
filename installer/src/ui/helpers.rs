// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shared Win32 helpers used by both installer UIs (full wizard + minimal updater).

use common::utils::wide;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Shell::ExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetDlgItem, GetMessageW, GetSystemMetrics, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, HICON, MSG, PostMessageW, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos, SetWindowTextW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_SETFONT,
};
use windows::core::PCWSTR;

/// App-defined window messages posted from the worker thread to the UI thread.
pub const WM_APP_PROGRESS: u32 = WM_APP + 1;
pub const WM_APP_DONE: u32 = WM_APP + 2;
pub const WM_APP_ERROR: u32 = WM_APP + 3;
/// Posted by the background plugin-step query thread when a result is ready.
pub const WM_APP_PLUGIN_STEP: u32 = WM_APP + 4;
/// Posted by the progress pipe reader thread; WPARAM carries the 0–100 value.
pub const WM_APP_PLUGIN_PROGRESS: u32 = WM_APP + 5;
/// Permission denied on the install dir; elevation may help. LPARAM is a
/// `Box<PermErrorPayload>` (path + plugin_inputs).
pub const WM_APP_PERM_ERROR: u32 = WM_APP + 6;
/// UAC was cancelled or the elevated worker failed to start. LPARAM is a
/// `Box<PathBuf>` (the rejected install dir, for go-back-to-Choose).
pub const WM_APP_PERM_DENIED: u32 = WM_APP + 7;
/// The user confirmed cancellation from the window's close button; the worker
/// has rolled the install back, so the window can close cleanly (no error page).
pub const WM_APP_CANCELLED: u32 = WM_APP + 8;

/// Register the progress-bar common control class.
pub fn init_progress_class() {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_PROGRESS_CLASS,
    };
    let _ = unsafe { InitCommonControlsEx(&icc) };
}

/// The DPI of the monitor `hwnd` is on (96 = 100% scale). Falls back to 96 if
/// the query fails. Scales the fixed-pixel layout per monitor so a move between
/// screens of different scale stays crisp (no bitmap stretch).
pub unsafe fn dpi_for(hwnd: HWND) -> i32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96 } else { d as i32 }
}

/// Scale a 96-dpi base measurement to the given DPI.
pub fn scale(v: i32, dpi: i32) -> i32 {
    v * dpi / 96
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
            (DEFAULT_PITCH.0 as u32) | ((FF_DONTCARE.0 as u32) << 4),
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
    let w = wide(&exe.to_string_lossy());
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        ExtractIconW(Some(HINSTANCE(hmod.0)), PCWSTR(w.as_ptr()), 0)
    }
}

/// Total window size whose *client area* is `client_w` × `client_h` for the
/// given styles at `dpi`. Control layout uses client coords, so pass this to
/// `CreateWindowExW` to get the intended margins (the raw size would be the
/// outer rect, leaving the client ~16 px narrower / ~39 px shorter).
///
/// Uses `AdjustWindowRectExForDpi` rather than the DPI-unaware
/// `AdjustWindowRectEx`: at 150 %+ the caption/borders are much taller than
/// their 96-dpi size, so the 96-dpi calculation underestimates the non-client
/// area and leaves the client too short — clipping the bottom controls
/// (progress bar, status, buttons). Pass the monitor DPI so the frame is sized
/// for the scale the layout is built at. At 96 dpi this matches the old result.
pub fn window_size_for_client(
    client_w: i32,
    client_h: i32,
    style: WINDOW_STYLE,
    ex: WINDOW_EX_STYLE,
    dpi: i32,
) -> (i32, i32) {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    let _ = unsafe { AdjustWindowRectExForDpi(&mut r, style, false, ex, dpi as u32) };
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
    String::from_utf16_lossy(&buf[..end])
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

/// Post a message carrying a value in WPARAM (thread-safe FFI).
pub fn post_wparam(hwnd_isize: isize, msg: u32, wparam: usize) {
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_isize as *mut _)),
            msg,
            WPARAM(wparam),
            LPARAM(0),
        )
    };
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
