// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Uninstaller UI facade. WinUI is preferred when its runtime is available;
//! the existing Win32 window remains the compatibility fallback.

mod progress;
mod win32;
mod winui;

use common::utils::wide;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW,
};
use windows::core::PCWSTR;

thread_local! {
    static T: RefCell<common::i18n::Translator> =
        RefCell::new(common::i18n::Translator::for_lang(common::i18n::current_lang()));
}

pub fn set_translator(translator: common::i18n::Translator) {
    T.with(|slot| *slot.borrow_mut() = translator);
}

pub fn tr() -> common::i18n::Translator {
    T.with(|slot| *slot.borrow())
}

#[cfg(feature = "hintway")]
pub fn backend_name() -> &'static str {
    if winui_support::available() {
        "winui"
    } else {
        "win32"
    }
}

#[derive(Clone)]
pub struct UninstallParams {
    pub title: String,
    pub subtitle: String,
    pub confirm_text: String,
    /// Worker invoked after confirmation; must publish progress and finish.
    pub worker: Worker,
    /// Start immediately once the UI loop is running (finalize stage).
    pub auto_start: bool,
}

/// Prefer WinUI and fall back to Win32 when it cannot start before the worker
/// is consumed.
pub fn run(params: UninstallParams) -> bool {
    if winui_support::available() {
        match winui::run(params.clone()) {
            Ok(result) => return result,
            Err(error) if params.worker.is_pending() => {
                common::log::warn(format!(
                    "WinUI startup failed ({error:#}); using the Win32 UI"
                ));
            }
            Err(error) => {
                common::log::error(format!("WinUI failed after uninstall started: {error:#}"));
                fatal(&format!("{error:#}"));
                return true;
            }
        }
    }
    win32::run(params)
}

pub fn fatal(msg: &str) {
    let text = wide(msg);
    let caption = wide(&tr().get("uninstall.fatal_caption"));
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// Modal info dialog used for the uninstall-complete confirmation.
pub fn info(msg: &str, caption: &str) {
    let text = wide(msg);
    let caption = wide(caption);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

pub type Progress = common::ProgressFn;

type WorkerFn = Box<dyn FnOnce(Progress) + Send>;

/// Cloneable handle to one uninstall worker. A retained clone allows fallback
/// when WinUI fails before starting the operation.
#[derive(Clone)]
pub struct Worker(std::sync::Arc<std::sync::Mutex<Option<WorkerFn>>>);

impl Worker {
    pub fn new(worker: impl FnOnce(Progress) + Send + 'static) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(
            worker,
        )))))
    }

    pub fn run(self, progress: Progress) -> anyhow::Result<()> {
        let worker = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("uninstall worker lock poisoned"))?
            .take()
            .ok_or_else(|| anyhow::anyhow!("uninstall worker already started"))?;
        worker(progress);
        Ok(())
    }

    fn is_pending(&self) -> bool {
        self.0
            .lock()
            .map(|worker| worker.is_some())
            .unwrap_or(false)
    }
}

/// Step-based adapter used by the finalize stage.
pub struct StepCounter {
    pub done: AtomicU64,
    pub total: u64,
    pub cb: Progress,
}

impl StepCounter {
    pub fn new(total: u64, cb: Progress) -> Self {
        Self {
            done: AtomicU64::new(0),
            total,
            cb,
        }
    }

    pub fn step(&self, label: &str) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        (self.cb)(done, self.total, label);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn worker_thread_inherits_the_process_language() {
        common::i18n::Translator::for_lang("fr").set_global();
        assert_eq!(common::i18n::current_lang(), "fr");

        let lang = std::thread::spawn(|| super::tr().lang()).join().unwrap();
        assert_eq!(lang, "fr");
    }
}
