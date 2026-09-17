// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Win32 window helpers shared by the installer and uninstaller UIs.
//!
//! Layout is in 96-dpi base units: controls are created without a position and
//! placed by [`move_controls`], which each window runs right after building its
//! controls and again on `WM_DPICHANGED`.

use crate::utils::wide;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, GetStockObject, HBRUSH, HFONT, OUT_DEFAULT_PRECIS, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Shell::ExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DispatchMessageW, GetDlgItem, GetMessageW, GetSystemMetrics,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HICON, HMENU, IDC_ARROW, LoadCursorW, MSG,
    MoveWindow, PostMessageW, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos, SetWindowTextW,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT, WM_SETICON, WNDCLASSEXW, WNDPROC,
    WS_CHILD, WS_TABSTOP,
};
use windows::core::PCWSTR;

/// A control's rectangle in 96-dpi base units: `(id, x, y, width, height)`.
pub type ControlRect = (usize, i32, i32, i32, i32);

/// Tab-stop button styles for [`child`].
pub const PUSH_BUTTON: WINDOW_STYLE = WINDOW_STYLE(WS_TABSTOP.0);
pub const DEFAULT_BUTTON: WINDOW_STYLE = WINDOW_STYLE(WS_TABSTOP.0 | 0x1);
pub const CHECKBOX: WINDOW_STYLE = WINDOW_STYLE(WS_TABSTOP.0 | 0x3);

const STM_SETICON: u32 = 0x0170;

/// Register the progress-bar common control class.
pub fn init_progress_class() {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_PROGRESS_CLASS,
    };
    let _ = unsafe { InitCommonControlsEx(&icc) };
}

/// Register the window class `class` for `wndproc`, then create a top-level
/// window of it whose client area is `client_w` × `client_h` at 96 dpi, showing
/// `icon` in the caption and taskbar. Not shown yet: size it with
/// [`fit_to_monitor`] first.
///
/// # Safety
/// `class` must point to a NUL-terminated UTF-16 string.
pub unsafe fn create_main_window(
    class: PCWSTR,
    wndproc: WNDPROC,
    title: &str,
    style: WINDOW_STYLE,
    client_w: i32,
    client_h: i32,
    icon: HICON,
) -> windows::core::Result<HWND> {
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(PCWSTR::null())?.0);
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: wndproc,
            hInstance: hinstance,
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            lpszClassName: class,
            hIconSm: icon,
            ..Default::default()
        });
        let title = wide(title);
        let ex = WINDOW_EX_STYLE(0);
        let (w, h) = window_size_for_client(client_w, client_h, style, ex, 96);
        let hwnd = CreateWindowExW(
            ex,
            class,
            PCWSTR(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            w,
            h,
            None,
            None,
            Some(hinstance),
            None,
        )?;
        if !icon.is_invalid() {
            // ICON_BIG, then ICON_SMALL.
            for size in [1, 0] {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(size)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
        Ok(hwnd)
    }
}

/// Size `hwnd` for the DPI of the monitor it opened on, so its client area is
/// `client_w` × `client_h` in 96-dpi units, and center it. Returns that DPI.
pub fn fit_to_monitor(hwnd: HWND, client_w: i32, client_h: i32, style: WINDOW_STYLE) -> i32 {
    unsafe {
        let dpi = dpi_for(hwnd);
        let (w, h) = window_size_for_client(
            scale(client_w, dpi),
            scale(client_h, dpi),
            style,
            WINDOW_EX_STYLE(0),
            dpi,
        );
        let _ = SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER);
        center(hwnd);
        dpi
    }
}

/// Handle `WM_DPICHANGED`: move `hwnd` to the rectangle Windows suggests in
/// `lparam` and return the new DPI from `wparam`. The caller then rebuilds its
/// fonts and layout at that DPI.
///
/// # Safety
/// `lparam` must be the `WM_DPICHANGED` one: a pointer to a `RECT`.
pub unsafe fn follow_dpi_change(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> i32 {
    unsafe {
        let rc = &*(lparam.0 as *const RECT);
        let _ = SetWindowPos(
            hwnd,
            None,
            rc.left,
            rc.top,
            rc.right - rc.left,
            rc.bottom - rc.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    ((wparam.0 >> 16) & 0xFFFF) as i32
}

/// Create a child control of `parent`. It has no size until [`move_controls`]
/// places it. The text is copied by Win32, so `text` need not outlive the call.
///
/// # Safety
/// `class` must point to a NUL-terminated UTF-16 string.
pub unsafe fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
) -> HWND {
    unsafe { child_ex(parent, WINDOW_EX_STYLE(0), class, text, style, id) }
}

/// [`child`] with extended styles.
///
/// # Safety
/// `class` must point to a NUL-terminated UTF-16 string.
pub unsafe fn child_ex(
    parent: HWND,
    ex: WINDOW_EX_STYLE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
) -> HWND {
    let text = wide(text);
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        CreateWindowExW(
            ex,
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut _)),
            Some(HINSTANCE(hinstance.0)),
            None,
        )
        .unwrap_or_default()
    }
}

/// Move each control of `parent` to its rectangle scaled to `dpi`. Controls
/// that do not exist are skipped.
pub fn move_controls(parent: HWND, dpi: i32, rects: &[ControlRect]) {
    let s = |v: i32| scale(v, dpi);
    for &(id, x, y, w, h) in rects {
        unsafe {
            let ctrl = GetDlgItem(Some(parent), id as i32).unwrap_or_default();
            if !ctrl.is_invalid() {
                let _ = MoveWindow(ctrl, s(x), s(y), s(w), s(h), true);
            }
        }
    }
}

/// Show `icon` in a `SS_ICON` static control. No-op when either handle is null.
pub fn set_static_icon(ctrl: HWND, icon: HICON) {
    if !ctrl.is_invalid() && !icon.is_invalid() {
        unsafe {
            SendMessageW(
                ctrl,
                STM_SETICON,
                Some(WPARAM(icon.0 as usize)),
                Some(LPARAM(0)),
            );
        }
    }
}

/// The DPI of the monitor `hwnd` is on (96 = 100% scale). Falls back to 96 if
/// the query fails. Scales the fixed-pixel layout per monitor so a move between
/// screens of different scale stays crisp (no bitmap stretch).
pub fn dpi_for(hwnd: HWND) -> i32 {
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
pub fn own_icon() -> HICON {
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
pub fn center(hwnd: HWND) {
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
pub fn set_font(parent: HWND, id: usize, font: HFONT) {
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
pub fn set_window_text(ctrl: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        let _ = SetWindowTextW(ctrl, PCWSTR(w.as_ptr()));
    };
}

/// Set the text of a child control by dialog id.
pub fn set_dlg_text(parent: HWND, id: usize, s: &str) {
    let h = unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() };
    set_window_text(h, s);
}

/// Read a control's text by its own `HWND`.
pub fn get_window_text(ctrl: HWND) -> String {
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
pub fn set_progress(parent: HWND, id: usize, scaled: i32) {
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
pub fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
