// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Control construction for each wizard view. All controls are created up front
//! and shown/hidden per phase by [`super::apply_phase`].

use super::{
    BANNER_H, ID_ACCEPT_CHK, ID_BACK_BTN, ID_BANNER, ID_BROWSE_BTN, ID_CANCEL_BTN, ID_CLOSE_BTN,
    ID_ERROR_BOX, ID_ERROR_ICON, ID_HEADER, ID_INSTALL_BTN, ID_LAUNCH_CHK, ID_LICENSE_EDIT,
    ID_NEXT_BTN, ID_PATH_EDIT, ID_PATH_LABEL, ID_PATH_WARN, ID_PATH_WARN_ICON, ID_PROGRESS,
    ID_STATUS, ID_SUBHEADER, PAD, STATE, WIN_H, WIN_W, tr,
};
use crate::ui::helpers::{self};
use common::model::installer_payload::InstallerPayload;
use common::model::payload_kind::PayloadKind;
use common::utils::wide;
use std::path::Path;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::PROGRESS_CLASSW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const BS_PUSHBUTTON: u32 = 0x0;
const BS_DEFPUSHBUTTON: u32 = 0x1;
const BS_AUTOCHECKBOX: u32 = 0x3;
const SS_ICON: u32 = 0x0003;
const SS_CENTERIMAGE: u32 = 0x0200;
const SS_REALSIZECONTROL: u32 = 0x0040;
const STM_SETICON: u32 = 0x0170;
const ES_READONLY: u32 = 0x0800;
const ES_MULTILINE: u32 = 0x0004;
const ES_LEFT: u32 = 0x0000;
const WS_VSCROLL: WINDOW_STYLE = WINDOW_STYLE(0x0020_0000);

const LOREM: &str = "END USER LICENSE AGREEMENT - SAMPLE\r\n\r\n\
Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
consequat.\r\n\r\n\
Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore \
eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, \
sunt in culpa qui officia deserunt mollit anim id est laborum.\r\n\r\n\
Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium \
doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore \
veritatis et quasi architecto beatae vitae dicta sunt explicabo.\r\n\r\n\
Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit, \
sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt.\r\n\r\n\
At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis \
praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias \
excepturi sint occaecati cupiditate non provident, similique sunt in culpa \
qui officia deserunt mollitia animi, id est laborum et dolorum fuga.\r\n\r\n\
By clicking 'I accept' you agree to be bound by the terms above.";

pub(super) unsafe fn build_controls(hwnd: HWND, payload: &InstallerPayload, default_path: &Path) {
    let hinst = HINSTANCE(unsafe { GetModuleHandleW(PCWSTR::null()).unwrap_or_default() }.0);
    unsafe {
        build_banner_header(hwnd, hinst, payload);
        build_license(hwnd, hinst, payload);
        build_choose(hwnd, hinst, default_path);
        build_progress(hwnd, hinst);
        build_error_box(hwnd, hinst);
        build_done(hwnd, hinst);
        build_buttons(hwnd, hinst);
        apply_fonts(hwnd);
    }
}

/// Banner strip + product header + subheader (always visible).
unsafe fn build_banner_header(hwnd: HWND, hinst: HINSTANCE, payload: &InstallerPayload) {
    let tr = tr();
    let header_s = tr.fmt(
        "install.header",
        &[
            ("product", &payload.product),
            ("version", &payload.to_version),
        ],
    );
    let header = wide(&header_s);
    let sub = match payload.kind {
        PayloadKind::Full => tr.get("install.sub_full"),
        PayloadKind::Patch => tr.fmt(
            "install.sub_patch",
            &[
                ("from", payload.from_version.as_deref().unwrap_or("")),
                ("to", &payload.to_version),
            ],
        ),
    };
    let sub_w = wide(&sub);
    // Remember the product banner so plugin pages can borrow the banner and the
    // built-in phases can restore it (see `super::apply_phase`).
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            let mut st = st.borrow_mut();
            st.header_text = header_s;
            st.sub_text = sub;
        }
    });

    unsafe {
        // Banner background - a wide empty STATIC; WM_CTLCOLORSTATIC paints it.
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
            PCWSTR(header.as_ptr()),
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
    }
}

/// License view: read-only EULA edit + "I accept" checkbox.
///
/// Layout (top→bottom): banner, license edit, accept checkbox, button row.
unsafe fn build_license(hwnd: HWND, hinst: HINSTANCE, payload: &InstallerPayload) {
    let tr = tr();
    let accept_w = wide(&tr.get("install.license_accept"));
    let checkbox_y = WIN_H - 124;
    let license_top = BANNER_H + PAD;
    let license_h = checkbox_y - license_top - 24;
    let license_w = wide(payload.license_text.as_deref().unwrap_or(LOREM));
    unsafe {
        let _ = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR(license_w.as_ptr()),
            WS_CHILD
                | WS_CLIPSIBLINGS
                | WS_VSCROLL
                | WINDOW_STYLE(ES_MULTILINE | ES_READONLY | ES_LEFT),
            PAD,
            license_top,
            WIN_W - PAD * 2,
            license_h,
            Some(hwnd),
            Some(HMENU(ID_LICENSE_EDIT as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(accept_w.as_ptr()),
            WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX),
            PAD,
            checkbox_y,
            WIN_W - PAD * 2,
            22,
            Some(hwnd),
            Some(HMENU(ID_ACCEPT_CHK as *mut _)),
            Some(hinst),
            None,
        );
    }
}

/// Choose view: destination label + path edit + Browse button.
unsafe fn build_choose(hwnd: HWND, hinst: HINSTANCE, default_path: &Path) {
    let tr = tr();
    let choose_label_w = wide(&tr.get("install.choose_label"));
    let browse_w = wide(&tr.get("install.browse"));
    let path_str = wide(&default_path.to_string_lossy());
    unsafe {
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(choose_label_w.as_ptr()),
            WS_CHILD,
            PAD,
            BANNER_H + PAD + 8,
            WIN_W - PAD * 2,
            20,
            Some(hwnd),
            Some(HMENU(ID_PATH_LABEL as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR(path_str.as_ptr()),
            WS_CHILD | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            PAD,
            BANNER_H + PAD + 32,
            WIN_W - PAD * 2 - 120,
            28,
            Some(hwnd),
            Some(HMENU(ID_PATH_EDIT as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(browse_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
            WIN_W - PAD - 110,
            BANNER_H + PAD + 32,
            110,
            28,
            Some(hwnd),
            Some(HMENU(ID_BROWSE_BTN as *mut _)),
            Some(hinst),
            None,
        );
        // Non-empty-folder warning, below the path row. Hidden until the chosen
        // folder is found to already contain files.
        let warn_y = BANNER_H + PAD + 72;
        const ICON_SZ: i32 = 20;
        let icon = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WINDOW_STYLE(SS_ICON | SS_REALSIZECONTROL),
            PAD,
            warn_y,
            ICON_SZ,
            ICON_SZ,
            Some(hwnd),
            Some(HMENU(ID_PATH_WARN_ICON as *mut _)),
            Some(hinst),
            None,
        );
        if let Ok(h) = icon
            && let Some(hicon) = stock_warning_icon()
        {
            SendMessageW(
                h,
                STM_SETICON,
                Some(WPARAM(hicon.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WINDOW_STYLE(SS_CENTERIMAGE),
            PAD + ICON_SZ + 8,
            warn_y,
            WIN_W - PAD * 2 - ICON_SZ - 8,
            ICON_SZ,
            Some(hwnd),
            Some(HMENU(ID_PATH_WARN as *mut _)),
            Some(hinst),
            None,
        );
    }
}

/// The shell's stock warning icon (small/16px), themed for the running Windows
/// version - flatter than the legacy `IDI_WARNING` triangle. Leaked for the
/// process lifetime (one handle); the OS reclaims it at exit.
unsafe fn stock_warning_icon() -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::Shell::{
        SHGSI_ICON, SHGSI_SMALLICON, SHGetStockIconInfo, SHSTOCKICONINFO, SIID_WARNING,
    };
    let mut sii = SHSTOCKICONINFO {
        cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        SHGetStockIconInfo(SIID_WARNING, SHGSI_ICON | SHGSI_SMALLICON, &mut sii).ok()?;
    }
    Some(sii.hIcon)
}

/// The shell's stock error icon (large/32px), themed for the running Windows
/// version. Leaked for the process lifetime; the OS reclaims it at exit.
unsafe fn stock_error_icon() -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::Shell::{
        SHGSI_ICON, SHGSI_LARGEICON, SHGetStockIconInfo, SHSTOCKICONINFO, SIID_ERROR,
    };
    let mut sii = SHSTOCKICONINFO {
        cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        SHGetStockIconInfo(SIID_ERROR, SHGSI_ICON | SHGSI_LARGEICON, &mut sii).ok()?;
    }
    Some(sii.hIcon)
}

/// Progress view: progress bar + status label.
unsafe fn build_progress(hwnd: HWND, hinst: HINSTANCE) {
    unsafe {
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
    }
}

/// Error view: modern-Windows error icon on the left + scrollable read-only
/// multiline edit on the right showing the full error detail.
unsafe fn build_error_box(hwnd: HWND, hinst: HINSTANCE) {
    const ICON_SZ: i32 = 32;
    const ICON_GAP: i32 = 12;
    let error_top = BANNER_H + PAD + 16;
    let error_h = WIN_H - 148 - error_top;
    let box_x = PAD + ICON_SZ + ICON_GAP;
    let box_w = WIN_W - PAD * 2 - ICON_SZ - ICON_GAP;
    unsafe {
        let icon = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(SS_ICON | SS_REALSIZECONTROL),
            PAD,
            error_top,
            ICON_SZ,
            ICON_SZ,
            Some(hwnd),
            Some(HMENU(ID_ERROR_ICON as *mut _)),
            Some(hinst),
            None,
        );
        if let Ok(h) = icon
            && let Some(hicon) = stock_error_icon()
        {
            SendMessageW(
                h,
                STM_SETICON,
                Some(WPARAM(hicon.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        let _ = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_CLIPSIBLINGS
                | WS_VSCROLL
                | WINDOW_STYLE(ES_MULTILINE | ES_READONLY | ES_LEFT),
            box_x,
            error_top,
            box_w,
            error_h,
            Some(hwnd),
            Some(HMENU(ID_ERROR_BOX as *mut _)),
            Some(hinst),
            None,
        );
    }
}

/// Done view extras: the "Run now" checkbox.
unsafe fn build_done(hwnd: HWND, hinst: HINSTANCE) {
    let run_now_w = wide(&tr().get("install.run_now"));
    unsafe {
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(run_now_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX),
            PAD,
            WIN_H - 124,
            WIN_W - PAD * 2,
            22,
            Some(hwnd),
            Some(HMENU(ID_LAUNCH_CHK as *mut _)),
            Some(hinst),
            None,
        );
    }
}

/// Shared bottom button row: Back, Next, Install, Cancel, Finish (shown per phase).
unsafe fn build_buttons(hwnd: HWND, hinst: HINSTANCE) {
    let tr = tr();
    let back_w = wide(&tr.get("install.back"));
    let next_w = wide(&tr.get("install.next"));
    let install_w = wide(&tr.get("install.install"));
    let cancel_w = wide(&tr.get("install.cancel"));
    let finish_w = wide(&tr.get("install.finish"));
    let btn_y = WIN_H - 84;
    unsafe {
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(back_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
            PAD,
            btn_y,
            100,
            32,
            Some(hwnd),
            Some(HMENU(ID_BACK_BTN as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(next_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON),
            WIN_W - PAD - 240,
            btn_y,
            110,
            32,
            Some(hwnd),
            Some(HMENU(ID_NEXT_BTN as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(install_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON),
            WIN_W - PAD - 240,
            btn_y,
            110,
            32,
            Some(hwnd),
            Some(HMENU(ID_INSTALL_BTN as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(cancel_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
            WIN_W - PAD - 120,
            btn_y,
            120,
            32,
            Some(hwnd),
            Some(HMENU(ID_CANCEL_BTN as *mut _)),
            Some(hinst),
            None,
        );
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(finish_w.as_ptr()),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON),
            WIN_W - PAD - 120,
            btn_y,
            120,
            32,
            Some(hwnd),
            Some(HMENU(ID_CLOSE_BTN as *mut _)),
            Some(hinst),
            None,
        );
    }
}

/// Reposition + resize every control for the given DPI. The coordinates here
/// are the single source of truth for the layout, in 96-dpi base units scaled
/// by `dpi`. Called once after creation and again on every `WM_DPICHANGED`
/// (move to a monitor of different scale). At 96 dpi this is identity, so
/// 100%-scale behaviour is unchanged.
pub(super) unsafe fn relayout(hwnd: HWND, dpi: i32) {
    let s = |v: i32| helpers::scale(v, dpi);

    let checkbox_y = WIN_H - 124;
    let license_top = BANNER_H + PAD;
    let license_h = checkbox_y - license_top - 24;
    let btn_y = WIN_H - 84;
    let warn_y = BANNER_H + PAD + 72;
    const ICON_SZ: i32 = 20;

    // (id, x, y, w, h) in 96-dpi base units - mirrors the build_* functions.
    let items: &[(usize, i32, i32, i32, i32)] = &[
        (ID_BANNER, 0, 0, WIN_W, BANNER_H),
        (ID_HEADER, PAD, 16, WIN_W - PAD * 2, 28),
        (ID_SUBHEADER, PAD, 46, WIN_W - PAD * 2, 20),
        (
            ID_LICENSE_EDIT,
            PAD,
            license_top,
            WIN_W - PAD * 2,
            license_h,
        ),
        (ID_ACCEPT_CHK, PAD, checkbox_y, WIN_W - PAD * 2, 22),
        (ID_PATH_LABEL, PAD, BANNER_H + PAD + 8, WIN_W - PAD * 2, 20),
        (
            ID_PATH_EDIT,
            PAD,
            BANNER_H + PAD + 32,
            WIN_W - PAD * 2 - 120,
            28,
        ),
        (
            ID_BROWSE_BTN,
            WIN_W - PAD - 110,
            BANNER_H + PAD + 32,
            110,
            28,
        ),
        (ID_PATH_WARN_ICON, PAD, warn_y, ICON_SZ, ICON_SZ),
        (
            ID_PATH_WARN,
            PAD + ICON_SZ + 8,
            warn_y,
            WIN_W - PAD * 2 - ICON_SZ - 8,
            ICON_SZ,
        ),
        (ID_PROGRESS, PAD, BANNER_H + PAD + 16, WIN_W - PAD * 2, 22),
        (ID_STATUS, PAD, BANNER_H + PAD + 48, WIN_W - PAD * 2, 48),
        (ID_ERROR_ICON, PAD, BANNER_H + PAD + 16, 32, 32),
        (
            ID_ERROR_BOX,
            PAD + 32 + 12,
            BANNER_H + PAD + 16,
            WIN_W - PAD * 2 - 32 - 12,
            WIN_H - 148 - (BANNER_H + PAD + 16),
        ),
        (ID_LAUNCH_CHK, PAD, WIN_H - 124, WIN_W - PAD * 2, 22),
        (ID_BACK_BTN, PAD, btn_y, 100, 32),
        (ID_NEXT_BTN, WIN_W - PAD - 240, btn_y, 110, 32),
        (ID_INSTALL_BTN, WIN_W - PAD - 240, btn_y, 110, 32),
        (ID_CANCEL_BTN, WIN_W - PAD - 120, btn_y, 120, 32),
        (ID_CLOSE_BTN, WIN_W - PAD - 120, btn_y, 120, 32),
    ];
    unsafe {
        for &(id, x, y, w, h) in items {
            let ctrl = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
            if !ctrl.is_invalid() {
                let _ = MoveWindow(ctrl, s(x), s(y), s(w), s(h), true);
            }
        }
    }
    // Plugin pages keep their own base-rect layout.
    super::plugin_pages::relayout(hwnd, dpi);
}

pub(super) unsafe fn apply_fonts(hwnd: HWND) {
    STATE.with(|s| {
        let Some(st) = s.borrow().as_ref().cloned() else {
            return;
        };
        let st = st.borrow();
        unsafe {
            helpers::set_font(hwnd, ID_HEADER, st.font_header);
            helpers::set_font(hwnd, ID_SUBHEADER, st.font_normal);
            for id in [
                ID_PATH_LABEL,
                ID_PATH_EDIT,
                ID_PATH_WARN,
                ID_BROWSE_BTN,
                ID_INSTALL_BTN,
                ID_CANCEL_BTN,
                ID_PROGRESS,
                ID_STATUS,
                ID_ERROR_BOX,
                ID_CLOSE_BTN,
                ID_LICENSE_EDIT,
                ID_ACCEPT_CHK,
                ID_NEXT_BTN,
                ID_BACK_BTN,
                ID_LAUNCH_CHK,
            ] {
                helpers::set_font(hwnd, id, st.font_normal);
            }
        }
    });
    super::plugin_pages::apply_fonts(hwnd);
}
