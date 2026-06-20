// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Elevated-worker mode for the installer.
//!
//! When the installer detects `ERROR_ACCESS_DENIED` on the target directory
//! it spawns itself as `--elevated-worker <pipe>` via UAC. This module runs
//! that worker: it connects to the named pipe, reads the install command,
//! performs the full install + finalize, and streams `WorkerEvent` lines back.

use anyhow::Result;
use common::elevation::{InstallWorkerCommand, WorkerEvent, send};
use std::sync::{Arc, Mutex};

pub fn run_as_worker(pipe_name: &str) -> Result<()> {
    // ── connect to the main-process pipe ─────────────────────────────────
    let handle = common::elevation::connect_pipe_client(pipe_name)?;
    let mut pipe = common::elevation::open_pipe_handle(handle);

    // ── read the install command (single JSON line) ───────────────────────
    let line = common::elevation::read_line_unbuffered(&mut pipe)?;
    let cmd: InstallWorkerCommand = serde_json::from_str(line.trim())?;

    // ── load the embedded payload ─────────────────────────────────────────
    let loaded = match crate::payload::load_and_verify() {
        Ok(l) => l,
        Err(e) => {
            let _ = send(
                &mut pipe,
                &WorkerEvent::Error {
                    msg: format!("{e:#}"),
                },
            );
            return Ok(());
        }
    };

    // ── progress callback → WorkerEvent::Progress via pipe ───────────────
    // Wrapped in Arc<Mutex> because rayon may call it from multiple threads.
    let pipe_shared: Arc<Mutex<std::fs::File>> = Arc::new(Mutex::new(pipe));
    let progress_fn: common::ProgressFn = {
        let ps = pipe_shared.clone();
        Arc::new(move |done, total, name| {
            if let Ok(mut p) = ps.lock() {
                let _ = send(
                    &mut *p,
                    &WorkerEvent::Progress {
                        done,
                        total,
                        name: name.to_string(),
                    },
                );
            }
        })
    };

    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx = crate::extract::InstallCtx {
        install_dir: cmd.install_dir.clone(),
        payload: &loaded.payload,
        zip_bytes: loaded.zip(),
        cancel,
        on_progress: progress_fn,
        plugin_inputs: cmd.plugin_inputs.clone(),
    };

    // ── run install ───────────────────────────────────────────────────────
    if let Err(e) = crate::extract::install(ctx) {
        if let Ok(mut p) = pipe_shared.lock() {
            let _ = send(
                &mut *p,
                &WorkerEvent::Error {
                    msg: format!("{e:#}"),
                },
            );
        }
        return Ok(());
    }

    // ── run finalize (requires_admin = true) ──────────────────────────────
    if let Err(e) = crate::install::finalize(
        &cmd.install_dir,
        &loaded.payload,
        &loaded.uninstaller_bytes,
        loaded.zip(),
        &cmd.plugin_inputs,
        true,
    ) {
        if let Ok(mut p) = pipe_shared.lock() {
            let _ = send(
                &mut *p,
                &WorkerEvent::Error {
                    msg: format!("finalize: {e:#}"),
                },
            );
        }
        return Ok(());
    }

    // ── done ─────────────────────────────────────────────────────────────
    if let Ok(mut p) = pipe_shared.lock() {
        let _ = send(&mut *p, &WorkerEvent::Done);
    }
    Ok(())
}
