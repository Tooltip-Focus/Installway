// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Render model and cross-thread mailboxes for the WinUI wizard.
//!
//! The render function is a pure function of [`Model`], so everything it draws
//! is `Clone + PartialEq` data. Build-time configuration lives in the
//! thread-locals below instead, and values that are neither `PartialEq` nor
//! `Send` (the [`Wizard`](super::wizard_state::Wizard), a `PluginPage`, the
//! collected inputs) stay in [`WIZARD`] or the mailboxes at the bottom, with the
//! model carrying only a counter or tag.

use common::model::install_dir_restriction::InstallDirRestriction;
use common::model::installer_payload::InstallerPayload;
use common::model::launch_option::LaunchOption;
use common::model::plugin_page::PluginInputs;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
pub(super) use winui_support::{AsyncValue, Progress};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Phase {
    License,
    Choose,
    Plugin,
    Progress,
    Done,
    Error,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Dialog {
    Warn(String),
    ConfirmCancel,
}

/// A one-shot edge from a background thread. Heavier payloads are parked in the
/// mailboxes below and picked up on the UI thread.
///
/// The trailing `u64` preserves the ordering of repeated plugin and close
/// events published between component polls.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(super) enum Signal {
    #[default]
    None,
    Done,
    Error(String),
    /// Needs elevation; [`PERM_ERROR`] holds the retry payload.
    PermError,
    /// UAC declined, or the elevated worker failed to start.
    PermDenied(String),
    /// Cancel rollback finished; close cleanly.
    Cancelled,
    /// Step query finished; [`QUERIED_STEP`] holds it.
    PluginStep(u64),
    PluginProgress(u32, u64),
    /// The close guard swallowed a `WM_CLOSE` and wants a confirmation.
    CloseRequested(u64),
}

/// Lets the `WM_CLOSE` guard raise a signal; it runs in a window subclass with
/// no `SetState` in hand. Published by the first render.
pub(super) static SIGNAL_SINK: Mutex<Option<AsyncValue<Signal>>> = Mutex::new(None);

pub(super) fn raise(signal: Signal) {
    if let Ok(guard) = SIGNAL_SINK.lock()
        && let Some(sink) = guard.as_ref()
    {
        sink.call(signal);
    }
}

#[derive(Clone, PartialEq, Debug)]
pub(super) struct Model {
    pub phase: Phase,
    pub license_accepted: bool,
    pub path: String,
    pub launch: bool,
    pub error_text: String,
    pub progress: Progress,
    /// Bumped when the current plugin page changes, so the render function
    /// re-reads the descriptor that cannot live in the model.
    pub page_epoch: u64,
    /// Live answers, keyed `"<page_id>.<widget_id>"`.
    pub answers: PluginInputs,
    /// A background query is running; nav is disabled.
    pub busy: bool,
    pub auto_run: bool,
    pub auto_marquee: bool,
    pub plugin_progress: u32,
    pub dialog: Option<Dialog>,
    pub cancelling: bool,
}

impl Model {
    pub(super) fn new(default_path: &std::path::Path) -> Self {
        Model {
            phase: Phase::License,
            license_accepted: false,
            path: default_path.to_string_lossy().into_owned(),
            launch: false,
            error_text: String::new(),
            progress: Progress::default(),
            page_epoch: 0,
            answers: PluginInputs::new(),
            busy: false,
            auto_run: false,
            auto_marquee: true,
            plugin_progress: 0,
            dialog: None,
            cancelling: false,
        }
    }
}

thread_local! {
    pub(super) static PAYLOAD: RefCell<Option<InstallerPayload>> = const { RefCell::new(None) };
    /// Staged banner PNG as a `file:///` URI; `None` keeps the plain header.
    pub(super) static BANNER_URI: RefCell<Option<String>> = const { RefCell::new(None) };
    pub(super) static LAUNCH_FLAG: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_LICENSE: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static SKIP_PATH: RefCell<bool> = const { RefCell::new(false) };
    pub(super) static RESTRICTION: RefCell<InstallDirRestriction> =
        const { RefCell::new(InstallDirRestriction::Enforce) };
    pub(super) static DEFAULT_PATH: RefCell<String> = const { RefCell::new(String::new()) };
    pub(super) static WIZARD: RefCell<Option<super::wizard_state::Wizard>> =
        const { RefCell::new(None) };
    static T: RefCell<common::i18n::Translator> = RefCell::new(common::i18n::Translator::default());
}

pub(super) fn tr() -> common::i18n::Translator {
    T.with(|t| *t.borrow())
}

pub(super) fn set_translator(t: common::i18n::Translator) {
    T.with(|slot| *slot.borrow_mut() = t);
}

pub(super) fn skip_license() -> bool {
    SKIP_LICENSE.with(|s| *s.borrow())
}

pub(super) fn skip_path() -> bool {
    SKIP_PATH.with(|s| *s.borrow())
}

pub(super) fn restriction() -> InstallDirRestriction {
    RESTRICTION.with(|r| *r.borrow())
}

pub(super) fn default_path() -> String {
    DEFAULT_PATH.with(|d| d.borrow().clone())
}

pub(super) fn launch_option() -> LaunchOption {
    PAYLOAD.with(|p| {
        p.borrow()
            .as_ref()
            .map(|p| p.launch_option)
            .unwrap_or_default()
    })
}

pub(super) fn has_plugin_pages() -> bool {
    WIZARD.with(|w| w.borrow().is_some())
}

pub(super) fn with_payload<T: Default>(f: impl FnOnce(&InstallerPayload) -> T) -> T {
    PAYLOAD.with(|p| p.borrow().as_ref().map(f).unwrap_or_default())
}

// Hand-off slots for values that cannot ride a `Signal`. A background thread
// parks the value, posts the signal, and the UI thread takes it.

pub(super) static QUERIED_STEP: Mutex<Option<crate::ui::wizard_engine::StepOutcome>> =
    Mutex::new(None);

pub(super) static PERM_ERROR: Mutex<Option<PermError>> = Mutex::new(None);

pub(super) struct PermError {
    pub path: PathBuf,
    pub plugin_inputs: common::plugin::InputsByPlugin,
}

/// Cancel flag handed to the install worker; set once per run.
pub(super) static CANCEL: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>> = Mutex::new(None);

pub(super) fn request_cancel() {
    if let Ok(guard) = CANCEL.lock()
        && let Some(flag) = guard.as_ref()
    {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
