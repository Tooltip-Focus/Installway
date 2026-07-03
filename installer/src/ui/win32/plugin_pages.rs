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
use crate::extract::TempDirGuard;
use crate::ui::helpers;
use common::model::choice_style::ChoiceStyle;
use common::model::page_step::PageStep;
use common::model::plugin_ctx::PluginCtx;
use common::model::plugin_entry::PluginEntry;
use common::model::plugin_page::PluginInputs;
use common::model::plugin_page::PluginPage;
use common::model::plugin_widget::PluginWidget;
use common::plugin::InputsByPlugin;
use common::utils::wide;
use std::path::PathBuf;
use std::sync::Arc;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

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

/// What the caller does after a wizard transition.
pub(super) enum Step {
    /// Render the now-current page (controls already built).
    Show,
    /// Every plugin finished — proceed to install with [`Wizard::inputs`].
    Install,
    /// Validation failed; stay on the current page (already warned).
    Stay,
    /// Backed out before the first plugin page — return to the built-in flow.
    Exit,
    /// Page had `buttons: false` — run the plugin's `up` in the background.
    AutoRun { marquee: bool },
}

/// One shown page in the current plugin's path (kept for Back).
struct Frame {
    slot: usize,
    page: PluginPage,
    back: bool,
    notice: String,
}

/// Drives the per-plugin step loop: ask the plugin for the next page given the
/// answers so far, render it, collect, repeat until `Done`, then the next plugin.
/// The plugin stays a stateless step function; all state lives here.
///
/// Pages are built on demand (one slot of controls each) and only shown/hidden —
/// never destroyed; a back-then-branch leaves a few hidden orphan controls, freed
/// at window close. Going Back just re-shows a prior page's retained controls.
pub(super) struct Wizard {
    plugins: Vec<(common::model::plugin_entry::PluginEntry, PathBuf)>, // (entry, extracted dll)
    base_ctx: PluginCtx,
    self_exe: PathBuf,
    /// Keeps the extracted-DLL temp dir alive. Shared (via `Arc`) into every
    /// background step query so a detached query thread can't read a DLL after
    /// the dir is removed (e.g. the user closed the window mid-query). `None` for
    /// the canned preview wizard (no real DLLs, no background thread).
    tmp: Option<Arc<TempDirGuard>>,
    cur: usize,            // current plugin index
    answers: PluginInputs, // current plugin's answers (committed pages + last shown)
    stack: Vec<Frame>,     // current plugin's path; last = shown page
    finished: InputsByPlugin,
    next_slot: usize,
    next_id: usize, // running control-id allocator (unique across pages)
    /// Preview/test: replay these steps instead of spawning a plugin.
    canned: Option<std::collections::VecDeque<PageStep>>,
}

impl Wizard {
    pub(super) fn new(
        plugins: Vec<(common::model::plugin_entry::PluginEntry, PathBuf)>,
        base_ctx: PluginCtx,
        self_exe: PathBuf,
        tmp: Arc<TempDirGuard>,
    ) -> Self {
        Wizard {
            plugins,
            base_ctx,
            self_exe,
            tmp: Some(tmp),
            cur: 0,
            answers: PluginInputs::new(),
            stack: Vec::new(),
            finished: InputsByPlugin::new(),
            next_slot: 0,
            next_id: ID_PLUGIN_BASE,
            canned: None,
        }
    }

    /// A preview/test wizard that replays `steps` for one synthetic plugin.
    #[cfg(debug_assertions)]
    pub(super) fn canned(steps: Vec<PageStep>) -> Self {
        let mut w = Wizard {
            plugins: Vec::new(),
            base_ctx: PluginCtx::default(),
            self_exe: PathBuf::new(),
            tmp: None,
            cur: 0,
            answers: PluginInputs::new(),
            stack: Vec::new(),
            finished: InputsByPlugin::new(),
            next_slot: 0,
            next_id: ID_PLUGIN_BASE,
            canned: None,
        };
        w.plugins.push((
            common::model::plugin_entry::PluginEntry::default(),
            PathBuf::new(),
        ));
        w.canned = Some(steps.into());
        w
    }

    /// Whether the current page opts in to Back (plugin can disable per page).
    pub(super) fn wants_back(&self) -> bool {
        self.stack.last().map(|f| f.back).unwrap_or(false)
    }
    /// Whether there's a previous plugin page to step back to.
    pub(super) fn can_pop(&self) -> bool {
        self.stack.len() > 1
    }
    pub(super) fn current_slot(&self) -> Option<usize> {
        self.stack.last().map(|f| f.slot)
    }
    pub(super) fn current_title(&self) -> (String, String) {
        match self.stack.last() {
            Some(f) => {
                let sub = if f.notice.is_empty() {
                    f.page.subtitle.clone()
                } else {
                    f.notice.clone()
                };
                (f.page.title.clone(), sub)
            }
            None => (String::new(), String::new()),
        }
    }
    /// Final answers, routed per plugin, for the install worker.
    pub(super) fn inputs(&self) -> InputsByPlugin {
        self.finished.clone()
    }

    /// Advance the canned (preview) wizard by replaying its queued steps through
    /// the same [`advance_steps`] engine the real async path uses, so the preview
    /// can't drift from production behavior. Synchronous (no subprocess), so it
    /// runs straight on the UI thread.
    unsafe fn step_canned(&mut self, hwnd: HWND) -> Step {
        let mut queued = self.canned.take().unwrap_or_default();
        let answers = std::mem::take(&mut self.answers);
        let finished = std::mem::take(&mut self.finished);
        let outcome = advance_steps(&self.plugins, self.cur, answers, finished, |_, _, _| {
            Ok(queued.pop_front().unwrap_or(PageStep::Done))
        });
        self.canned = Some(queued);
        unsafe { self.apply_step_outcome(hwnd, outcome) }
    }

    /// First page of the whole flow (canned preview only — the real wizard
    /// dispatches a background query; see `handlers::dispatch_plugin_query`).
    pub(super) unsafe fn start(&mut self, hwnd: HWND) -> Step {
        unsafe { self.step_canned(hwnd) }
    }

    /// Collect the current page, then replay the next canned step.
    pub(super) unsafe fn forward(&mut self, hwnd: HWND) -> Step {
        if !unsafe { self.collect_page(hwnd) } {
            return Step::Stay; // required field missing (already warned)
        }
        unsafe { self.step_canned(hwnd) }
    }

    /// Step back to the previous page, or signal Exit at the first page.
    pub(super) fn back(&mut self) -> Step {
        if self.stack.len() > 1 {
            self.stack.pop();
            Step::Show
        } else {
            Step::Exit
        }
    }

    pub(super) fn is_canned(&self) -> bool {
        self.canned.is_some()
    }

    /// Collect the current page's answers into `self.answers`. Returns `false`
    /// (after warning the user) when a required field is empty. UI thread only.
    pub(super) unsafe fn collect_page(&mut self, hwnd: HWND) -> bool {
        let Some(slot) = self.current_slot() else {
            return true;
        };
        let Some(vals) = (unsafe { collect_slot(hwnd, slot) }) else {
            return false;
        };
        self.answers.extend(vals);
        true
    }

    /// Extract state for a background `run_step_query` call. `None` when all
    /// plugins are already exhausted.
    pub(super) fn step_args(&self) -> Option<StepArgs> {
        if self.cur >= self.plugins.len() {
            return None;
        }
        Some(StepArgs {
            self_exe: self.self_exe.clone(),
            base_ctx: self.base_ctx.clone(),
            plugins: self.plugins.clone(),
            cur: self.cur,
            answers: self.answers.clone(),
            finished: self.finished.clone(),
            _keepalive: self.tmp.clone(),
            on_progress: None,
        })
    }

    /// Apply a `StepOutcome` from the background thread: update Wizard state
    /// and (for `Page`) build the new page's controls. UI thread only.
    pub(super) unsafe fn apply_step_outcome(&mut self, hwnd: HWND, outcome: StepOutcome) -> Step {
        match outcome {
            StepOutcome::Install(finished) => {
                self.finished = finished;
                Step::Install
            }
            StepOutcome::Page {
                cur,
                answers,
                finished,
                page,
                notice,
                back,
            } => {
                if cur != self.cur {
                    self.stack.clear();
                }
                self.cur = cur;
                self.answers = answers;
                self.finished = finished;
                let auto_run = !page.buttons;
                let marquee = page
                    .widgets
                    .iter()
                    .find_map(|w| {
                        if let PluginWidget::Progress { marquee } = w {
                            Some(*marquee)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(true);
                unsafe { self.push(hwnd, page, notice, back) };
                if auto_run {
                    Step::AutoRun { marquee }
                } else {
                    Step::Show
                }
            }
        }
    }

    /// Build one page's controls (a fresh slot) and make it the current frame.
    unsafe fn push(&mut self, hwnd: HWND, page: PluginPage, notice: String, back: bool) {
        let slot = self.next_slot;
        self.next_slot += 1;
        let hinst = HINSTANCE(unsafe { GetModuleHandleW(PCWSTR::null()).unwrap_or_default() }.0);
        let content_w = WIN_W - PAD * 2;
        let mut y = BANNER_H + PAD + 8;
        let mut fields = Vec::new();
        for widget in &page.widgets {
            let f = unsafe {
                build_widget(
                    hwnd,
                    hinst,
                    slot,
                    &page.id,
                    widget,
                    content_w,
                    &mut y,
                    &mut self.next_id,
                )
            };
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
        self.stack.push(Frame {
            slot,
            page,
            back,
            notice,
        });
        relayout(hwnd, dpi);
        apply_fonts(hwnd);
    }
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

// ---- Background step query ----------------------------------------------

/// Wizard state extracted for the background query thread (all Clone + Send).
pub(super) struct StepArgs {
    self_exe: PathBuf,
    base_ctx: PluginCtx,
    plugins: Vec<(PluginEntry, PathBuf)>,
    pub(super) cur: usize,
    pub(super) answers: PluginInputs,
    pub(super) finished: InputsByPlugin,
    _keepalive: Option<Arc<TempDirGuard>>,
    /// Set by `dispatch_plugin_run` when the page has a deterministic progress bar.
    pub(super) on_progress: Option<Box<dyn Fn(u32) + Send>>,
}

/// Result of a completed background `run_step_query` call.
pub(super) enum StepOutcome {
    /// Every plugin finished — proceed to install with these answers.
    Install(common::plugin::InputsByPlugin),
    /// Show this page (it belongs to plugin `cur`).
    Page {
        cur: usize,
        answers: PluginInputs,
        finished: common::plugin::InputsByPlugin,
        page: PluginPage,
        notice: String,
        back: bool,
    },
}

/// Run on a background thread: call `installway_up` for the current plugin,
/// commit its answers, then continue querying from the next plugin.
pub(super) fn run_plugin_then_step(args: StepArgs) -> StepOutcome {
    let StepArgs {
        self_exe,
        base_ctx,
        plugins,
        cur,
        mut answers,
        mut finished,
        _keepalive,
        on_progress,
    } = args;
    if let Some((entry, dll)) = plugins.get(cur) {
        let inputs_json = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".into());
        if let Err(e) = common::plugin::run_up_single(
            &self_exe,
            &base_ctx,
            entry,
            dll,
            &inputs_json,
            on_progress,
        ) {
            common::log::warn(format!("plugin '{}' up (wizard): {e:#}", entry.name));
        }
        finished
            .entry(entry.name.clone())
            .or_default()
            .extend(std::mem::take(&mut answers));
    }
    let next = cur + 1;
    advance_steps(
        &plugins,
        next,
        answers,
        finished,
        |entry, dll, answers_json| {
            common::plugin::query_step(&self_exe, &base_ctx, entry, dll, answers_json)
        },
    )
}

/// Run on a background thread: advance through plugins (spawning each plugin's
/// step subprocess) until one returns a `Page`, or all are exhausted.
pub(super) fn run_step_query(args: StepArgs) -> StepOutcome {
    // `_keepalive` stays bound until this fn returns, holding the temp dir alive
    // for the whole query (the DLLs are read inside `advance_steps`).
    let StepArgs {
        self_exe,
        base_ctx,
        plugins,
        cur,
        answers,
        finished,
        _keepalive,
        on_progress: _,
    } = args;
    advance_steps(
        &plugins,
        cur,
        answers,
        finished,
        |entry, dll, answers_json| {
            common::plugin::query_step(&self_exe, &base_ctx, entry, dll, answers_json)
        },
    )
}

/// Pure stepping core: walk plugins from `cur`, asking `query` for each one's
/// next step given the answers so far. On `Done`, the current answers are routed
/// into `finished` under the plugin's name and the next plugin is tried; on
/// `Page`, stop and return it. A `query` error is logged and treated as `Done`
/// (the plugin is skipped but its `up` still runs later) so a bad step never
/// blocks the wizard nor drops the remaining plugins. The plugin call is
/// injected so this logic is unit-testable without spawning a subprocess.
fn advance_steps(
    plugins: &[(PluginEntry, PathBuf)],
    mut cur: usize,
    mut answers: PluginInputs,
    mut finished: InputsByPlugin,
    mut query: impl FnMut(&PluginEntry, &std::path::Path, &str) -> anyhow::Result<PageStep>,
) -> StepOutcome {
    loop {
        if cur >= plugins.len() {
            return StepOutcome::Install(finished);
        }
        let (entry, dll) = &plugins[cur];
        let answers_json = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".into());
        let step = match query(entry, dll, &answers_json) {
            Ok(step) => step,
            Err(e) => {
                common::log::warn(format!("plugin '{}' step: {e:#}", entry.name));
                PageStep::Done
            }
        };
        match step {
            PageStep::Done => {
                finished
                    .entry(entry.name.clone())
                    .or_default()
                    .extend(std::mem::take(&mut answers));
                cur += 1;
            }
            PageStep::Page { page, notice, back } => {
                return StepOutcome::Page {
                    cur,
                    answers,
                    finished,
                    page,
                    notice,
                    back,
                };
            }
        }
    }
}

/// Append a STATIC label control for widgets that have a non-empty label.
/// Mutates `rects`, `y`, and `next_id` in place; no-op when `label` is empty.
#[allow(clippy::too_many_arguments)]
unsafe fn build_label_row(
    hwnd: HWND,
    hinst: HINSTANCE,
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
        mk(
            hwnd,
            hinst,
            w!("STATIC"),
            label,
            WINDOW_STYLE(0),
            WINDOW_EX_STYLE(0),
            lid,
            (x, *y, content_w, 20),
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
    hinst: HINSTANCE,
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
                mk(
                    hwnd,
                    hinst,
                    w!("STATIC"),
                    text,
                    WINDOW_STYLE(0),
                    WINDOW_EX_STYLE(0),
                    id,
                    (x, *y, content_w, 20),
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
            unsafe { build_label_row(hwnd, hinst, x, y, content_w, label, next_id, &mut rects) };
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
                let h = mk(
                    hwnd,
                    hinst,
                    w!("EDIT"),
                    default,
                    style,
                    WS_EX_CLIENTEDGE,
                    eid,
                    (x, *y, content_w, h_px),
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
                let h = mk(
                    hwnd,
                    hinst,
                    w!("BUTTON"),
                    label,
                    WINDOW_STYLE(BS_AUTOCHECKBOX) | WS_TABSTOP,
                    WINDOW_EX_STYLE(0),
                    cid,
                    (x, *y, content_w, 22),
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
            unsafe { build_label_row(hwnd, hinst, x, y, content_w, label, next_id, &mut rects) };
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
                        let h = mk(
                            hwnd,
                            hinst,
                            w!("COMBOBOX"),
                            "",
                            WINDOW_STYLE(CBS_DROPDOWNLIST) | WS_TABSTOP | WS_VSCROLL_S,
                            WINDOW_EX_STYLE(0),
                            cid,
                            (x, *y, content_w, 200),
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
                            let h = mk(
                                hwnd,
                                hinst,
                                w!("BUTTON"),
                                &o.label,
                                style,
                                WINDOW_EX_STYLE(0),
                                rid,
                                (x + 16, *y, content_w - 16, 22),
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
            unsafe { build_label_row(hwnd, hinst, x, y, content_w, label, next_id, &mut rects) };
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
                    let h = mk(
                        hwnd,
                        hinst,
                        w!("BUTTON"),
                        &o.label,
                        WINDOW_STYLE(BS_AUTOCHECKBOX) | WS_TABSTOP,
                        WINDOW_EX_STYLE(0),
                        cid,
                        (x + 16, *y, content_w - 16, 22),
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
                mk(
                    hwnd,
                    hinst,
                    w!("msctls_progress32"),
                    "",
                    WINDOW_STYLE(0),
                    WINDOW_EX_STYLE(0),
                    id,
                    (x, *y, content_w, 20),
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

/// Create one hidden child control. The window text is copied by Win32, so the
/// wide buffer need not outlive this call.
#[allow(clippy::too_many_arguments)]
unsafe fn mk(
    hwnd: HWND,
    hinst: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex: WINDOW_EX_STYLE,
    id: usize,
    r: (i32, i32, i32, i32),
) -> HWND {
    let t = wide(text);
    unsafe {
        CreateWindowExW(
            ex,
            class,
            PCWSTR(t.as_ptr()),
            WS_CHILD | WS_CLIPSIBLINGS | style,
            r.0,
            r.1,
            r.2,
            r.3,
            Some(hwnd),
            Some(HMENU(id as *mut _)),
            Some(hinst),
            None,
        )
        .unwrap_or_default()
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
    let s = |v: i32| helpers::scale(v, dpi);
    with_state(|state| unsafe {
        for f in &state.plugin_fields {
            for &(id, x, y, w, h) in &f.rects {
                let ctrl = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
                if !ctrl.is_invalid() {
                    let _ = MoveWindow(ctrl, s(x), s(y), s(w), s(h), true);
                }
            }
        }
    });
}

/// Apply the normal font to every plugin control (mirrors `views::apply_fonts`).
pub(super) fn apply_fonts(hwnd: HWND) {
    with_state(|state| {
        let font = state.font_normal;
        unsafe {
            for f in &state.plugin_fields {
                for &(id, ..) in &f.rects {
                    helpers::set_font(hwnd, id, font);
                }
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
    unsafe {
        helpers::set_dlg_text(hwnd, ID_HEADER, &title);
        helpers::set_dlg_text(hwnd, ID_SUBHEADER, &subtitle);
    }
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

#[cfg(test)]
mod tests {
    use super::{StepOutcome, advance_steps};
    use common::model::page_step::PageStep;
    use common::model::plugin_entry::PluginEntry;
    use common::model::plugin_page::PluginInputs;
    use common::model::plugin_page::PluginPage;
    use common::plugin::InputsByPlugin;
    use std::path::{Path, PathBuf};

    fn plugins(names: &[&str]) -> Vec<(PluginEntry, PathBuf)> {
        names
            .iter()
            .map(|n| {
                (
                    PluginEntry {
                        name: (*n).into(),
                        ..Default::default()
                    },
                    PathBuf::new(),
                )
            })
            .collect()
    }

    fn page(id: &str) -> PageStep {
        PageStep::Page {
            page: PluginPage {
                id: id.into(),
                title: String::new(),
                subtitle: String::new(),
                widgets: vec![],
                buttons: true,
            },
            notice: String::new(),
            back: true,
        }
    }

    fn answers(pairs: &[(&str, &str)]) -> PluginInputs {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    /// Every plugin returning `Done` exhausts the list and proceeds to install,
    /// recording an entry for each plugin (even with no answers).
    #[test]
    fn advance_all_done_installs() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(PageStep::Done),
        );
        match out {
            StepOutcome::Install(f) => {
                assert!(f.contains_key("a"));
                assert!(f.contains_key("b"));
            }
            _ => panic!("expected install"),
        }
    }

    /// The first plugin to return a `Page` stops the walk at its index.
    #[test]
    fn advance_stops_at_first_page() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "a" {
                    Ok(page("p"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Page { cur, .. } => assert_eq!(cur, 0),
            _ => panic!("expected page"),
        }
    }

    /// When a plugin finishes, its collected answers are routed under its name and
    /// the carried `answers` is reset for the next plugin.
    #[test]
    fn advance_routes_answers_to_finishing_plugin() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            answers(&[("region.country", "FR")]),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "b" {
                    Ok(page("p"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Page {
                cur,
                answers,
                finished,
                ..
            } => {
                assert_eq!(cur, 1);
                assert!(answers.is_empty());
                assert_eq!(finished["a"]["region.country"], "FR");
            }
            _ => panic!("expected page"),
        }
    }

    /// A step-query error is non-fatal: the plugin is finalized (its answers kept
    /// for `up`) and the walk continues to the remaining plugins.
    #[test]
    fn advance_skips_plugin_on_error() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            answers(&[("a.k", "v")]),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "a" {
                    Err(anyhow::anyhow!("boom"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Install(f) => {
                assert_eq!(f["a"]["a.k"], "v");
                assert!(f.contains_key("b"));
            }
            _ => panic!("expected install"),
        }
    }

    /// The answers collected so far are serialized into the JSON handed to each
    /// step query (so a plugin can branch on a prior page's answer).
    #[test]
    fn advance_passes_answers_json_to_query() {
        let pl = plugins(&["a"]);
        let mut seen = String::new();
        advance_steps(
            &pl,
            0,
            answers(&[("region.country", "DOM")]),
            InputsByPlugin::new(),
            |_, _: &Path, json| {
                seen = json.to_string();
                Ok(PageStep::Done)
            },
        );
        assert!(seen.contains("region.country"));
        assert!(seen.contains("DOM"));
    }

    /// State threads across successive calls (one per Next click): a plugin shows
    /// a page, then on the follow-up call returns Done with the page's answers
    /// routed under its name.
    #[test]
    fn advance_threads_state_across_calls() {
        let pl = plugins(&["a"]);
        let out1 = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(page("page1")),
        );
        let (cur, mut answers, finished) = match out1 {
            StepOutcome::Page {
                cur,
                answers,
                finished,
                page,
                ..
            } => {
                assert_eq!(page.id, "page1");
                (cur, answers, finished)
            }
            _ => panic!("expected page1"),
        };
        // The user fills page1; the handler would collect these before re-querying.
        answers.insert("page1.x".into(), "1".into());
        let out2 = advance_steps(&pl, cur, answers, finished, |_, _: &Path, _| {
            Ok(PageStep::Done)
        });
        match out2 {
            StepOutcome::Install(f) => assert_eq!(f["a"]["page1.x"], "1"),
            _ => panic!("expected install"),
        }
    }

    /// An empty plugin list installs immediately with no recorded answers.
    #[test]
    fn advance_empty_plugins_installs() {
        let pl: Vec<(PluginEntry, PathBuf)> = Vec::new();
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(PageStep::Done),
        );
        assert!(matches!(out, StepOutcome::Install(f) if f.is_empty()));
    }
}
