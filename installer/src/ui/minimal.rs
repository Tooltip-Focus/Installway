// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Compact, auto-starting update UI for app-triggered self-updates.
//!
//! No license page, no path picker, no Install button - it starts immediately
//! and just shows progress. Layout:
//!
//! ```text
//!  ┌────────────────────────────────────────────┐
//!  │  ██      Applying update                    │
//!  │  ██      MyApp 1.2                          │
//!  │          [██████████░░░░░░░]  62%           │
//!  │          Updating bin/app.exe               │
//!  └────────────────────────────────────────────┘
//! ```
//! App icon on the left, text + progress on the right. Closes itself on
//! success; on failure it stays with the error message.

use crate::extract::{InstallCtx, install};
use crate::payload::LoadedPayload;
use crate::ui::helpers::{
    self, WM_APP_DONE, WM_APP_ERROR, WM_APP_PROGRESS, create_font, own_icon, post, scale_progress,
    set_dlg_text, set_progress,
};
use anyhow::Result;
use common::utils::wide;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::thread;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, GetStockObject, HBRUSH, HFONT,
    InvalidateRect, SetBkMode, SetTextColor, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::PROGRESS_CLASSW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const ID_ICON: usize = 1;
const ID_TITLE: usize = 2;
const ID_SUB: usize = 3;
const ID_PROGRESS: usize = 4;
const ID_STATUS: usize = 5;

const STM_SETICON: u32 = 0x0170;
const SS_ICON: u32 = 0x0003;

const WIN_W: i32 = 480; // client width
const WIN_H: i32 = 140; // client height
const PAD: i32 = 20;
const ICON_SZ: i32 = 48;
const COL_X: i32 = PAD + ICON_SZ + 20; // text column start

struct Prog {
    done: u64,
    total: u64,
    name: String,
}

struct State {
    cancel: Arc<AtomicBool>,
    prog: Arc<Mutex<Prog>>,
    font_title: HFONT,
    font_body: HFONT,
    bg: HBRUSH,
    hicon: HICON,
    /// Current monitor DPI (96 = 100%); updated on `WM_DPICHANGED`.
    dpi: i32,
}

thread_local! {
    static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
    static T: RefCell<common::i18n::Translator> = RefCell::new(common::i18n::Translator::default());
}

fn tr() -> common::i18n::Translator {
    T.with(|t| *t.borrow())
}

pub fn run(
    loaded: LoadedPayload,
    install_dir: PathBuf,
    launch_flag: bool,
    translator: common::i18n::Translator,
) -> Result<()> {
    T.with(|t| *t.borrow_mut() = translator);

    // Build window + register state (the only part that needs FFI).
    let win = unsafe { build_window(&loaded.payload)? };

    // Worker runs in safe code; only the message posts touch FFI.
    spawn_worker(
        win.hwnd_isize,
        install_dir,
        launch_flag,
        win.cancel,
        win.prog,
    );

    unsafe { helpers::pump_messages() };
    Ok(())
}

/// Dev-only: show the minimal window with sample mid-progress, no install worker.
#[cfg(debug_assertions)]
pub fn preview(translator: common::i18n::Translator) -> Result<()> {
    T.with(|t| *t.borrow_mut() = translator);
    let payload = crate::ui::sample_payload("minimal");
    unsafe {
        let win = build_window(&payload)?;
        let hwnd = HWND(win.hwnd_isize as *mut _);
        set_progress(hwnd, ID_PROGRESS, scale_progress(62, 100));
        set_dlg_text(hwnd, ID_STATUS, "62%  bin/app.exe");
        helpers::pump_messages();
    }
    Ok(())
}

struct Window {
    hwnd_isize: isize,
    cancel: Arc<AtomicBool>,
    prog: Arc<Mutex<Prog>>,
}

unsafe fn build_window(
    payload: &common::model::installer_payload::InstallerPayload,
) -> Result<Window> {
    helpers::init_progress_class();
    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }?;

    let class_name = w!("InstallwayMiniWnd");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: HINSTANCE(hinstance.0),
        hIcon: HICON::default(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        hbrBackground: HBRUSH(unsafe { GetStockObject(WHITE_BRUSH) }.0),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: class_name,
        hIconSm: HICON::default(),
    };
    unsafe { RegisterClassExW(&wc) };

    let hicon = unsafe { own_icon() };

    let title_w = wide(&tr().get("install.minimal_title"));
    let state = Rc::new(RefCell::new(State {
        cancel: Arc::new(AtomicBool::new(false)),
        prog: Arc::new(Mutex::new(Prog {
            done: 0,
            total: 0,
            name: String::new(),
        })),
        font_title: create_font("Segoe UI Semibold", 20, FW_SEMIBOLD.0 as i32),
        font_body: create_font("Segoe UI", 15, FW_NORMAL.0 as i32),
        bg: unsafe { CreateSolidBrush(COLORREF(0x00FFFFFF)) },
        hicon,
        dpi: 96,
    }));
    let cancel = state.borrow().cancel.clone();
    let prog = state.borrow().prog.clone();
    STATE.with(|s| *s.borrow_mut() = Some(state));

    // No min/max box, fixed small tool-like window (still has close).
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let (ww, wh) = helpers::window_size_for_client(WIN_W, WIN_H, style, WINDOW_EX_STYLE(0));
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_w.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ww,
            wh,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        )
    }?;
    if !hicon.is_invalid() {
        unsafe {
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
    }

    unsafe {
        // Scale to the monitor this window opened on (per-monitor DPI aware).
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
        build_controls(hwnd, payload);
        relayout(hwnd, dpi);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }

    Ok(Window {
        hwnd_isize: hwnd.0 as isize,
        cancel,
        prog,
    })
}

/// Auto-start the install worker (no button). Posts progress/done/error back
/// to the window thread.
fn spawn_worker(
    hwnd_isize: isize,
    install_dir: PathBuf,
    launch_flag: bool,
    cancel: Arc<AtomicBool>,
    prog: Arc<Mutex<Prog>>,
) {
    thread::spawn(move || {
        let mut loaded = match crate::payload::load_and_verify() {
            Ok(l) => l,
            Err(e) => return post_err(hwnd_isize, &format!("{e}")),
        };
        // Compact UI is non-interactive: plugin pages use their declared defaults.
        let plugin_inputs = match crate::ui::headless_plugin_inputs(&loaded, &install_dir) {
            Ok(m) => m,
            Err(e) => return post_err(hwnd_isize, &format!("{e}")),
        };

        // Pre-flight write test — same check as the interactive wizard.
        let needs_elevation = !common::elevation::is_already_elevated()
            && matches!(
                crate::extract::check_writable(&install_dir),
                Err(ref e) if e.is::<crate::extract::PermissionDeniedError>()
            );

        if needs_elevation {
            run_elevated_worker(
                hwnd_isize,
                &install_dir,
                &plugin_inputs,
                launch_flag,
                &prog,
                &loaded,
            );
            return;
        }

        let prog_cb: common::ProgressFn = {
            let prog = prog.clone();
            Arc::new(move |done, total, name| {
                if let Ok(mut p) = prog.lock() {
                    p.done = done;
                    p.total = total;
                    p.name = name.to_string();
                }
                post(hwnd_isize, WM_APP_PROGRESS);
            })
        };
        // Reached when writable without elevation, or already running elevated.
        // Machine-wide iff the target is a shared location (catches an
        // already-admin run into Program Files); the elevated worker handles the
        // needs-elevation path with requires_admin = true.
        let requires_admin = common::paths::is_machine_location(&install_dir);
        // Resolve feature packs from the headless answers and filter the manifest.
        crate::extract::resolve_and_filter(
            &mut loaded,
            &install_dir,
            requires_admin,
            &plugin_inputs,
        );
        let ctx = InstallCtx {
            install_dir: install_dir.clone(),
            payload: &loaded.payload,
            zip_bytes: loaded.zip(),
            cancel,
            on_progress: prog_cb,
            plugin_inputs: plugin_inputs.clone(),
            requires_admin,
            hwnd_parent: hwnd_isize,
            translator: tr(),
        };
        if let Err(e) = install(ctx) {
            return post_err(hwnd_isize, &format!("{e}"));
        }
        if let Err(e) = crate::install::finalize(
            &install_dir,
            &loaded.payload,
            &loaded.uninstaller_bytes,
            loaded.zip(),
            &plugin_inputs,
            requires_admin,
        ) {
            return post_err(hwnd_isize, &format!("finalize: {e}"));
        }
        if launch_flag && !loaded.payload.manifest.exe.is_empty() {
            let _ = crate::install::launch_product(&install_dir, &loaded.payload.manifest.exe);
        }
        post(hwnd_isize, WM_APP_DONE);
    });
}

fn run_elevated_worker(
    hwnd_isize: isize,
    install_dir: &std::path::Path,
    plugin_inputs: &common::plugin::InputsByPlugin,
    launch_flag: bool,
    prog: &Arc<Mutex<Prog>>,
    loaded: &crate::payload::LoadedPayload,
) {
    let result =
        crate::elevation::run_elevated_install(install_dir, plugin_inputs, |done, total, name| {
            if let Ok(mut p) = prog.lock() {
                p.done = done;
                p.total = total;
                p.name = name.to_string();
            }
            post(hwnd_isize, WM_APP_PROGRESS);
        });
    match result {
        Ok(()) => {
            if launch_flag && !loaded.payload.manifest.exe.is_empty() {
                let _ = crate::install::launch_product(install_dir, &loaded.payload.manifest.exe);
            }
            post(hwnd_isize, WM_APP_DONE);
        }
        Err(e) if e.is::<crate::elevation::UacCancelledError>() => {
            post_err(hwnd_isize, &tr().get("install.uac_cancelled"));
        }
        Err(e) => post_err(hwnd_isize, &format!("{e:#}")),
    }
}

fn post_err(hwnd_isize: isize, msg: &str) {
    let ptr = Box::into_raw(Box::new(msg.to_string())) as isize;
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_isize as *mut _)),
            WM_APP_ERROR,
            WPARAM(0),
            LPARAM(ptr),
        )
    };
}

/// Recreate the two fonts at the given DPI and store them (deleting the old).
unsafe fn rebuild_fonts(dpi: i32) {
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            let mut st = st.borrow_mut();
            unsafe {
                let _ = DeleteObject(st.font_title.into());
                let _ = DeleteObject(st.font_body.into());
            }
            st.font_title = create_font(
                "Segoe UI Semibold",
                helpers::scale(20, dpi),
                FW_SEMIBOLD.0 as i32,
            );
            st.font_body = create_font("Segoe UI", helpers::scale(15, dpi), FW_NORMAL.0 as i32);
            st.dpi = dpi;
        }
    });
}

/// Reposition + resize every control for `dpi` (96-dpi base, scaled). Run after
/// creation and on each `WM_DPICHANGED`. Identity at 96 dpi.
unsafe fn relayout(hwnd: HWND, dpi: i32) {
    let s = |v: i32| helpers::scale(v, dpi);
    let col_w = WIN_W - COL_X - PAD;
    let items: &[(usize, i32, i32, i32, i32)] = &[
        (ID_ICON, PAD, PAD, ICON_SZ, ICON_SZ),
        (ID_TITLE, COL_X, PAD, col_w, 26),
        (ID_SUB, COL_X, PAD + 28, col_w, 20),
        (ID_PROGRESS, COL_X, PAD + 56, col_w, 18),
        (ID_STATUS, COL_X, PAD + 80, col_w, 20),
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

/// (Re)apply the stored fonts to the text controls.
unsafe fn apply_fonts(hwnd: HWND) {
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            let st = st.borrow();
            unsafe {
                helpers::set_font(hwnd, ID_TITLE, st.font_title);
                helpers::set_font(hwnd, ID_SUB, st.font_body);
                helpers::set_font(hwnd, ID_STATUS, st.font_body);
            }
        }
    });
}

unsafe fn build_controls(hwnd: HWND, payload: &common::model::installer_payload::InstallerPayload) {
    let hinst = unsafe { GetModuleHandleW(PCWSTR::null()).unwrap_or_default() };
    let hinst = HINSTANCE(hinst.0);
    let tr = tr();

    let title_w = wide(&tr.get("install.minimal_title"));
    let sub_w = wide(&tr.fmt(
        "install.minimal_sub",
        &[
            ("product", &payload.product),
            ("version", &payload.to_version),
        ],
    ));

    unsafe {
        // Icon (static, owner sets via STM_SETICON)
        let icon_ctrl = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR::null(),
            WS_VISIBLE | WS_CHILD | WINDOW_STYLE(SS_ICON),
            PAD,
            PAD,
            ICON_SZ,
            ICON_SZ,
            Some(hwnd),
            Some(HMENU(ID_ICON as *mut _)),
            Some(hinst),
            None,
        )
        .ok();
        if let Some(ic) = icon_ctrl {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    let h = st.borrow().hicon;
                    if !h.is_invalid() {
                        SendMessageW(ic, STM_SETICON, Some(WPARAM(h.0 as usize)), Some(LPARAM(0)));
                    }
                }
            });
        }

        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(title_w.as_ptr()),
            WS_VISIBLE | WS_CHILD,
            COL_X,
            PAD,
            WIN_W - COL_X - PAD,
            26,
            Some(hwnd),
            Some(HMENU(ID_TITLE as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(sub_w.as_ptr()),
            WS_VISIBLE | WS_CHILD,
            COL_X,
            PAD + 28,
            WIN_W - COL_X - PAD,
            20,
            Some(hwnd),
            Some(HMENU(ID_SUB as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PROGRESS_CLASSW,
            PCWSTR::null(),
            WS_VISIBLE | WS_CHILD,
            COL_X,
            PAD + 56,
            WIN_W - COL_X - PAD,
            18,
            Some(hwnd),
            Some(HMENU(ID_PROGRESS as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_VISIBLE | WS_CHILD,
            COL_X,
            PAD + 80,
            WIN_W - COL_X - PAD,
            20,
            Some(hwnd),
            Some(HMENU(ID_STATUS as *mut _)),
            Some(hinst),
            None,
        );
    }

    unsafe { apply_fonts(hwnd) }
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
            let sub = GetDlgItem(Some(hwnd), ID_SUB as i32).unwrap_or_default();
            let status = GetDlgItem(Some(hwnd), ID_STATUS as i32).unwrap_or_default();
            let _ = SetBkMode(hdc, TRANSPARENT);
            // Subtitle + status in muted gray, title in near-black.
            if ctrl == sub || ctrl == status {
                SetTextColor(hdc, COLORREF(0x00777777));
            } else {
                SetTextColor(hdc, COLORREF(0x00202020));
            }
            LRESULT(STATE.with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|st| st.borrow().bg.0 as isize)
                    .unwrap_or(0)
            }))
        },
        m if m == WM_APP_PROGRESS => unsafe {
            update_progress(hwnd);
            LRESULT(0)
        },
        m if m == WM_APP_DONE => unsafe {
            set_dlg_text(hwnd, ID_STATUS, &tr().get("install.minimal_done"));
            set_progress(hwnd, ID_PROGRESS, 10000);
            // Brief pause so the user sees 100%, then close.
            let _ = SetTimer(Some(hwnd), 1, 900, None);
            LRESULT(0)
        },
        WM_TIMER => unsafe {
            let _ = KillTimer(Some(hwnd), 1);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        },
        m if m == WM_APP_ERROR => unsafe {
            let text = if lparam.0 != 0 {
                *Box::from_raw(lparam.0 as *mut String)
            } else {
                String::new()
            };
            set_dlg_text(
                hwnd,
                ID_STATUS,
                &format!("{}{}", tr().get("install.err_prefix"), text),
            );
            LRESULT(0)
        },
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    let st = st.borrow();
                    let _ = DeleteObject(st.font_title.into());
                    let _ = DeleteObject(st.font_body.into());
                    let _ = DeleteObject(st.bg.into());
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn update_progress(hwnd: HWND) {
    STATE.with(|s| {
        let Some(st) = s.borrow().as_ref().cloned() else {
            return;
        };
        let st = st.borrow();
        let (done, total, name) = match st.prog.lock() {
            Ok(p) => (p.done, p.total, p.name.clone()),
            Err(_) => return,
        };
        let scaled = scale_progress(done, total);
        unsafe { set_progress(hwnd, ID_PROGRESS, scaled) };
        let pct = scaled / 100;
        unsafe { set_dlg_text(hwnd, ID_STATUS, &format!("{}%  {}", pct, name)) };
    });
}
