// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Dynamic, data-driven wizard pages contributed by `ui = true` plugins (ABI
//! v2). Unlike the built-in views (whose controls are hardcoded), these are
//! created at runtime from each plugin's [`common::model::plugin_page::PluginPage`]
//! descriptor: one control per widget, laid out top-to-bottom, shown/hidden per
//! `Phase::Plugin(i)`. Controls are created once and only shown/hidden (never
//! destroyed mid-flow), mirroring the built-in pages' lifecycle.
//!
//! The collected answers land in [`super::PLUGIN_INPUTS`] keyed
//! `"<page_id>.<widget_id>"`, threaded to the plugin's `installway_up` later.

use super::{
    BANNER_H, BM_GETCHECK, BM_SETCHECK, ID_HEADER, ID_PLUGIN_BASE, ID_SUBHEADER, PAD, STATE, WIN_H,
    WIN_W, WIZARD,
};
use crate::ui::helpers;
pub(super) use crate::ui::wizard_engine::{
    Step, StepOutcome, run_plugin_then_step, run_step_query,
};
use common::model::choice_style::ChoiceStyle;
#[cfg(debug_assertions)]
use common::model::page_step::PageStep;
use common::model::plugin_page::PluginInputs;
use common::model::plugin_page::PluginPage;
use common::model::plugin_widget::PluginWidget;
use common::utils::wide;
use std::ops::{Deref, DerefMut};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const BS_AUTOCHECKBOX: u32 = 0x0003;
const BS_AUTORADIOBUTTON: u32 = 0x0009;
const WS_GROUP_S: WINDOW_STYLE = WINDOW_STYLE(0x0002_0000);
const WS_VSCROLL_S: WINDOW_STYLE = WINDOW_STYLE(0x0020_0000);
const CBS_DROPDOWNLIST: u32 = 0x0003;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const EM_SETCUEBANNER: u32 = 0x1501;
const ES_PASSWORD: u32 = 0x0020;
const ES_NUMBER: u32 = 0x2000;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;

const ROW_GAP: i32 = 10;

/// How a built control is read back at collect time.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum FieldKind {
    Label,
    Text,
    Checkbox,
    Radio,
    Combo,
    /// Checkbox group; value is the checked option values joined by `,`.
    MultiCheck,
    /// Marker only — no Win32 control, no collected value. `true` = marquee,
    /// `false` = deterministic (plugin drives % via `emit_progress`).
    Progress(bool),
}

/// One rendered widget: every HWND it owns (with base-DPI layout), plus what's
/// needed to show/hide, re-font, and read it back.
pub(super) struct PluginField {
    /// Index into [`super::PLUGIN_PAGES`] (the flattened page list).
    pub page: usize,
    /// `"<page_id>.<widget_id>"`; empty for a label (contributes no value).
    pub key: String,
    pub kind: FieldKind,
    pub required: bool,
    /// Value-bearing control ids (one for text/checkbox/combo; one per option
    /// for radio). Read in `read_field`.
    pub ctrl_ids: Vec<usize>,
    /// SingleChoice option values, aligned to `ctrl_ids` (radio) or combo items.
    pub values: Vec<String>,
    /// Every owned control id with its 96-dpi base rect `(id, x, y, w, h)`,
    /// scaled in `relayout`.
    pub rects: Vec<(usize, i32, i32, i32, i32)>,
}

fn key(page_id: &str, widget_id: &str) -> String {
    format!("{page_id}.{widget_id}")
}

/// The plugin-page wizard, with the controls it builds for each page.
///
/// Pages are built on demand (one slot of controls each) and only shown/hidden —
/// never destroyed; a back-then-branch leaves a few hidden orphan controls, freed
/// at window close. Going Back just re-shows a prior page's retained controls.
pub(super) struct Wizard {
    /// The shared wizard state; each frame's UI state is its control slot.
    flow: crate::ui::wizard_engine::Wizard<usize>,
    next_slot: usize,
    /// Running control-id allocator (unique across pages).
    next_id: usize,
}

impl Deref for Wizard {
    type Target = crate::ui::wizard_engine::Wizard<usize>;
    fn deref(&self) -> &Self::Target {
        &self.flow
    }
}

impl DerefMut for Wizard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.flow
    }
}

impl Wizard {
    pub(super) fn new(
        plugins: Vec<(common::model::plugin_entry::PluginEntry, std::path::PathBuf)>,
        base_ctx: common::model::plugin_ctx::PluginContext,
        self_exe: std::path::PathBuf,
        tmp: Option<std::sync::Arc<crate::extract::TempDirGuard>>,
    ) -> Self {
        Self::with_flow(crate::ui::wizard_engine::Wizard::new(
            plugins, base_ctx, self_exe, tmp,
        ))
    }

    /// A preview wizard that replays `steps` for one synthetic plugin.
    #[cfg(debug_assertions)]
    pub(super) fn canned(steps: Vec<PageStep>) -> Self {
        Self::with_flow(crate::ui::wizard_engine::Wizard::canned(steps))
    }

    fn with_flow(flow: crate::ui::wizard_engine::Wizard<usize>) -> Self {
        Wizard {
            flow,
            next_slot: 0,
            next_id: ID_PLUGIN_BASE,
        }
    }

    pub(super) fn current_slot(&self) -> Option<usize> {
        self.current().map(|f| f.ui)
    }

    /// First page of the whole flow (canned preview only — the real wizard
    /// dispatches a background query; see `handlers::dispatch_plugin_query`).
    pub(super) unsafe fn start(&mut self, hwnd: HWND) -> Step {
        let Wizard {
            flow,
            next_slot,
            next_id,
        } = self;
        flow.step_canned(|page| unsafe { build_page(hwnd, page, next_slot, next_id) })
    }

    /// Collect the current page, then replay the next canned step.
    pub(super) unsafe fn forward(&mut self, hwnd: HWND) -> Step {
        if !unsafe { self.collect_page(hwnd) } {
            return Step::Stay; // required field missing (already warned)
        }
        unsafe { self.start(hwnd) }
    }

    /// Collect the current page's answers. Returns `false` (after warning the
    /// user) when a required field is empty. UI thread only.
    pub(super) unsafe fn collect_page(&mut self, hwnd: HWND) -> bool {
        let Some(slot) = self.current_slot() else {
            return true;
        };
        let Some(vals) = (unsafe { collect_slot(hwnd, slot) }) else {
            return false;
        };
        let mut answers = self.answers().clone();
        answers.extend(vals);
        self.commit(answers);
        true
    }

    /// Apply a `StepOutcome` from the background thread, building the new
    /// page's controls. UI thread only.
    pub(super) unsafe fn apply_step_outcome(&mut self, hwnd: HWND, outcome: StepOutcome) -> Step {
        let Wizard {
            flow,
            next_slot,
            next_id,
        } = self;
        flow.apply_outcome(outcome, |page| unsafe {
            build_page(hwnd, page, next_slot, next_id)
        })
    }
}

/// Build one page's controls in a fresh slot, returned for its frame.
unsafe fn build_page(
    hwnd: HWND,
    page: &PluginPage,
    next_slot: &mut usize,
    next_id: &mut usize,
) -> usize {
    let slot = *next_slot;
    *next_slot += 1;
    let content_w = WIN_W - PAD * 2;
    let mut y = BANNER_H + PAD + 8;
    let mut fields = Vec::new();
    for widget in &page.widgets {
        let f = unsafe { build_widget(hwnd, slot, &page.id, widget, content_w, &mut y, next_id) };
        fields.push(f);
    }
    // Button row occupies the bottom ~84 px; content past that is clipped.
    let content_limit = WIN_H - 84;
    if y > content_limit {
        common::log::warn(format!(
            "plugin page '{}': content {}px exceeds window {}px — bottom widgets clipped",
            page.id, y, content_limit
        ));
    }
    let dpi = STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            st.borrow_mut().plugin_fields.extend(fields);
            st.borrow().dpi
        } else {
            96
        }
    });
    relayout(hwnd, dpi);
    apply_fonts(hwnd);
    slot
}

// ---- Auto-run helpers ---------------------------------------------------

const PBS_MARQUEE: u32 = 0x0008;
const PBM_SETMARQUEE: u32 = 0x400 + 10;
const PBM_SETPOS: u32 = 0x402;
const PBM_SETRANGE: u32 = 0x401;

/// Find the progress bar control for the current slot, if any.
fn current_progress_bar(hwnd: HWND) -> Option<HWND> {
    let slot = current_slot()?;
    STATE.with(|s| {
        s.borrow().as_ref().and_then(|st| {
            st.borrow()
                .plugin_fields
                .iter()
                .find(|f| f.page == slot && matches!(f.kind, FieldKind::Progress(_)))
                .and_then(|f| f.ctrl_ids.first().copied())
                .map(|id| unsafe { GetDlgItem(Some(hwnd), id as i32).unwrap_or_default() })
        })
    })
}

/// Set marquee mode on the current slot's progress bar control and initialise its range.
fn set_slot_marquee(hwnd: HWND, marquee: bool) {
    let Some(bar) = current_progress_bar(hwnd) else {
        return;
    };
    if bar.is_invalid() {
        return;
    }
    unsafe {
        let style = GetWindowLongW(bar, GWL_STYLE) as u32;
        if marquee {
            SetWindowLongW(bar, GWL_STYLE, (style | PBS_MARQUEE) as i32);
            SendMessageW(bar, PBM_SETMARQUEE, Some(WPARAM(1)), Some(LPARAM(60)));
        } else {
            SendMessageW(bar, PBM_SETMARQUEE, Some(WPARAM(0)), Some(LPARAM(0)));
            SetWindowLongW(bar, GWL_STYLE, (style & !PBS_MARQUEE) as i32);
            // Range 0–10000 matches the host's standard progress scale.
            SendMessageW(
                bar,
                PBM_SETRANGE,
                Some(WPARAM(0)),
                Some(LPARAM(10000 << 16)),
            );
            SendMessageW(bar, PBM_SETPOS, Some(WPARAM(0)), None);
        }
    }
}

/// Called from `act_step` when entering an auto-run page: hide nav buttons and
/// start the progress bar in the correct mode.
pub(super) fn apply_auto_run(hwnd: HWND, marquee: bool) {
    unsafe {
        for id in [super::ID_BACK_BTN, super::ID_NEXT_BTN, super::ID_CANCEL_BTN] {
            let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
    set_slot_marquee(hwnd, marquee);
}

/// Drive the deterministic progress bar on the current slot's Progress widget.
/// `scaled` is 0–10000 (same scale as the host's standard progress bar).
pub(super) fn update_current_progress(hwnd: HWND, scaled: i32) {
    let Some(bar) = current_progress_bar(hwnd) else {
        return;
    };
    if !bar.is_invalid() {
        unsafe {
            SendMessageW(bar, PBM_SETPOS, Some(WPARAM(scaled as usize)), None);
        }
    }
}

/// Append a STATIC label control for widgets that have a non-empty label.
/// Mutates `rects`, `y`, and `next_id` in place; no-op when `label` is empty.
#[allow(clippy::too_many_arguments)]
unsafe fn build_label_row(
    hwnd: HWND,
    x: i32,
    y: &mut i32,
    content_w: i32,
    label: &str,
    next_id: &mut usize,
    rects: &mut Vec<(usize, i32, i32, i32, i32)>,
) {
    if label.is_empty() {
        return;
    }
    let lid = *next_id;
    *next_id += 1;
    unsafe {
        helpers::child_ex(
            hwnd,
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            label,
            WS_CLIPSIBLINGS | WINDOW_STYLE(0),
            lid,
        );
    }
    rects.push((lid, x, *y, content_w, 20));
    *y += 22;
}

/// Create the controls for one widget, advancing the layout cursor `y` and the
/// id counter, and return its [`PluginField`].
#[allow(clippy::too_many_arguments)]
unsafe fn build_widget(
    hwnd: HWND,
    page: usize,
    page_id: &str,
    widget: &PluginWidget,
    content_w: i32,
    y: &mut i32,
    next_id: &mut usize,
) -> PluginField {
    let x = PAD;
    match widget {
        PluginWidget::Label { text, .. } => {
            let id = *next_id;
            *next_id += 1;
            unsafe {
                helpers::child_ex(
                    hwnd,
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    text,
                    WS_CLIPSIBLINGS | WINDOW_STYLE(0),
                    id,
                );
            }
            let rects = vec![(id, x, *y, content_w, 20)];
            *y += 20 + ROW_GAP;
            PluginField {
                page,
                key: String::new(),
                kind: FieldKind::Label,
                required: false,
                ctrl_ids: vec![],
                values: vec![],
                rects,
            }
        }
        PluginWidget::Text {
            id: wid,
            label,
            default,
            required,
            placeholder,
            password,
            number,
            multiline,
        } => {
            let mut rects = Vec::new();
            unsafe { build_label_row(hwnd, x, y, content_w, label, next_id, &mut rects) };
            let mut alloc = || {
                let id = *next_id;
                *next_id += 1;
                id
            };
            let mut style = WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_TABSTOP;
            if *password {
                style |= WINDOW_STYLE(ES_PASSWORD);
            }
            if *number {
                style |= WINDOW_STYLE(ES_NUMBER);
            }
            if *multiline {
                style |= WINDOW_STYLE(ES_MULTILINE | ES_AUTOVSCROLL) | WS_VSCROLL_S;
            }
            let h_px = if *multiline { 72 } else { 28 };
            let eid = alloc();
            unsafe {
                let h = helpers::child_ex(
                    hwnd,
                    WS_EX_CLIENTEDGE,
                    w!("EDIT"),
                    default,
                    WS_CLIPSIBLINGS | style,
                    eid,
                );
                if !placeholder.is_empty() && !*multiline {
                    let p = wide(placeholder);
                    SendMessageW(
                        h,
                        EM_SETCUEBANNER,
                        Some(WPARAM(1)),
                        Some(LPARAM(p.as_ptr() as isize)),
                    );
                }
            }
            rects.push((eid, x, *y, content_w, h_px));
            *y += h_px + ROW_GAP;
            PluginField {
                page,
                key: key(page_id, wid),
                kind: FieldKind::Text,
                required: *required,
                ctrl_ids: vec![eid],
                values: vec![],
                rects,
            }
        }
        PluginWidget::Checkbox {
            id: wid,
            label,
            default,
        } => {
            let cid = *next_id;
            *next_id += 1;
            unsafe {
                let h = helpers::child_ex(
                    hwnd,
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    label,
                    WS_CLIPSIBLINGS | WINDOW_STYLE(BS_AUTOCHECKBOX) | WS_TABSTOP,
                    cid,
                );
                if *default {
                    SendMessageW(
                        h,
                        BM_SETCHECK,
                        Some(WPARAM(BST_CHECKED.0 as usize)),
                        Some(LPARAM(0)),
                    );
                }
            }
            let rects = vec![(cid, x, *y, content_w, 22)];
            *y += 22 + ROW_GAP;
            PluginField {
                page,
                key: key(page_id, wid),
                kind: FieldKind::Checkbox,
                required: false,
                ctrl_ids: vec![cid],
                values: vec![],
                rects,
            }
        }
        PluginWidget::SingleChoice {
            id: wid,
            label,
            options,
            style,
            default,
            required,
        } => {
            let mut rects = Vec::new();
            unsafe { build_label_row(hwnd, x, y, content_w, label, next_id, &mut rects) };
            let mut alloc = || {
                let id = *next_id;
                *next_id += 1;
                id
            };
            let default_idx = options
                .iter()
                .position(|o| o.value == *default)
                .unwrap_or(0);
            let values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
            match style {
                ChoiceStyle::Combo => {
                    let cid = alloc();
                    unsafe {
                        // The height arg is the dropped-down extent; the closed box
                        // occupies one row, so the cursor only advances ~28.
                        let h = helpers::child_ex(
                            hwnd,
                            WINDOW_EX_STYLE(0),
                            w!("COMBOBOX"),
                            "",
                            WS_CLIPSIBLINGS
                                | WINDOW_STYLE(CBS_DROPDOWNLIST)
                                | WS_TABSTOP
                                | WS_VSCROLL_S,
                            cid,
                        );
                        for o in options {
                            let s = wide(&o.label);
                            SendMessageW(
                                h,
                                CB_ADDSTRING,
                                Some(WPARAM(0)),
                                Some(LPARAM(s.as_ptr() as isize)),
                            );
                        }
                        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(default_idx)), Some(LPARAM(0)));
                    }
                    rects.push((cid, x, *y, content_w, 200));
                    *y += 28 + ROW_GAP;
                    PluginField {
                        page,
                        key: key(page_id, wid),
                        kind: FieldKind::Combo,
                        required: *required,
                        ctrl_ids: vec![cid],
                        values,
                        rects,
                    }
                }
                ChoiceStyle::Radio => {
                    let mut ids = Vec::new();
                    for (i, o) in options.iter().enumerate() {
                        let rid = alloc();
                        let style = if i == 0 {
                            WINDOW_STYLE(BS_AUTORADIOBUTTON) | WS_TABSTOP | WS_GROUP_S
                        } else {
                            WINDOW_STYLE(BS_AUTORADIOBUTTON) | WS_TABSTOP
                        };
                        unsafe {
                            let h = helpers::child_ex(
                                hwnd,
                                WINDOW_EX_STYLE(0),
                                w!("BUTTON"),
                                &o.label,
                                WS_CLIPSIBLINGS | style,
                                rid,
                            );
                            if i == default_idx {
                                SendMessageW(
                                    h,
                                    BM_SETCHECK,
                                    Some(WPARAM(BST_CHECKED.0 as usize)),
                                    Some(LPARAM(0)),
                                );
                            }
                        }
                        rects.push((rid, x + 16, *y, content_w - 16, 22));
                        ids.push(rid);
                        *y += 24;
                    }
                    *y += ROW_GAP;
                    PluginField {
                        page,
                        key: key(page_id, wid),
                        kind: FieldKind::Radio,
                        required: *required,
                        ctrl_ids: ids,
                        values,
                        rects,
                    }
                }
            }
        }
        PluginWidget::MultiChoice {
            id: wid,
            label,
            options,
            default,
            required,
        } => {
            let mut rects = Vec::new();
            unsafe { build_label_row(hwnd, x, y, content_w, label, next_id, &mut rects) };
            let mut alloc = || {
                let id = *next_id;
                *next_id += 1;
                id
            };
            let values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
            let mut ids = Vec::new();
            for o in options {
                let cid = alloc();
                unsafe {
                    let h = helpers::child_ex(
                        hwnd,
                        WINDOW_EX_STYLE(0),
                        w!("BUTTON"),
                        &o.label,
                        WS_CLIPSIBLINGS | WINDOW_STYLE(BS_AUTOCHECKBOX) | WS_TABSTOP,
                        cid,
                    );
                    if default.contains(&o.value) {
                        SendMessageW(
                            h,
                            BM_SETCHECK,
                            Some(WPARAM(BST_CHECKED.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                }
                rects.push((cid, x + 16, *y, content_w - 16, 22));
                ids.push(cid);
                *y += 24;
            }
            *y += ROW_GAP;
            PluginField {
                page,
                key: key(page_id, wid),
                kind: FieldKind::MultiCheck,
                required: *required,
                ctrl_ids: ids,
                values,
                rects,
            }
        }
        PluginWidget::Progress { marquee } => {
            let id = *next_id;
            *next_id += 1;
            unsafe {
                helpers::child_ex(
                    hwnd,
                    WINDOW_EX_STYLE(0),
                    w!("msctls_progress32"),
                    "",
                    WS_CLIPSIBLINGS | WINDOW_STYLE(0),
                    id,
                );
            }
            let rects = vec![(id, x, *y, content_w, 20)];
            *y += 20 + ROW_GAP;
            PluginField {
                page,
                key: String::new(),
                kind: FieldKind::Progress(*marquee),
                required: false,
                ctrl_ids: vec![id],
                values: vec![],
                rects,
            }
        }
    }
}

fn with_state<F: FnOnce(&super::UiState)>(f: F) {
    STATE.with(|st| {
        let Some(state) = st.borrow().as_ref().cloned() else {
            return;
        };
        f(&state.borrow());
    });
}

/// Reposition every plugin control for `dpi` (mirrors `views::relayout`).
pub(super) fn relayout(hwnd: HWND, dpi: i32) {
    with_state(|state| {
        for f in &state.plugin_fields {
            helpers::move_controls(hwnd, dpi, &f.rects);
        }
    });
}

/// Apply the normal font to every plugin control (mirrors `views::apply_fonts`).
pub(super) fn apply_fonts(hwnd: HWND) {
    with_state(|state| {
        let font = state.font_normal;
        for f in &state.plugin_fields {
            for &(id, ..) in &f.rects {
                helpers::set_font(hwnd, id, font);
            }
        }
    });
}

/// The slot of the wizard's current page, if any.
pub(super) fn current_slot() -> Option<usize> {
    WIZARD.with(|w| w.borrow().as_ref().and_then(|z| z.current_slot()))
}

/// Show only `active`'s controls; hide every plugin control when `None`.
pub(super) unsafe fn apply_visibility(hwnd: HWND, active: Option<usize>) {
    with_state(|state| unsafe {
        for f in &state.plugin_fields {
            let vis = active == Some(f.page);
            for &(id, ..) in &f.rects {
                let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
                let _ = ShowWindow(h, if vis { SW_SHOW } else { SW_HIDE });
            }
        }
    });
}

/// Put the current page's title + subtitle/notice in the banner (verbatim — the
/// plugin already localized them).
pub(super) unsafe fn set_banner(hwnd: HWND) {
    let (title, subtitle) = WIZARD.with(|w| {
        w.borrow()
            .as_ref()
            .map(|z| z.current_title())
            .unwrap_or_default()
    });
    helpers::set_dlg_text(hwnd, ID_HEADER, &title);
    helpers::set_dlg_text(hwnd, ID_SUBHEADER, &subtitle);
}

/// A field snapshot taken under the `STATE` borrow: `(kind, required, key,
/// ctrl_ids, option values)`. Read back by `read_field` after the borrow drops.
type FieldSnapshot = (FieldKind, bool, String, Vec<usize>, Vec<String>);

/// Read the answers on `slot`. `None` (after a warning) if a required field is
/// empty/unselected, so the caller can keep the user on the page.
unsafe fn collect_slot(hwnd: HWND, slot: usize) -> Option<PluginInputs> {
    // Snapshot the slot's fields first so the STATE borrow is released before we
    // touch controls or show a message box.
    let fields: Vec<FieldSnapshot> = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| {
                st.borrow()
                    .plugin_fields
                    .iter()
                    .filter(|f| {
                        f.page == slot
                            && !matches!(f.kind, FieldKind::Label | FieldKind::Progress(_))
                    })
                    .map(|f| {
                        (
                            f.kind,
                            f.required,
                            f.key.clone(),
                            f.ctrl_ids.clone(),
                            f.values.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    });

    let mut out = PluginInputs::new();
    for (kind, required, key, ctrl_ids, values) in fields {
        match unsafe { read_field(hwnd, kind, &ctrl_ids, &values) } {
            Some(v) => {
                out.insert(key, v);
            }
            None if required => {
                unsafe {
                    super::message_box(
                        hwnd,
                        &super::tr().get("install.field_required"),
                        MB_ICONWARNING,
                    )
                };
                return None;
            }
            None => {
                out.insert(key, String::new());
            }
        }
    }
    Some(out)
}

/// Read one field's current value. `None` means empty/unselected (a required
/// field then fails validation; a non-required one stores an empty string).
unsafe fn read_field(
    hwnd: HWND,
    kind: FieldKind,
    ids: &[usize],
    values: &[String],
) -> Option<String> {
    unsafe {
        match kind {
            FieldKind::Label | FieldKind::Progress(_) => None,
            FieldKind::Text => {
                let h = GetDlgItem(Some(hwnd), ids[0] as i32).unwrap_or_default();
                let t = helpers::get_window_text(h);
                if t.trim().is_empty() { None } else { Some(t) }
            }
            FieldKind::Checkbox => {
                let h = GetDlgItem(Some(hwnd), ids[0] as i32).unwrap_or_default();
                let checked = SendMessageW(h, BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0;
                Some(if checked {
                    "true".into()
                } else {
                    "false".into()
                })
            }
            FieldKind::Radio => {
                for (i, id) in ids.iter().enumerate() {
                    let h = GetDlgItem(Some(hwnd), *id as i32).unwrap_or_default();
                    if SendMessageW(h, BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0 {
                        return values.get(i).cloned();
                    }
                }
                None
            }
            FieldKind::Combo => {
                let h = GetDlgItem(Some(hwnd), ids[0] as i32).unwrap_or_default();
                let idx = SendMessageW(h, CB_GETCURSEL, None, None).0;
                if idx < 0 {
                    None
                } else {
                    values.get(idx as usize).cloned()
                }
            }
            FieldKind::MultiCheck => {
                let picked: Vec<&str> = ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| {
                        let h = GetDlgItem(Some(hwnd), **id as i32).unwrap_or_default();
                        SendMessageW(h, BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0
                    })
                    .filter_map(|(i, _)| values.get(i).map(String::as_str))
                    .collect();
                if picked.is_empty() {
                    None
                } else {
                    Some(picked.join(","))
                }
            }
        }
    }
}
