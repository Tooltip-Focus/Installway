// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Win32 UI for the uninstaller. Two phases sharing one HWND:
//!   - `Confirm` - title + product info + Yes / No buttons
//!   - `Progress` - title + progress bar + status label
//!
//! Identical visual style as the installer (Segoe UI, banner strip, ~700×400).

use common::utils::wide;
use std::cell::RefCell;
use std::os::windows::ffi::OsStringExt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_PITCH, DeleteObject, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, GetStockObject, HBRUSH,
    HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, SetBkMode, SetTextColor, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
    PROGRESS_CLASSW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const BS_PUSHBUTTON: u32 = 0x0;
const BS_DEFPUSHBUTTON: u32 = 0x1;

const ID_HEADER: usize = 1001;
const ID_SUBHEADER: usize = 1002;
const ID_BANNER: usize = 1003;
const ID_CONFIRM_TEXT: usize = 1004;
const ID_YES_BTN: usize = 1005;
const ID_NO_BTN: usize = 1006;
const ID_PROGRESS: usize = 1007;
const ID_STATUS: usize = 1008;

const WM_APP_PROGRESS: u32 = WM_APP + 1;
const WM_APP_DONE: u32 = WM_APP + 2;

const WIN_W: i32 = 600;
const WIN_H: i32 = 360;
const BANNER_H: i32 = 72;
const PAD: i32 = 24;
const BANNER_BG: u32 = 0x00F3F3F3;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Confirm,
    Progress,
    Done,
}

struct State {
    phase: Phase,
    progress_done: u64,
    progress_total: u64,
    status: String,
    font_body: HFONT,
    font_header: HFONT,
    banner_brush: HBRUSH,
    card_brush: HBRUSH,
    yes_clicked: bool,
    /// Current monitor DPI (96 = 100%); updated on `WM_DPICHANGED`.
    dpi: i32,
}

thread_local! {
    static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
    static T: RefCell<common::i18n::Translator> = RefCell::new(common::i18n::Translator::default());
}

pub fn set_translator(tr: common::i18n::Translator) {
    T.with(|t| *t.borrow_mut() = tr);
}

pub fn tr() -> common::i18n::Translator {
    T.with(|t| *t.borrow())
}

pub struct UninstallParams {
    pub title: String,
    pub subtitle: String,
    pub confirm_text: String,
    /// Worker invoked after Yes; must call `progress` and finish.
    pub worker: Worker,
    /// If true, the window skips the Confirm phase and starts the worker as
    /// soon as the message loop is running. Used by Stage 2 (no user choice).
    pub auto_start: bool,
}

/// Show a single window driving the whole uninstall.
/// Returns true if user confirmed and the worker ran (the worker may still
/// have produced internal errors - those are reported via the status label).
pub fn run(params: UninstallParams) -> bool {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        let _ = InitCommonControlsEx(&icc);
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();

        // Own embedded icon (the app's icon, stamped into uninstall.exe at build).
        let hicon = own_icon();

        let class_name = w!("RustUninstallerWnd");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(hinstance.0),
            hIcon: hicon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: hicon,
        };
        RegisterClassExW(&wc);

        let title_w = wide(&params.title);
        let state = Rc::new(RefCell::new(State {
            phase: Phase::Confirm,
            progress_done: 0,
            progress_total: 0,
            status: String::new(),
            font_body: create_font("Segoe UI", 16, FW_NORMAL.0 as i32),
            font_header: create_font("Segoe UI Semibold", 22, FW_SEMIBOLD.0 as i32),
            banner_brush: CreateSolidBrush(COLORREF(BANNER_BG)),
            card_brush: CreateSolidBrush(COLORREF(0x00FFFFFF)),
            yes_clicked: false,
            dpi: 96,
        }));
        STATE.with(|s| *s.borrow_mut() = Some(state.clone()));

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_w.as_ptr()),
            WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIN_W,
            WIN_H,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return false,
        };

        if !hicon.is_invalid() {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(1)),
                Some(LPARAM(hicon.0 as isize)),
            );
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(0)),
                Some(LPARAM(hicon.0 as isize)),
            );
        }

        // Scale to the monitor this window opened on (per-monitor DPI aware):
        // resize, rebuild fonts, lay out at that DPI - so a move to a screen of
        // different scale stays crisp instead of dropping/clipping controls.
        let dpi = dpi_for(hwnd);
        rebuild_fonts(dpi);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            scale(WIN_W, dpi),
            scale(WIN_H, dpi),
            SWP_NOMOVE | SWP_NOZORDER,
        );
        center(hwnd);
        build_controls(hwnd, &params);
        relayout(hwnd, dpi);
        if params.auto_start {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    st.borrow_mut().yes_clicked = true;
                }
            });
            apply_phase(hwnd, Phase::Progress);
        } else {
            apply_phase(hwnd, Phase::Confirm);
        }

        let _ = ShowWindow(hwnd, SW_SHOW);

        let mut worker_holder: Option<Worker> = Some(params.worker);
        let hwnd_isize = hwnd.0 as isize;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            // Lazy-start the worker only after the Yes button click flips state.
            let started = STATE.with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|st| st.borrow().yes_clicked)
                    .unwrap_or(false)
            });
            if started && let Some(w) = worker_holder.take() {
                thread::spawn(move || {
                    let progress: Progress = Arc::new(move |done, total, name| {
                        set_progress(done, total, name);
                        let _ = PostMessageW(
                            Some(HWND(hwnd_isize as *mut _)),
                            WM_APP_PROGRESS,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    });
                    w(progress);
                    let _ = PostMessageW(
                        Some(HWND(hwnd_isize as *mut _)),
                        WM_APP_DONE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                });
            }
        }

        STATE.with(|s| {
            s.borrow()
                .as_ref()
                .map(|st| st.borrow().yes_clicked)
                .unwrap_or(false)
        })
    }
}

/// Load this exe's own primary icon (stamped at build) for window + taskbar.
unsafe fn own_icon() -> windows::Win32::UI::WindowsAndMessaging::HICON {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::ExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::HICON;
    let Ok(exe) = std::env::current_exe() else {
        return HICON::default();
    };
    let w: Vec<u16> = std::ffi::OsStr::new(&exe)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        ExtractIconW(Some(HINSTANCE(hmod.0)), PCWSTR(w.as_ptr()), 0)
    }
}

fn create_font(name: &str, height: i32, weight: i32) -> HFONT {
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

unsafe fn apply_font(hwnd: HWND, id: usize, font: HFONT) {
    unsafe {
        let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
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

/// Scale a 96-dpi base measurement to the given DPI.
fn scale(v: i32, dpi: i32) -> i32 {
    v * dpi / 96
}

/// The DPI of the monitor `hwnd` is on (96 = 100%). 96 fallback on failure.
unsafe fn dpi_for(hwnd: HWND) -> i32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96 } else { d as i32 }
}

/// Recreate both fonts at `dpi` (deleting the old) and store them.
unsafe fn rebuild_fonts(dpi: i32) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let mut st = state.borrow_mut();
            unsafe {
                let _ = DeleteObject(st.font_body.into());
                let _ = DeleteObject(st.font_header.into());
            }
            st.font_body = create_font("Segoe UI", scale(16, dpi), FW_NORMAL.0 as i32);
            st.font_header = create_font("Segoe UI Semibold", scale(22, dpi), FW_SEMIBOLD.0 as i32);
            st.dpi = dpi;
        }
    });
}

/// (Re)apply the stored fonts to the controls.
unsafe fn apply_fonts(hwnd: HWND) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let st = state.borrow();
            unsafe {
                apply_font(hwnd, ID_HEADER, st.font_header);
                for id in [
                    ID_SUBHEADER,
                    ID_CONFIRM_TEXT,
                    ID_PROGRESS,
                    ID_STATUS,
                    ID_YES_BTN,
                    ID_NO_BTN,
                ] {
                    apply_font(hwnd, id, st.font_body);
                }
            }
        }
    });
}

/// Reposition + resize every control for `dpi` (96-dpi base units, scaled).
/// Run after creation and on each `WM_DPICHANGED`. Identity at 96 dpi.
unsafe fn relayout(hwnd: HWND, dpi: i32) {
    let s = |v: i32| scale(v, dpi);
    let btn_y = WIN_H - 84;
    let items: &[(usize, i32, i32, i32, i32)] = &[
        (ID_BANNER, 0, 0, WIN_W, BANNER_H),
        (ID_HEADER, PAD, 16, WIN_W - PAD * 2, 28),
        (ID_SUBHEADER, PAD, 46, WIN_W - PAD * 2, 20),
        (ID_CONFIRM_TEXT, PAD, BANNER_H + PAD, WIN_W - PAD * 2, 120),
        (ID_PROGRESS, PAD, BANNER_H + PAD + 16, WIN_W - PAD * 2, 22),
        (ID_STATUS, PAD, BANNER_H + PAD + 48, WIN_W - PAD * 2, 48),
        (ID_YES_BTN, WIN_W - PAD - 260, btn_y, 140, 32),
        (ID_NO_BTN, WIN_W - PAD - 110, btn_y, 110, 32),
    ];
    unsafe {
        for &(id, x, y, w, h) in items {
            let ctrl = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
            if !ctrl.is_invalid() {
                let _ = MoveWindow(ctrl, s(x), s(y), s(w), s(h), true);
            }
        }
    }
}

unsafe fn build_controls(hwnd: HWND, p: &UninstallParams) {
    let hinst = unsafe { GetModuleHandleW(PCWSTR::null()).unwrap_or_default() };
    let hinst = HINSTANCE(hinst.0);

    let header_w = wide(&p.title);
    let sub_w = wide(&p.subtitle);
    let confirm_w = wide(&p.confirm_text);
    let yes_w = wide(&tr().get("uninstall.yes"));
    let no_w = wide(&tr().get("uninstall.no"));

    unsafe {
        // Banner
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_VISIBLE | WS_CHILD,
            0,
            0,
            WIN_W,
            BANNER_H,
            Some(hwnd),
            Some(HMENU(ID_BANNER as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(header_w.as_ptr()),
            WS_VISIBLE | WS_CHILD,
            PAD,
            16,
            WIN_W - PAD * 2,
            28,
            Some(hwnd),
            Some(HMENU(ID_HEADER as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(sub_w.as_ptr()),
            WS_VISIBLE | WS_CHILD,
            PAD,
            46,
            WIN_W - PAD * 2,
            20,
            Some(hwnd),
            Some(HMENU(ID_SUBHEADER as *mut _)),
            Some(hinst),
            None,
        );

        // Confirm phase
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(confirm_w.as_ptr()),
            WS_CHILD,
            PAD,
            BANNER_H + PAD,
            WIN_W - PAD * 2,
            120,
            Some(hwnd),
            Some(HMENU(ID_CONFIRM_TEXT as *mut _)),
            Some(hinst),
            None,
        );

        // Progress phase
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PROGRESS_CLASSW,
            PCWSTR::null(),
            WS_CHILD,
            PAD,
            BANNER_H + PAD + 16,
            WIN_W - PAD * 2,
            22,
            Some(hwnd),
            Some(HMENU(ID_PROGRESS as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_CHILD,
            PAD,
            BANNER_H + PAD + 48,
            WIN_W - PAD * 2,
            48,
            Some(hwnd),
            Some(HMENU(ID_STATUS as *mut _)),
            Some(hinst),
            None,
        );

        // Buttons
        let btn_y = WIN_H - 84;
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(yes_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON),
            WIN_W - PAD - 260,
            btn_y,
            140,
            32,
            Some(hwnd),
            Some(HMENU(ID_YES_BTN as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(no_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
            WIN_W - PAD - 110,
            btn_y,
            110,
            32,
            Some(hwnd),
            Some(HMENU(ID_NO_BTN as *mut _)),
            Some(hinst),
            None,
        );
    }

    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let st = state.borrow();
            unsafe {
                apply_font(hwnd, ID_HEADER, st.font_header);
                for id in [
                    ID_SUBHEADER,
                    ID_CONFIRM_TEXT,
                    ID_PROGRESS,
                    ID_STATUS,
                    ID_YES_BTN,
                    ID_NO_BTN,
                ] {
                    apply_font(hwnd, id, st.font_body);
                }
            }
        }
    });
}

unsafe fn apply_phase(hwnd: HWND, phase: Phase) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            state.borrow_mut().phase = phase;
        }
    });
    let show = |id: usize, vis: bool| unsafe {
        let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
        let _ = ShowWindow(h, if vis { SW_SHOW } else { SW_HIDE });
    };
    match phase {
        Phase::Confirm => {
            show(ID_CONFIRM_TEXT, true);
            show(ID_YES_BTN, true);
            show(ID_NO_BTN, true);
            show(ID_PROGRESS, false);
            show(ID_STATUS, false);
        }
        Phase::Progress | Phase::Done => {
            show(ID_CONFIRM_TEXT, false);
            show(ID_YES_BTN, false);
            show(ID_NO_BTN, false);
            show(ID_PROGRESS, true);
            show(ID_STATUS, true);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DPICHANGED => unsafe {
            // Moved to a monitor of different scale: resize to the suggested
            // rect, rebuild fonts + lay out at the new DPI, repaint.
            let new_dpi = ((wparam.0 >> 16) & 0xFFFF) as i32;
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
            rebuild_fonts(new_dpi);
            apply_fonts(hwnd);
            relayout(hwnd, new_dpi);
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        },
        WM_CTLCOLORSTATIC => unsafe {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
            let ctrl = HWND(lparam.0 as *mut _);
            let banner = GetDlgItem(Some(hwnd), ID_BANNER as i32).unwrap_or_default();
            let header = GetDlgItem(Some(hwnd), ID_HEADER as i32).unwrap_or_default();
            let sub = GetDlgItem(Some(hwnd), ID_SUBHEADER as i32).unwrap_or_default();
            let _ = SetBkMode(hdc, TRANSPARENT);
            if ctrl == banner || ctrl == header || ctrl == sub {
                SetTextColor(hdc, COLORREF(0x00333333));
                return LRESULT(STATE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .map(|st| st.borrow().banner_brush.0 as isize)
                        .unwrap_or(0)
                }));
            }
            LRESULT(STATE.with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|st| st.borrow().card_brush.0 as isize)
                    .unwrap_or(0)
            }))
        },
        WM_COMMAND => unsafe {
            let id = wparam.0 & 0xFFFF;
            match id {
                ID_YES_BTN => {
                    STATE.with(|s| {
                        if let Some(st) = s.borrow().as_ref() {
                            st.borrow_mut().yes_clicked = true;
                        }
                    });
                    apply_phase(hwnd, Phase::Progress);
                }
                ID_NO_BTN => {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                _ => {}
            }
            LRESULT(0)
        },
        m if m == WM_APP_PROGRESS => unsafe {
            update_progress(hwnd);
            LRESULT(0)
        },
        m if m == WM_APP_DONE => unsafe {
            apply_phase(hwnd, Phase::Done);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        },
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            STATE.with(|s| {
                if let Some(state) = s.borrow().as_ref() {
                    let st = state.borrow();
                    let _ = DeleteObject(st.font_body.into());
                    let _ = DeleteObject(st.font_header.into());
                    let _ = DeleteObject(st.banner_brush.into());
                    let _ = DeleteObject(st.card_brush.into());
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn set_progress(done: u64, total: u64, name: &str) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let mut st = state.borrow_mut();
            st.progress_done = done;
            st.progress_total = total;
            st.status = name.to_string();
        }
    });
}

unsafe fn update_progress(hwnd: HWND) {
    STATE.with(|s| {
        let Some(state) = s.borrow().as_ref().cloned() else {
            return;
        };
        let st = state.borrow();
        let bar = unsafe { GetDlgItem(Some(hwnd), ID_PROGRESS as i32).unwrap_or_default() };
        let label = unsafe { GetDlgItem(Some(hwnd), ID_STATUS as i32).unwrap_or_default() };
        let total = if st.progress_total == 0 {
            1
        } else {
            st.progress_total
        };
        let scaled = ((st.progress_done as u128 * 10000u128) / total as u128) as i32;
        unsafe {
            SendMessageW(bar, PBM_SETRANGE32, Some(WPARAM(0)), Some(LPARAM(10000)));
            SendMessageW(
                bar,
                PBM_SETPOS,
                Some(WPARAM(scaled as usize)),
                Some(LPARAM(0)),
            );
            let label_text = wide(&st.status);
            let _ = SetWindowTextW(label, PCWSTR(label_text.as_ptr()));
        }
    });
}

unsafe fn center(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    };
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
    }
}

pub fn fatal(msg: &str) {
    let t = wide(msg);
    let c = wide(&tr().get("uninstall.fatal_caption"));
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// Modal info dialog (e.g. the "uninstall complete" confirmation).
pub fn info(msg: &str, caption: &str) {
    let t = wide(msg);
    let c = wide(caption);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

#[allow(dead_code)]
pub fn os_string_from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    std::ffi::OsString::from_wide(&buf[..end])
        .to_string_lossy()
        .into_owned()
}

/// Progress callback type alias (shared installer/uninstaller shape).
pub type Progress = common::ProgressFn;

/// The uninstall worker: runs after Yes, driving `progress` to completion.
pub type Worker = Box<dyn FnOnce(Progress) + Send>;

/// Tracker counts as a convenience for stages that want a step-based bar.
pub struct StepCounter {
    pub done: AtomicU64,
    pub total: u64,
    pub cb: Progress,
    #[allow(dead_code)]
    pub cancelled: Arc<AtomicBool>,
}

impl StepCounter {
    pub fn new(total: u64, cb: Progress) -> Self {
        Self {
            done: AtomicU64::new(0),
            total,
            cb,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn step(&self, label: &str) {
        let d = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        (self.cb)(d, self.total, label);
    }
    pub fn report(&self, label: &str) {
        let d = self.done.load(Ordering::Relaxed);
        (self.cb)(d, self.total, label);
    }
}
