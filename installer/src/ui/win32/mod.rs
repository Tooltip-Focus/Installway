// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Full installer wizard. Phases: License → Choose → Progress → Done/Error.
//!
//! `mod.rs` owns the window, shared state, message loop and phase switching;
//! [`views`] builds the controls for each phase; [`handlers`] runs the button
//! and worker logic.

mod handlers;
mod plugin_pages;
mod views;

use crate::payload::LoadedPayload;
use crate::ui::helpers;
use anyhow::Result;
use common::model::choice_option::ChoiceOption;
use common::model::choice_style::ChoiceStyle;
use common::model::install_dir_restriction::InstallDirRestriction;
use common::model::installer_payload::InstallerPayload;
use common::model::page_step::PageStep;
use common::model::plugin_page::PluginPage;
use common::model::plugin_widget::PluginWidget;
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
pub(super) const ID_ERROR_BOX: usize = 1019;
pub(super) const ID_ERROR_ICON: usize = 1020;

/// First dialog id for dynamically-built plugin-page controls. Kept well clear
/// of the built-in ids (1001-1018); allocated sequentially in `plugin_pages`.
pub(super) const ID_PLUGIN_BASE: usize = 5000;

pub(super) const WIN_W: i32 = 700;
pub(super) const WIN_H: i32 = 500;
pub(super) const BANNER_H: i32 = 72;
pub(super) const PAD: i32 = 24;

const ACCENT_LIGHT: u32 = 0x00F3F3F3; // light gray banner card

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Phase {
    License,
    Choose,
    /// The plugin wizard's current page (driven by [`WIZARD`]).
    Plugin,
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
    pub error_brush: HBRUSH,
    pub license_accepted: bool,
    pub chosen_path: Option<PathBuf>,
    /// Runtime-built controls for plugin-contributed pages (empty with no UI
    /// plugins). See [`plugin_pages`].
    pub(in crate::ui::win32) plugin_fields: Vec<plugin_pages::PluginField>,
    /// The product banner header/subheader text, captured at build so it can be
    /// restored after a plugin page overwrote the banner with its own title.
    pub header_text: String,
    pub sub_text: String,
    /// Current monitor DPI (96 = 100%); drives scaled layout + fonts, updated on
    /// `WM_DPICHANGED`.
    pub dpi: i32,
}

thread_local! {
    pub(super) static STATE: RefCell<Option<Rc<RefCell<UiState>>>> = const { RefCell::new(None) };
    pub(super) static PAYLOAD: RefCell<Option<InstallerPayload>> = const { RefCell::new(None) };
    pub(super) static UNINSTALLER: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    pub(super) static LAUNCH_FLAG: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_LICENSE: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_PATH: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static RESTRICTION: RefCell<InstallDirRestriction> =
        const { RefCell::new(InstallDirRestriction::Enforce) };
    pub(super) static DEFAULT_PATH: RefCell<String> = const { RefCell::new(String::new()) };
    /// The plugin-page wizard engine (None when no `ui = true` plugin).
    pub(super) static WIZARD: RefCell<Option<plugin_pages::Wizard>> = const { RefCell::new(None) };
    static T: RefCell<common::i18n::Translator> = RefCell::new(common::i18n::Translator::default());
}

fn skip_license() -> bool {
    SKIP_LICENSE.with(|s| *s.borrow())
}
fn skip_path() -> bool {
    SKIP_PATH.with(|s| *s.borrow())
}
fn restriction() -> InstallDirRestriction {
    RESTRICTION.with(|r| *r.borrow())
}
fn default_path() -> String {
    DEFAULT_PATH.with(|d| d.borrow().clone())
}

/// Whether a `ui = true` plugin wizard is active.
pub(super) fn has_plugin_pages() -> bool {
    WIZARD.with(|w| w.borrow().is_some())
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
    ui_plugins: Option<crate::extract::UiPlugins>,
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
    RESTRICTION.with(|r| *r.borrow_mut() = loaded.payload.install_dir_restriction);
    DEFAULT_PATH.with(|d| *d.borrow_mut() = default_path.to_string_lossy().into_owned());
    // Seed the wizard. The extracted-DLL temp dir is held by an `Arc` shared
    // between this guard and every background step query, so the dir survives an
    // in-flight query even if the window closes before it returns.
    let _ui_guard = ui_plugins.map(|u| {
        let tmp = Arc::new(u.tmp);
        WIZARD.with(|w| {
            *w.borrow_mut() = Some(plugin_pages::Wizard::new(
                u.plugins,
                u.base_ctx,
                u.self_exe,
                tmp.clone(),
            ));
        });
        tmp
    });
    T.with(|t| *t.borrow_mut() = translator);

    let has_plugin = has_plugin_pages();

    unsafe {
        let hwnd = create_window(&loaded.payload, &default_path)?;
        // Pick the first interactive page. With both built-in pages skipped, the
        // plugin wizard (if any) is the only user step; otherwise install straight
        // to the default path.
        if skip_license && skip_path {
            if has_plugin {
                let _ = ShowWindow(hwnd, SW_SHOW);
                handlers::begin_plugin_wizard(hwnd);
            } else {
                apply_phase(hwnd, Phase::Progress);
                let _ = ShowWindow(hwnd, SW_SHOW);
                handlers::on_install(hwnd);
            }
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
    payload: &common::model::installer_payload::InstallerPayload,
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
        let error_brush = CreateSolidBrush(COLORREF(0x00F2F2FF));

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
            error_brush,
            license_accepted: false,
            chosen_path: Some(default_path.to_path_buf()),
            plugin_fields: Vec::new(),
            header_text: String::new(),
            sub_text: String::new(),
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
        "plugin" => Phase::Plugin,
        "progress" => Phase::Progress,
        "done" => Phase::Done,
        "error" => Phase::Error,
        _ => Phase::License,
    };
    PAYLOAD.with(|p| *p.borrow_mut() = Some(payload.clone()));
    LAUNCH_FLAG.with(|l| *l.borrow_mut() = true);
    T.with(|t| *t.borrow_mut() = translator);

    // `--preview plugin`: a canned one-page wizard (no real plugin/payload needed)
    // so the dynamic renderer can be exercised.
    if matches!(phase, Phase::Plugin) {
        let page = PluginPage {
            id: "region".into(),
            title: "Choose your country".into(),
            subtitle: "Sample plugin page (preview)".into(),
            widgets: vec![
                PluginWidget::Label {
                    id: String::new(),
                    text: "Where will you use this app?".into(),
                },
                PluginWidget::SingleChoice {
                    id: "country".into(),
                    label: "Country".into(),
                    options: vec![
                        ChoiceOption {
                            label: "France".into(),
                            value: "FR".into(),
                        },
                        ChoiceOption {
                            label: "DOM-TOM".into(),
                            value: "DOM".into(),
                        },
                        ChoiceOption {
                            label: "Other".into(),
                            value: "XX".into(),
                        },
                    ],
                    style: ChoiceStyle::Radio,
                    default: "FR".into(),
                    required: true,
                },
                PluginWidget::Text {
                    id: "license".into(),
                    label: "License key".into(),
                    default: String::new(),
                    required: false,
                    placeholder: "optional".into(),
                    password: true,
                    number: false,
                    multiline: false,
                },
                PluginWidget::MultiChoice {
                    id: "addons".into(),
                    label: "Optional add-ons".into(),
                    options: vec![
                        ChoiceOption {
                            label: "Documentation".into(),
                            value: "docs".into(),
                        },
                        ChoiceOption {
                            label: "Samples".into(),
                            value: "samples".into(),
                        },
                    ],
                    default: vec!["docs".into()],
                    required: false,
                },
            ],
            buttons: true,
        };
        let step = PageStep::Page {
            page,
            notice: String::new(),
            back: true,
        };
        WIZARD.with(|w| *w.borrow_mut() = Some(plugin_pages::Wizard::canned(vec![step])));
    }

    let default_path = PathBuf::from(r"C:\Program Files\Sample App");
    unsafe {
        let hwnd = create_window(&payload, &default_path)?;
        if matches!(phase, Phase::Plugin) {
            handlers::begin_plugin_wizard(hwnd);
        } else {
            apply_phase(hwnd, phase);
        }

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
                helpers::set_dlg_text(
                    hwnd,
                    ID_ERROR_BOX,
                    "Sample error: the disk became full while writing bin/app.exe.",
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
        Phase::Plugin => (false, false, false, false),
        Phase::Progress => (false, false, true, false),
        Phase::Done => (false, false, true, true),
        Phase::Error => (false, false, false, true),
    };

    if phase != Phase::Plugin {
        let (h, s) = STATE.with(|st| {
            st.borrow()
                .as_ref()
                .map(|x| {
                    let x = x.borrow();
                    (x.header_text.clone(), x.sub_text.clone())
                })
                .unwrap_or_default()
        });
        unsafe {
            helpers::set_dlg_text(hwnd, ID_HEADER, &h);
            helpers::set_dlg_text(hwnd, ID_SUBHEADER, &s);
        }
    }

    if phase == Phase::Error {
        unsafe {
            helpers::set_dlg_text(hwnd, ID_HEADER, &tr().get("install.err_title"));
            helpers::set_dlg_text(hwnd, ID_SUBHEADER, &tr().get("install.err_sub"));
        }
    }

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
    show(ID_STATUS, matches!(phase, Phase::Progress | Phase::Done));
    show(ID_ERROR_BOX, phase == Phase::Error);
    show(ID_ERROR_ICON, phase == Phase::Error);

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
    // Install button's enabled state reflect the current path right away. With
    // plugin pages pending, the primary button advances to them, so label it
    // "Next" rather than "Install".
    if phase == Phase::Choose {
        unsafe { handlers::update_path_warning(hwnd) };
        let label = if has_plugin_pages() {
            "install.next"
        } else {
            "install.install"
        };
        unsafe { helpers::set_dlg_text(hwnd, ID_INSTALL_BTN, &tr().get(label)) };
    }

    // With no Choose page (and no plugin pages) the License "Next" is really the
    // install trigger; otherwise it advances to the next page.
    if phase == Phase::License {
        let label = if skip_path() && !has_plugin_pages() {
            "install.install"
        } else {
            "install.next"
        };
        unsafe { helpers::set_dlg_text(hwnd, ID_NEXT_BTN, &tr().get(label)) };
    }

    // Plugin wizard: show the current page's controls + nav buttons; hide all
    // plugin controls on the built-in phases. The primary button is always Next
    // (we only learn it's the last page when the next step returns Done).
    let active_plugin = if phase == Phase::Plugin {
        plugin_pages::current_slot()
    } else {
        None
    };
    unsafe { plugin_pages::apply_visibility(hwnd, active_plugin) };
    if phase == Phase::Plugin {
        let has_builtin = !skip_path() || !skip_license();
        let can_back = WIZARD.with(|w| {
            w.borrow()
                .as_ref()
                .map(|z| z.wants_back() && (z.can_pop() || has_builtin))
                .unwrap_or(false)
        });
        show(ID_BACK_BTN, can_back);
        show(ID_NEXT_BTN, true);
        show(ID_INSTALL_BTN, false);
        show(ID_CANCEL_BTN, true);
        show(ID_CLOSE_BTN, false);
        unsafe {
            // Choose may have relabeled Install to "Next"; restore Next's label.
            helpers::set_dlg_text(hwnd, ID_NEXT_BTN, &tr().get("install.next"));
            plugin_pages::set_banner(hwnd);
        }
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
        WM_CTLCOLOREDIT => unsafe {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
            let ctrl = HWND(lparam.0 as *mut _);
            let error_box = GetDlgItem(Some(hwnd), ID_ERROR_BOX as i32).unwrap_or_default();
            if ctrl == error_box {
                let _ = SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x000000AA));
                return LRESULT(STATE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .map(|st| st.borrow().error_brush.0 as isize)
                        .unwrap_or(0)
                }));
            }
            LRESULT(DefWindowProcW(hwnd, msg, wparam, lparam).0)
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
            let text = if lparam.0 != 0 {
                *Box::from_raw(lparam.0 as *mut String)
            } else {
                String::new()
            };
            STATE.with(|s| {
                if let Some(state) = s.borrow().as_ref() {
                    state.borrow_mut().error_text = text.clone();
                }
            });
            helpers::set_dlg_text(hwnd, ID_ERROR_BOX, &text);
            apply_phase(hwnd, Phase::Error);
            LRESULT(0)
        },
        m if m == helpers::WM_APP_PERM_ERROR => unsafe {
            // Unbox the payload (path + plugin_inputs).
            let payload = if lparam.0 != 0 {
                *Box::from_raw(lparam.0 as *mut handlers::PermErrorPayload)
            } else {
                return LRESULT(0);
            };
            // Retrieve progress state so the orchestration thread can update it.
            let progress_shared =
                STATE.with(|s| s.borrow().as_ref().map(|st| st.borrow().progress.clone()));
            let Some(progress_shared) = progress_shared else {
                return LRESULT(0);
            };
            // Directly trigger UAC — the UAC dialog IS the user's yes/no.
            handlers::start_elevated_install(hwnd.0 as isize, payload, progress_shared);
            LRESULT(0)
        },
        m if m == helpers::WM_APP_PERM_DENIED => unsafe {
            // UAC was cancelled or the worker failed to start.
            let path = if lparam.0 != 0 {
                *Box::from_raw(lparam.0 as *mut PathBuf)
            } else {
                PathBuf::new()
            };
            if !skip_path() {
                apply_phase(hwnd, Phase::Choose);
                message_box(hwnd, &tr().get("install.perm_denied_path"), MB_ICONWARNING);
            } else {
                let msg = format!(
                    "No permission to write to:\n{}\n\nThis location requires administrator rights.",
                    path.display()
                );
                STATE.with(|s| {
                    if let Some(state) = s.borrow().as_ref() {
                        state.borrow_mut().error_text = msg.clone();
                    }
                });
                helpers::set_dlg_text(hwnd, ID_ERROR_BOX, &msg);
                apply_phase(hwnd, Phase::Error);
            }
            LRESULT(0)
        },
        m if m == helpers::WM_APP_PLUGIN_STEP => unsafe {
            handlers::on_plugin_step(hwnd);
            LRESULT(0)
        },
        m if m == helpers::WM_APP_PLUGIN_PROGRESS => {
            let scaled = (wparam.0.min(100) as i32) * 100;
            plugin_pages::update_current_progress(hwnd, scaled);
            LRESULT(0)
        }
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
                    let _ = DeleteObject(st.error_brush.into());
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
