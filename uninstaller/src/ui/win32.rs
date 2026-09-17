// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Win32 fallback UI for the uninstaller. Two phases sharing one HWND:
//!   - `Confirm` - title + product info + Yes / No buttons
//!   - `Progress` - title + progress bar + status label
//!
//! Identical visual style as the installer (Segoe UI, banner strip, ~700×400).

use super::progress::{self, ProgressStore};
use super::{UninstallParams, Worker, tr};
use common::win32::{
    ControlRect, DEFAULT_BUTTON, PUSH_BUTTON, child, create_font, create_main_window,
    fit_to_monitor, follow_dpi_change, init_progress_class, own_icon, post, scale, scale_progress,
    set_dlg_text, set_font, set_progress,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT, InvalidateRect,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::PROGRESS_CLASSW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const ID_HEADER: usize = 1001;
const ID_SUBHEADER: usize = 1002;
const ID_BANNER: usize = 1003;
const ID_CONFIRM_TEXT: usize = 1004;
const ID_YES_BTN: usize = 1005;
const ID_NO_BTN: usize = 1006;
const ID_PROGRESS: usize = 1007;
const ID_STATUS: usize = 1008;
const ID_PROGRESS_TIMER: usize = 1;
const PROGRESS_POLL_MS: u32 = 50;

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
    font_body: HFONT,
    font_header: HFONT,
    banner_brush: HBRUSH,
    card_brush: HBRUSH,
    yes_clicked: bool,
    progress: ProgressStore,
    /// Current monitor DPI (96 = 100%); updated on `WM_DPICHANGED`.
    dpi: i32,
}

thread_local! {
    static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Show a single window driving the whole uninstall.
/// Returns true if user confirmed and the worker ran (the worker may still
/// have produced internal errors - those are reported via the status label).
pub(super) fn run(params: UninstallParams) -> bool {
    unsafe {
        init_progress_class();
        let progress_store = ProgressStore::default();
        let state = Rc::new(RefCell::new(State {
            phase: Phase::Confirm,
            font_body: create_font("Segoe UI", 16, FW_NORMAL.0 as i32),
            font_header: create_font("Segoe UI Semibold", 22, FW_SEMIBOLD.0 as i32),
            banner_brush: CreateSolidBrush(COLORREF(BANNER_BG)),
            card_brush: CreateSolidBrush(COLORREF(0x00FFFFFF)),
            yes_clicked: false,
            progress: progress_store.clone(),
            dpi: 96,
        }));
        STATE.with(|s| *s.borrow_mut() = Some(state.clone()));

        let style = WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION;
        // Own embedded icon (the app's icon, stamped into uninstall.exe at build).
        let Ok(hwnd) = create_main_window(
            w!("RustUninstallerWnd"),
            Some(wndproc),
            &params.title,
            style,
            WIN_W,
            WIN_H,
            own_icon(),
        ) else {
            return false;
        };

        // Scale to the monitor this window opened on (per-monitor DPI aware):
        // resize, rebuild fonts, lay out at that DPI - so a move to a screen of
        // different scale stays crisp instead of dropping/clipping controls.
        let dpi = fit_to_monitor(hwnd, WIN_W, WIN_H, style);
        rebuild_fonts(dpi);
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
                let progress = progress::callback(&progress_store);
                thread::spawn(move || {
                    let _ = w.run(progress);
                    post(hwnd_isize, WM_APP_DONE);
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
            set_font(hwnd, ID_HEADER, st.font_header);
            for id in [
                ID_SUBHEADER,
                ID_CONFIRM_TEXT,
                ID_PROGRESS,
                ID_STATUS,
                ID_YES_BTN,
                ID_NO_BTN,
            ] {
                set_font(hwnd, id, st.font_body);
            }
        }
    });
}

/// Every control's rectangle, in 96-dpi base units: the only place the layout
/// lives.
const LAYOUT: &[ControlRect] = &[
    (ID_BANNER, 0, 0, WIN_W, BANNER_H),
    (ID_HEADER, PAD, 16, WIN_W - PAD * 2, 28),
    (ID_SUBHEADER, PAD, 46, WIN_W - PAD * 2, 20),
    (ID_CONFIRM_TEXT, PAD, BANNER_H + PAD, WIN_W - PAD * 2, 120),
    (ID_PROGRESS, PAD, BANNER_H + PAD + 16, WIN_W - PAD * 2, 22),
    (ID_STATUS, PAD, BANNER_H + PAD + 48, WIN_W - PAD * 2, 48),
    (ID_YES_BTN, WIN_W - PAD - 260, WIN_H - 84, 140, 32),
    (ID_NO_BTN, WIN_W - PAD - 110, WIN_H - 84, 110, 32),
];

/// Place every control for `dpi`. Run after creation and on each
/// `WM_DPICHANGED`.
unsafe fn relayout(hwnd: HWND, dpi: i32) {
    common::win32::move_controls(hwnd, dpi, LAYOUT)
}

unsafe fn build_controls(hwnd: HWND, p: &UninstallParams) {
    let hidden = WINDOW_STYLE(0);
    unsafe {
        child(hwnd, w!("STATIC"), "", WS_VISIBLE, ID_BANNER);
        child(hwnd, w!("STATIC"), &p.title, WS_VISIBLE, ID_HEADER);
        child(hwnd, w!("STATIC"), &p.subtitle, WS_VISIBLE, ID_SUBHEADER);
        // Confirm phase
        child(hwnd, w!("STATIC"), &p.confirm_text, hidden, ID_CONFIRM_TEXT);
        // Progress phase
        child(hwnd, PROGRESS_CLASSW, "", hidden, ID_PROGRESS);
        child(hwnd, w!("STATIC"), "", hidden, ID_STATUS);
        // Buttons
        let (yes, no) = (tr().get("uninstall.yes"), tr().get("uninstall.no"));
        child(hwnd, w!("BUTTON"), &yes, DEFAULT_BUTTON, ID_YES_BTN);
        child(hwnd, w!("BUTTON"), &no, PUSH_BUTTON, ID_NO_BTN);
        apply_fonts(hwnd);
    }
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
            let _ = unsafe { KillTimer(Some(hwnd), ID_PROGRESS_TIMER) };
            show(ID_CONFIRM_TEXT, true);
            show(ID_YES_BTN, true);
            show(ID_NO_BTN, true);
            show(ID_PROGRESS, false);
            show(ID_STATUS, false);
        }
        Phase::Progress => {
            show(ID_CONFIRM_TEXT, false);
            show(ID_YES_BTN, false);
            show(ID_NO_BTN, false);
            show(ID_PROGRESS, true);
            show(ID_STATUS, true);
            let _ = unsafe { SetTimer(Some(hwnd), ID_PROGRESS_TIMER, PROGRESS_POLL_MS, None) };
        }
        Phase::Done => {
            let _ = unsafe { KillTimer(Some(hwnd), ID_PROGRESS_TIMER) };
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
            let new_dpi = follow_dpi_change(hwnd, wparam, lparam);
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
        WM_TIMER if wparam.0 == ID_PROGRESS_TIMER => unsafe {
            update_progress(hwnd);
            LRESULT(0)
        },
        m if m == WM_APP_DONE => unsafe {
            update_progress(hwnd);
            apply_phase(hwnd, Phase::Done);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        },
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), ID_PROGRESS_TIMER);
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

unsafe fn update_progress(hwnd: HWND) {
    let progress = STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|state| state.borrow().progress.get())
            .unwrap_or_default()
    });
    set_progress(
        hwnd,
        ID_PROGRESS,
        scale_progress(progress.done, progress.total),
    );
    set_dlg_text(hwnd, ID_STATUS, &progress.name);
}
