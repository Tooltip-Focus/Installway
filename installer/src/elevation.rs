// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! UAC elevation for installs to admin-only locations. Two sides of the same
//! named-pipe protocol: [`run_elevated_install`] runs in the main UI process
//! (spawns the elevated worker, relays progress), [`run_as_worker`] runs in that
//! `--elevated-worker` subprocess (does the install + finalize).

use anyhow::Result;
use common::elevation::{InstallWorkerCommand, WorkerEvent, send};
use std::sync::{Arc, Mutex};

/// Runs the elevated worker subprocess side: connect to the main-process pipe,
/// read the install command, perform install + finalize, stream events back.
pub fn run_as_worker(pipe_name: &str) -> Result<()> {
    let handle = common::elevation::connect_pipe_client(pipe_name)?;
    let mut pipe = common::elevation::open_pipe_handle(handle);

    let translator = common::i18n::Translator::detect(&[]);
    translator.set_global();

    let line = common::elevation::read_line_unbuffered(&mut pipe)?;
    let cmd: InstallWorkerCommand = serde_json::from_str(line.trim())?;

    let mut loaded = match crate::payload::load_and_verify() {
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

    // Resolve feature packs (machine-wide install) from the inputs the main
    // process forwarded, and filter the manifest before staging.
    crate::extract::resolve_and_filter(&mut loaded, &cmd.install_dir, true, &cmd.plugin_inputs);

    // Arc<Mutex> because rayon may call the progress callback from multiple threads.
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
        // This worker only runs after the main process needed elevation, i.e. a
        // machine-wide install.
        requires_admin: true,
        hwnd_parent: 0, // elevated subprocess: no dialog, force-kill silently
        translator,
    };

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

    if let Ok(mut p) = pipe_shared.lock() {
        let _ = send(&mut *p, &WorkerEvent::Done);
    }
    Ok(())
}

/// Returned when the user declines the UAC prompt. Callers can distinguish
/// this from pipe/install failures via `e.is::<UacCancelledError>()`.
#[derive(Debug)]
pub struct UacCancelledError;

impl std::fmt::Display for UacCancelledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UAC elevation was cancelled")
    }
}

impl std::error::Error for UacCancelledError {}

/// Main-process orchestrator: create pipe, trigger UAC, relay WorkerEvent lines
/// via `on_progress`. Returns `Ok(())` on Done, `Err(UacCancelledError)` if
/// the user declines, or a plain anyhow error for other failures.
pub fn run_elevated_install(
    install_dir: &std::path::Path,
    plugin_inputs: &common::plugin::InputsByPlugin,
    mut on_progress: impl FnMut(u64, u64, &str),
) -> anyhow::Result<()> {
    use common::elevation::recv;
    use windows::Win32::Foundation::CloseHandle;

    let pipe_name = common::elevation::pipe_name(std::process::id());
    let pipe_handle = common::elevation::create_pipe_server(&pipe_name)?;

    if common::elevation::spawn_elevated_worker(&pipe_name).is_err() {
        unsafe {
            let _ = CloseHandle(pipe_handle);
        }
        return Err(anyhow::Error::new(UacCancelledError));
    }

    common::elevation::wait_for_client(pipe_handle)?;
    let mut pipe = common::elevation::open_pipe_handle(pipe_handle);

    send(
        &mut pipe,
        &InstallWorkerCommand {
            install_dir: install_dir.to_path_buf(),
            plugin_inputs: plugin_inputs.clone(),
        },
    )?;

    let mut reader = common::elevation::event_reader(pipe);
    loop {
        match recv::<WorkerEvent, _>(&mut reader) {
            Ok(Some(WorkerEvent::Progress { done, total, name })) => {
                on_progress(done, total, &name)
            }
            Ok(Some(WorkerEvent::Done)) => return Ok(()),
            Ok(Some(WorkerEvent::Error { msg })) => anyhow::bail!("{msg}"),
            Ok(None) => anyhow::bail!("elevated worker exited unexpectedly"),
            Err(e) => return Err(e.context("pipe read error")),
        }
    }
}
