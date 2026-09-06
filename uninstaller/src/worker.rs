// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::Result;
use common::elevation::{WorkerEvent, send};
use common::model::manifest::Manifest;
use std::fs::File;

pub fn run(pipe_name: &str) -> Result<()> {
    common::i18n::Translator::detect(&[]).set_global();

    let handle = common::elevation::connect_pipe_client(pipe_name)?;
    let mut pipe = common::elevation::open_pipe_handle(handle);

    match uninstall(&mut pipe) {
        Ok(()) => {
            let _ = send(&mut pipe, &WorkerEvent::Done);
        }
        // A failure is terminal for the worker: the parent surfaces the message
        // and no `Done` is sent. The process still exits 0 - the parent learns
        // about the failure from the event, not from the exit code.
        Err(e) => {
            let _ = send(
                &mut pipe,
                &WorkerEvent::Error {
                    msg: format!("{e:#}"),
                },
            );
        }
    }
    Ok(())
}

/// The uninstall itself, streaming progress over `pipe` as it runs. The first
/// failure aborts; the caller turns it into a `WorkerEvent::Error`.
fn uninstall(pipe: &mut File) -> Result<()> {
    let data_dir = crate::cleanup::self_dir()?;
    let info = crate::cleanup::read_info(&data_dir)?;

    // Manifest may be missing (partial delete); fall back to an empty one.
    let manifest = crate::cleanup::read_manifest(&data_dir)
        .unwrap_or_else(|_| Manifest::fallback(&info.version, info.exe.as_deref()));

    let app_dir = std::path::PathBuf::from(&info.install_dir);

    crate::stages::uninstall::do_cleanup(
        &info,
        &manifest,
        &app_dir,
        &data_dir,
        |done, total, name| {
            let _ = send(
                pipe,
                &WorkerEvent::Progress {
                    done,
                    total,
                    name: name.to_string(),
                },
            );
        },
    );

    crate::stages::uninstall::spawn_finalize(
        Some(&app_dir),
        &data_dir,
        Some(&info.product),
        info.show_uninstall_complete,
    )
}
