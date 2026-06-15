// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Full installer wizard. Phases: License → Choose → Progress → Done/Error.
//!
//! `mod.rs` owns the window, shared state, message loop and phase switching;
//! [`views`] builds the controls for each phase; [`handlers`] runs the button
//! and worker logic.

mod handlers;
mod views;

use crate::payload::LoadedPayload;
use crate::ui::helpers;
use anyhow::Result;
use common::utils::wide;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, GetStockObject, HBRUSH, HFONT,
    InvalidateRect, SetBkMode, SetTextColor, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub(super) const BM_GETCHECK: u32 = 0x00F0;
pub(super) const BM_SETCHECK: u32 = 0x00F1;

pub(super) const ID_PATH_EDIT: usize = 1001;
pub(super) const ID_BROWSE_BTN: usize = 1002;
pub(super) const ID_INSTALL_BTN: usize = 1003;
pub(super) const ID_CANCEL_BTN: usize = 1004;
pub(super) const ID_PROGRESS: usize = 1005;
pub(super) const ID_STATUS: usize = 1006;
pub(super) const ID_HEADER: usize = 1007;
pub(super) const ID_SUBHEADER: usize = 1008;
pub(super) const ID_PATH_LABEL: usize = 1009;
pub(super) const ID_CLOSE_BTN: usize = 1010;
pub(super) const ID_LICENSE_EDIT: usize = 1011;
pub(super) const ID_ACCEPT_CHK: usize = 1012;
pub(super) const ID_NEXT_BTN: usize = 1013;
pub(super) const ID_BACK_BTN: usize = 1014;
pub(super) const ID_LAUNCH_CHK: usize = 1015;
pub(super) const ID_BANNER: usize = 1016;
pub(super) const ID_PATH_WARN: usize = 1017;
pub(super) const ID_PATH_WARN_ICON: usize = 1018;

pub(super) const WIN_W: i32 = 700;
pub(super) const WIN_H: i32 = 500;
pub(super) const BANNER_H: i32 = 72;
pub(super) const PAD: i32 = 24;

const ACCENT_LIGHT: u32 = 0x00F3F3F3; // light gray banner card

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Phase {
    License,
    Choose,
    Progress,
    Done,
    Error,
}

pub(super) struct ProgressState {
    pub done: u64,
    pub total: u64,
    pub name: String,
}

pub(super) struct UiState {
    pub phase: Phase,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<std::sync::Mutex<ProgressState>>,
    pub error_text: String,
    pub font_normal: HFONT,
    pub font_bold: HFONT,
    pub font_header: HFONT,
    pub banner_brush: HBRUSH,
    pub card_brush: HBRUSH,
    pub license_accepted: bool,
    pub chosen_path: Option<PathBuf>,
    /// Current monitor DPI (96 = 100%); drives scaled layout + fonts, updated on
    /// `WM_DPICHANGED`.
    pub dpi: i32,
}

thread_local! {
    pub(super) static STATE: RefCell<Option<Rc<RefCell<UiState>>>> = const { RefCell::new(None) };
    pub(super) static PAYLOAD: RefCell<Option<common::models::InstallerPayload>> = const { RefCell::new(None) };
    pub(super) static UNINSTALLER: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    pub(super) static LAUNCH_FLAG: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_LICENSE: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_PATH: RefCell<bool> = const { RefCell::new(false) };
    static T: RefCell<common::i18n::Translator> = RefCell::new(common::i18n::Translator::default());
}

fn skip_license() -> bool {
    SKIP_LICENSE.with(|s| *s.borrow())
}
fn skip_path() -> bool {
    SKIP_PATH.with(|s| *s.borrow())
}

pub(super) fn tr() -> common::i18n::Translator {
    T.with(|t| *t.borrow())
}

pub fn run(
    loaded: LoadedPayload,
    default_path: PathBuf,
    launch_flag: bool,
    already_installed: bool,
    translator: common::i18n::Translator,
) -> Result<()> {
    // An existing install fixes the target folder: a patch must go there, and a
    // full reinstall/upgrade should too (no accidental second copy). So the
    // Choose page is always skipped when already installed, regardless of the
    // build-time `skip_path`. `default_path` is already the prior folder.
    let skip_license = loaded.payload.skip_license;
    let skip_path = loaded.payload.skip_path || already_installed;

    PAYLOAD.with(|p| *p.borrow_mut() = Some(loaded.payload.clone()));
    UNINSTALLER.with(|u| *u.borrow_mut() = Some(loaded.uninstaller_bytes.clone()));
    LAUNCH_FLAG.with(|l| *l.borrow_mut() = launch_flag);
    SKIP_LICENSE.with(|s| *s.borrow_mut() = skip_license);
    SKIP_PATH.with(|s| *s.borrow_mut() = skip_path);
    T.with(|t| *t.borrow_mut() = translator);

    unsafe {
        let hwnd = create_window(&loaded.payload, &default_path)?;
        // Pick the first non-skipped page; if both are skipped there is no user
        // step, so install immediately to the default path.
        if skip_license && skip_path {
            apply_phase(hwnd, Phase::Progress);
            let _ = ShowWindow(hwnd, SW_SHOW);
            handlers::on_install(hwnd);
        } else {
            apply_phase(
                hwnd,
                if skip_license {
                    Phase::Choose
                } else {
                    Phase::License
                },
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        helpers::pump_messages();
    }
    Ok(())
}

/// Register the class, build the window + all controls, install state. Shared by
/// the real `run` and the dev-only `preview`. Does not show the window or set a
/// phase.
unsafe fn create_window(
    payload: &common::models::InstallerPayload,
    default_path: &Path,
) -> Result<HWND> {
    unsafe {
        helpers::init_progress_class();
        let hinstance = GetModuleHandleW(PCWSTR::null())?;
        let hicon = helpers::own_icon();

        let class_name = w!("InstallwayWnd");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(hinstance.0),
            hIcon: hicon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: hicon,
        };
        RegisterClassExW(&wc);

        let font_normal = helpers::create_font("Segoe UI", 16, FW_NORMAL.0 as i32);
        let font_bold = helpers::create_font("Segoe UI", 16, FW_SEMIBOLD.0 as i32);
        let font_header = helpers::create_font("Segoe UI Semibold", 22, FW_SEMIBOLD.0 as i32);
        let banner_brush = CreateSolidBrush(COLORREF(ACCENT_LIGHT));
        let card_brush = CreateSolidBrush(COLORREF(0x00FFFFFF));

        let title = wide(&tr().fmt(
            "install.window_title",
            &[
                ("product", &payload.product),
                ("version", &payload.to_version),
            ],
        ));

        let state = Rc::new(RefCell::new(UiState {
            phase: Phase::License,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(std::sync::Mutex::new(ProgressState {
                done: 0,
                total: 0,
                name: String::new(),
            })),
            error_text: String::new(),
            font_normal,
            font_bold,
            font_header,
            banner_brush,
            card_brush,
            license_accepted: false,
            chosen_path: Some(default_path.to_path_buf()),
            dpi: 96,
        }));
        STATE.with(|s| *s.borrow_mut() = Some(state.clone()));

        let style = WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION | WS_MINIMIZEBOX;
        let (ww, wh) = helpers::window_size_for_client(WIN_W, WIN_H, style, WINDOW_EX_STYLE(0));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ww,
            wh,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        )?;

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
        // resize the frame, rebuild fonts, then lay out controls at that DPI.
        let dpi = helpers::dpi_for(hwnd);
        rebuild_fonts(dpi);
        let (sw, sh) = helpers::window_size_for_client(
            helpers::scale(WIN_W, dpi),
            helpers::scale(WIN_H, dpi),
            style,
            WINDOW_EX_STYLE(0),
        );
        let _ = SetWindowPos(hwnd, None, 0, 0, sw, sh, SWP_NOMOVE | SWP_NOZORDER);

        helpers::center(hwnd);
        views::build_controls(hwnd, payload, default_path);
        views::relayout(hwnd, dpi);
        Ok(hwnd)
    }
}

/// Recreate the three UI fonts at the given DPI and store them (deleting the
/// old handles). Heights are the 96-dpi base sizes scaled by `dpi`.
unsafe fn rebuild_fonts(dpi: i32) {
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            let mut st = st.borrow_mut();
            unsafe {
                let _ = DeleteObject(st.font_normal.into());
                let _ = DeleteObject(st.font_bold.into());
                let _ = DeleteObject(st.font_header.into());
            }
            st.font_normal =
                helpers::create_font("Segoe UI", helpers::scale(16, dpi), FW_NORMAL.0 as i32);
            st.font_bold =
                helpers::create_font("Segoe UI", helpers::scale(16, dpi), FW_SEMIBOLD.0 as i32);
            st.font_header = helpers::create_font(
                "Segoe UI Semibold",
                helpers::scale(22, dpi),
                FW_SEMIBOLD.0 as i32,
            );
            st.dpi = dpi;
        }
    });
}

/// Dev-only: show the wizard jumped straight to one view with sample data, no
/// install worker. `view` is one of `license|choose|progress|done|error`.
#[cfg(debug_assertions)]
pub fn preview(view: &str, translator: common::i18n::Translator) -> Result<()> {
    let payload = crate::ui::sample_payload(view);
    // Accept a `-patch` suffix (e.g. `choose-patch`) to preview the patch variant.
    let phase = match view.split('-').next().unwrap_or(view) {
        "choose" => Phase::Choose,
        "progress" => Phase::Progress,
        "done" => Phase::Done,
        "error" => Phase::Error,
        _ => Phase::License,
    };
    PAYLOAD.with(|p| *p.borrow_mut() = Some(payload.clone()));
    LAUNCH_FLAG.with(|l| *l.borrow_mut() = true);
    T.with(|t| *t.borrow_mut() = translator);

    let default_path = PathBuf::from(r"C:\Program Files\Sample App");
    unsafe {
        let hwnd = create_window(&payload, &default_path)?;
        apply_phase(hwnd, phase);

        // Populate the view with believable sample content.
        match phase {
            Phase::Progress => {
                STATE.with(|s| {
                    if let Some(st) = s.borrow().as_ref()
                        && let Ok(mut p) = st.borrow().progress.lock()
                    {
                        p.done = 7_700_000;
                        p.total = 12_345_678;
                        p.name = "bin/app.exe".to_string();
                    }
                });
                handlers::update_progress(hwnd);
            }
            Phase::Error => {
                let msg = "Sample error: the disk became full while writing bin/app.exe.";
                helpers::set_dlg_text(
                    hwnd,
                    ID_STATUS,
                    &format!("{}{}", tr().get("install.err_prefix"), msg),
                );
            }
            _ => {}
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        helpers::pump_messages();
    }
    Ok(())
}

pub(super) unsafe fn apply_phase(hwnd: HWND, phase: Phase) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            state.borrow_mut().phase = phase;
        }
    });

    let show = |id: usize, vis: bool| unsafe {
        let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
        let _ = ShowWindow(h, if vis { SW_SHOW } else { SW_HIDE });
    };

    // Header/banner always visible.
    show(ID_BANNER, true);
    show(ID_HEADER, true);
    show(ID_SUBHEADER, true);

    let (lic, choose, prog, _done) = match phase {
        Phase::License => (true, false, false, false),
        Phase::Choose => (false, true, false, false),
        Phase::Progress => (false, false, true, false),
        Phase::Done => (false, false, true, true),
        Phase::Error => (false, false, false, true),
    };

    show(ID_LICENSE_EDIT, lic);
    show(ID_ACCEPT_CHK, lic);

    show(ID_PATH_LABEL, choose);
    show(ID_PATH_EDIT, choose);
    show(ID_BROWSE_BTN, choose);
    // The non-empty-folder warning (icon + text) visibility + the Install
    // button's enabled state are (re)computed below when entering Choose.
    if !choose {
        show(ID_PATH_WARN, false);
        show(ID_PATH_WARN_ICON, false);
    }

    show(ID_PROGRESS, prog);
    show(
        ID_STATUS,
        phase == Phase::Progress || phase == Phase::Done || phase == Phase::Error,
    );
    show(ID_LAUNCH_CHK, phase == Phase::Done);

    show(ID_BACK_BTN, phase == Phase::Choose && !skip_license());
    show(ID_NEXT_BTN, phase == Phase::License);
    show(ID_INSTALL_BTN, phase == Phase::Choose);
    show(
        ID_CANCEL_BTN,
        phase == Phase::License || phase == Phase::Choose || phase == Phase::Progress,
    );
    show(ID_CLOSE_BTN, phase == Phase::Done || phase == Phase::Error);

    // Entering Choose: evaluate the destination folder so the warning + the
    // Install button's enabled state reflect the current path right away.
    if phase == Phase::Choose {
        unsafe { handlers::update_path_warning(hwnd) };
    }

    // With no Choose page, the License "Next" is really the install trigger.
    if phase == Phase::License {
        let label = if skip_path() {
            "install.install"
        } else {
            "install.next"
        };
        unsafe { helpers::set_dlg_text(hwnd, ID_NEXT_BTN, &tr().get(label)) };
    }

    if phase == Phase::Done {
        unsafe {
            helpers::set_dlg_text(hwnd, ID_STATUS, &tr().get("install.done"));
            // Default the launch checkbox to checked if launch flag set OR exe known.
            let default_checked = LAUNCH_FLAG.with(|l| *l.borrow())
                || PAYLOAD.with(|p| {
                    p.borrow()
                        .as_ref()
                        .map(|p| !p.manifest.exe.is_empty())
                        .unwrap_or(false)
                });
            let h = GetDlgItem(Some(hwnd), ID_LAUNCH_CHK as i32).unwrap_or_default();
            SendMessageW(
                h,
                BM_SETCHECK,
                Some(WPARAM(if default_checked {
                    BST_CHECKED.0 as usize
                } else {
                    BST_UNCHECKED.0 as usize
                })),
                Some(LPARAM(0)),
            );
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DPICHANGED => unsafe {
            // Moved to a monitor of different scale. lParam is the suggested new
            // window rect; HIWORD(wParam) the new DPI. Resize to it, rebuild
            // fonts + lay out controls at the new DPI, then repaint - keeps the
            // wizard crisp instead of leaving controls mis-scaled / off-window.
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
            views::apply_fonts(hwnd);
            views::relayout(hwnd, new_dpi);
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        },
        WM_CTLCOLORSTATIC => unsafe {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
            let ctrl = HWND(lparam.0 as *mut _);
            let banner = GetDlgItem(Some(hwnd), ID_BANNER as i32).unwrap_or_default();
            let header = GetDlgItem(Some(hwnd), ID_HEADER as i32).unwrap_or_default();
            let sub = GetDlgItem(Some(hwnd), ID_SUBHEADER as i32).unwrap_or_default();
            let warn = GetDlgItem(Some(hwnd), ID_PATH_WARN as i32).unwrap_or_default();
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
            if ctrl == warn {
                // Red (0x00BBGGRR)
                SetTextColor(hdc, COLORREF(0x000000C0));
                return LRESULT(STATE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .map(|st| st.borrow().card_brush.0 as isize)
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
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            // Re-check the destination as the user edits the path field.
            if id == ID_PATH_EDIT && code == EN_CHANGE {
                handlers::update_path_warning(hwnd);
            }
            match id {
                ID_BROWSE_BTN => handlers::on_browse(hwnd),
                ID_INSTALL_BTN => handlers::on_install(hwnd),
                ID_CANCEL_BTN => handlers::on_cancel(hwnd),
                ID_NEXT_BTN => handlers::on_next(hwnd),
                ID_BACK_BTN => handlers::on_back(hwnd),
                ID_ACCEPT_CHK => handlers::on_accept_toggle(hwnd),
                ID_CLOSE_BTN => handlers::on_finish(hwnd),
                _ => {}
            }
            LRESULT(0)
        },
        m if m == helpers::WM_APP_PROGRESS => unsafe {
            handlers::update_progress(hwnd);
            LRESULT(0)
        },
        m if m == helpers::WM_APP_DONE => unsafe {
            apply_phase(hwnd, Phase::Done);
            LRESULT(0)
        },
        m if m == helpers::WM_APP_ERROR => unsafe {
            STATE.with(|s| {
                if let Some(state) = s.borrow().as_ref() {
                    let text = state.borrow().error_text.clone();
                    helpers::set_dlg_text(
                        hwnd,
                        ID_STATUS,
                        &format!("{}{}", tr().get("install.err_prefix"), text),
                    );
                }
            });
            apply_phase(hwnd, Phase::Error);
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
                    let _ = DeleteObject(st.font_normal.into());
                    let _ = DeleteObject(st.font_bold.into());
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

/// Modal message box with the localized caption.
pub(super) unsafe fn message_box(hwnd: HWND, text: &str, style: MESSAGEBOX_STYLE) {
    let t = wide(text);
    let c = wide(&tr().get("install.msg_caption"));
    unsafe {
        MessageBoxW(Some(hwnd), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), style);
    }
}
