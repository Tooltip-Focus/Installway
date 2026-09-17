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
use crate::ui::helpers::{
    self, CHECKBOX, ControlRect, DEFAULT_BUTTON, PUSH_BUTTON, child, child_ex, move_controls,
    set_static_icon,
};
use common::model::installer_payload::InstallerPayload;
use common::model::payload_kind::PayloadKind;
use std::path::Path;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::PROGRESS_CLASSW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const SS_ICON: u32 = 0x0003;
const SS_CENTERIMAGE: u32 = 0x0200;
const SS_REALSIZECONTROL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const ES_MULTILINE: u32 = 0x0004;
const ES_LEFT: u32 = 0x0000;
const WS_VSCROLL: WINDOW_STYLE = WINDOW_STYLE(0x0020_0000);
/// Scrollable read-only multiline text: the license and the error detail.
const READ_ONLY_TEXT: WINDOW_STYLE =
    WINDOW_STYLE(WS_CLIPSIBLINGS.0 | WS_VSCROLL.0 | ES_MULTILINE | ES_READONLY | ES_LEFT);

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
    unsafe {
        build_banner_header(hwnd, payload);
        build_license(hwnd, payload);
        build_choose(hwnd, default_path);
        build_progress(hwnd);
        build_error_box(hwnd);
        build_done(hwnd);
        build_buttons(hwnd);
        apply_fonts(hwnd);
    }
}

/// Banner strip + product header + subheader (always visible).
unsafe fn build_banner_header(hwnd: HWND, payload: &InstallerPayload) {
    let tr = tr();
    let header = tr.fmt(
        "install.header",
        &[
            ("product", &payload.product),
            ("version", &payload.to_version),
        ],
    );
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
    unsafe {
        // Flat accent strip, filled by WM_CTLCOLORSTATIC. Skipped when a banner
        // image is packaged: the parent then paints it in WM_PAINT instead.
        if !super::has_banner_image() {
            child(hwnd, w!("STATIC"), "", WS_VISIBLE, ID_BANNER);
        }
        child(hwnd, w!("STATIC"), &header, WS_VISIBLE, ID_HEADER);
        child(hwnd, w!("STATIC"), &sub, WS_VISIBLE, ID_SUBHEADER);
    }
    // Remember the product banner so plugin pages can borrow the banner and the
    // built-in phases can restore it (see `super::apply_phase`).
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            let mut st = st.borrow_mut();
            st.header_text = header;
            st.sub_text = sub;
        }
    });
}

/// License view: read-only EULA edit + "I accept" checkbox.
unsafe fn build_license(hwnd: HWND, payload: &InstallerPayload) {
    let license = payload.license_text.as_deref().unwrap_or(LOREM);
    let accept = tr().get("install.license_accept");
    unsafe {
        child_ex(
            hwnd,
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            license,
            READ_ONLY_TEXT,
            ID_LICENSE_EDIT,
        );
        child(
            hwnd,
            w!("BUTTON"),
            &accept,
            WS_CLIPSIBLINGS | CHECKBOX,
            ID_ACCEPT_CHK,
        );
    }
}

/// Choose view: destination label + path edit + Browse button, and the
/// non-empty-folder warning below them, hidden until the chosen folder is found
/// to already contain files.
unsafe fn build_choose(hwnd: HWND, default_path: &Path) {
    let tr = tr();
    let path = default_path.to_string_lossy();
    let path_style = WINDOW_STYLE(ES_AUTOHSCROLL as u32);
    let icon_style = WINDOW_STYLE(SS_ICON | SS_REALSIZECONTROL);
    unsafe {
        let label = tr.get("install.choose_label");
        child(hwnd, w!("STATIC"), &label, WINDOW_STYLE(0), ID_PATH_LABEL);
        child_ex(
            hwnd,
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            &path,
            path_style,
            ID_PATH_EDIT,
        );
        child(
            hwnd,
            w!("BUTTON"),
            &tr.get("install.browse"),
            PUSH_BUTTON,
            ID_BROWSE_BTN,
        );
        let icon = child(hwnd, w!("STATIC"), "", icon_style, ID_PATH_WARN_ICON);
        if let Some(hicon) = stock_warning_icon() {
            set_static_icon(icon, hicon);
        }
        let warn_style = WINDOW_STYLE(SS_CENTERIMAGE);
        child(hwnd, w!("STATIC"), "", warn_style, ID_PATH_WARN);
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
unsafe fn build_progress(hwnd: HWND) {
    unsafe {
        child(hwnd, PROGRESS_CLASSW, "", WINDOW_STYLE(0), ID_PROGRESS);
        child(hwnd, w!("STATIC"), "", WINDOW_STYLE(0), ID_STATUS);
    }
}

/// Error view: modern-Windows error icon on the left + scrollable read-only
/// multiline edit on the right showing the full error detail.
unsafe fn build_error_box(hwnd: HWND) {
    let icon_style = WS_CLIPSIBLINGS | WINDOW_STYLE(SS_ICON | SS_REALSIZECONTROL);
    unsafe {
        let icon = child(hwnd, w!("STATIC"), "", icon_style, ID_ERROR_ICON);
        if let Some(hicon) = stock_error_icon() {
            set_static_icon(icon, hicon);
        }
        child_ex(
            hwnd,
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            "",
            READ_ONLY_TEXT,
            ID_ERROR_BOX,
        );
    }
}

/// Done view extras: the "Run now" checkbox.
unsafe fn build_done(hwnd: HWND) {
    let run_now = tr().get("install.run_now");
    unsafe { child(hwnd, w!("BUTTON"), &run_now, CHECKBOX, ID_LAUNCH_CHK) };
}

/// Shared bottom button row: Back, Next, Install, Cancel, Finish (shown per phase).
unsafe fn build_buttons(hwnd: HWND) {
    let tr = tr();
    for (key, style, id) in [
        ("install.back", PUSH_BUTTON, ID_BACK_BTN),
        ("install.next", DEFAULT_BUTTON, ID_NEXT_BTN),
        ("install.install", DEFAULT_BUTTON, ID_INSTALL_BTN),
        ("install.cancel", PUSH_BUTTON, ID_CANCEL_BTN),
        ("install.finish", DEFAULT_BUTTON, ID_CLOSE_BTN),
    ] {
        unsafe { child(hwnd, w!("BUTTON"), &tr.get(key), style, id) };
    }
}

/// Place every control for the given DPI. The coordinates here are the single
/// source of truth for the layout, in 96-dpi base units scaled by `dpi`: the
/// build functions above create the controls without a position. Called once
/// after creation and again on every `WM_DPICHANGED` (move to a monitor of
/// different scale).
pub(super) unsafe fn relayout(hwnd: HWND, dpi: i32) {
    let checkbox_y = WIN_H - 124;
    let license_top = BANNER_H + PAD;
    let license_h = checkbox_y - license_top - 24;
    let btn_y = WIN_H - 84;
    let warn_y = BANNER_H + PAD + 72;
    const ICON_SZ: i32 = 20;

    let items: &[ControlRect] = &[
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
    move_controls(hwnd, dpi, items);
    // Plugin pages keep their own base-rect layout.
    super::plugin_pages::relayout(hwnd, dpi);
}

pub(super) unsafe fn apply_fonts(hwnd: HWND) {
    STATE.with(|s| {
        let Some(st) = s.borrow().as_ref().cloned() else {
            return;
        };
        let st = st.borrow();
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
    });
    // Plugin pages keep their own controls; rescale their fonts too (mirrors
    // `relayout`), so a plugin progress page stays crisp after a DPI change.
    super::plugin_pages::apply_fonts(hwnd);
}
