// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Background work: the install pipeline, the elevated retry, and plugin-step
//! queries. Nothing here runs on the UI thread. Where the Win32 backend posts
//! `WM_APP_*`, each thread here pushes a [`Signal`] through an
//! [`AsyncSetState`], which marshals the write and the re-render for us.

use super::model::{
    CANCEL, PERM_ERROR, PermError, Progress, QUERIED_STEP, Signal, tr, with_payload,
};
use crate::extract::{InstallCtx, install};
use crate::install as install_mod;
use crate::ui::wizard_engine::{StepArgs, run_plugin_then_step, run_step_query};
use common::plugin::InputsByPlugin;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows_reactor::AsyncSetState;

/// The primary button stays enabled until the outcome lands, so a double-click
/// would otherwise race two threads on [`QUERIED_STEP`].
static QUERY_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The setters a background thread needs to talk back to the UI.
#[derive(Clone)]
pub(super) struct Feed {
    pub signal: AsyncSetState<Signal>,
    pub progress: AsyncSetState<Progress>,
}

pub(super) fn start_install(feed: Feed, path: PathBuf, plugin_inputs: InputsByPlugin) {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut slot) = CANCEL.lock() {
        *slot = Some(cancel.clone());
    }
    let translator = tr();

    thread::spawn(move || {
        let mut loaded = match crate::payload::load_and_verify() {
            Ok(l) => l,
            Err(e) => {
                #[cfg(feature = "hintway")]
                crate::analytics::error(crate::analytics::classify_error(&e));
                feed.signal.call(Signal::Error(format!("{e}")));
                return;
            }
        };
        // Machine-wide iff the target is shared (e.g. Program Files); catches an
        // already-admin run that never trips the PermissionDenied path.
        let requires_admin = common::paths::is_machine_location(&path);
        crate::extract::resolve_and_filter(&mut loaded, &path, requires_admin, &plugin_inputs);

        let progress_cb: common::ProgressFn = {
            let set = feed.progress.clone();
            Arc::new(move |done, total, name: &str| {
                set.call(Progress {
                    done,
                    total,
                    name: name.to_string(),
                });
            })
        };
        let ctx = InstallCtx {
            install_dir: path.clone(),
            payload: &loaded.payload,
            zip_bytes: loaded.zip(),
            cancel: cancel.clone(),
            on_progress: progress_cb,
            plugin_inputs: plugin_inputs.clone(),
            requires_admin,
            hwnd_parent: super::active_hwnd(),
            translator,
        };
        #[cfg(feature = "hintway")]
        crate::analytics::stage("extract");
        // Lock held across finalize so a concurrent run can't interleave.
        let _install_lock = match install(ctx) {
            Ok(lock) => lock,
            Err(e) => {
                // A confirmed cancel rolled back; close cleanly rather than
                // reporting it as an install failure.
                if cancel.load(Ordering::Relaxed) {
                    common::log::info("install cancelled by user");
                    feed.signal.call(Signal::Cancelled);
                    return;
                }
                if e.downcast_ref::<crate::extract::PermissionDeniedError>()
                    .is_some()
                {
                    #[cfg(feature = "hintway")]
                    crate::analytics::error("permission_denied");
                    if let Ok(mut slot) = PERM_ERROR.lock() {
                        *slot = Some(PermError {
                            path: path.clone(),
                            plugin_inputs: plugin_inputs.clone(),
                        });
                    }
                    feed.signal.call(Signal::PermError);
                } else {
                    #[cfg(feature = "hintway")]
                    crate::analytics::error(crate::analytics::classify_error(&e));
                    feed.signal.call(Signal::Error(format!("{e}")));
                }
                return;
            }
        };
        #[cfg(feature = "hintway")]
        crate::analytics::stage("finalize");
        if let Err(e) = install_mod::finalize(
            &path,
            &loaded.payload,
            &loaded.uninstaller_bytes,
            loaded.zip(),
            &plugin_inputs,
            requires_admin,
        ) {
            #[cfg(feature = "hintway")]
            crate::analytics::error(crate::analytics::classify_error(&e));
            feed.signal.call(Signal::Error(format!("finalize: {e}")));
            return;
        }
        #[cfg(feature = "hintway")]
        crate::analytics::stage("done");
        feed.signal.call(Signal::Done);
    });
}

/// Retry through an elevated worker. The UAC prompt is the user's yes/no, so
/// this fires straight away without asking first.
pub(super) fn start_elevated_install(feed: Feed, payload: PermError) {
    thread::spawn(move || {
        let progress = feed.progress.clone();
        let result = crate::elevation::run_elevated_install(
            &payload.path,
            &payload.plugin_inputs,
            move |done, total, name: &str| {
                progress.call(Progress {
                    done,
                    total,
                    name: name.to_string(),
                });
            },
        );
        match result {
            Ok(()) => {
                #[cfg(feature = "hintway")]
                crate::analytics::stage("done");
                feed.signal.call(Signal::Done);
            }
            Err(e) if e.is::<crate::elevation::UacCancelledError>() => {
                #[cfg(feature = "hintway")]
                crate::analytics::error("elevation_cancelled");
                feed.signal.call(Signal::PermDenied(
                    payload.path.to_string_lossy().into_owned(),
                ));
            }
            Err(e) => {
                #[cfg(feature = "hintway")]
                crate::analytics::error(crate::analytics::classify_error(&e));
                feed.signal.call(Signal::Error(format!("{e:#}")));
            }
        }
    });
}

/// `false` when a query is already in flight, so the caller leaves the model be.
pub(super) fn dispatch_step(feed: Feed, args: StepArgs, seq: u64) -> bool {
    if QUERY_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return false;
    }
    thread::spawn(move || {
        let outcome = run_step_query(args);
        park_and_signal(&feed, outcome, seq);
    });
    true
}

/// Run the current plugin's `installway_up`, then query the next step. Used by
/// auto-run pages; with `!marquee` its `emit_progress` calls are relayed.
pub(super) fn dispatch_run(feed: Feed, mut args: StepArgs, seq: u64, marquee: bool) -> bool {
    if QUERY_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return false;
    }
    if !marquee {
        let signal = feed.signal.clone();
        let progress_seq = Mutex::new(0u64);
        args.on_progress = Some(Box::new(move |v| {
            // Two identical percentages in a row must still wake the UI.
            let mut n = progress_seq.lock().unwrap();
            *n += 1;
            signal.call(Signal::PluginProgress(v, *n));
        }));
    }
    thread::spawn(move || {
        let outcome = run_plugin_then_step(args);
        park_and_signal(&feed, outcome, seq);
    });
    true
}

fn park_and_signal(feed: &Feed, outcome: crate::ui::wizard_engine::StepOutcome, seq: u64) {
    if let Ok(mut slot) = QUERIED_STEP.lock() {
        *slot = Some(outcome);
    }
    QUERY_IN_FLIGHT.store(false, Ordering::SeqCst);
    feed.signal.call(Signal::PluginStep(seq));
}

pub(super) fn launch_product(path: &std::path::Path) {
    let exe = with_payload(|p| p.manifest.exe.clone());
    let _ = crate::install::launch_product(path, exe.as_deref());
}
