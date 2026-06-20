// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::Result;
use common::elevation::{WorkerEvent, send};
use common::model::manifest::Manifest;

pub fn run(pipe_name: &str) -> Result<()> {
    let handle = common::elevation::connect_pipe_client(pipe_name)?;
    let mut pipe = common::elevation::open_pipe_handle(handle);

    let data_dir = match crate::cleanup::self_dir() {
        Ok(d) => d,
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

    let info = match crate::cleanup::read_info(&data_dir) {
        Ok(i) => i,
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

    let manifest = crate::cleanup::read_manifest(&data_dir).unwrap_or_else(|_| Manifest {
        version: info.version.clone(),
        exe: info.exe.clone(),
        files: Default::default(),
        deleted_files: Vec::new(),
        full_size: 0,
        total_patch_size: 0,
    });

    let app_dir = std::path::PathBuf::from(&info.install_dir);

    crate::stages::uninstall::do_cleanup(
        &info,
        &manifest,
        &app_dir,
        &data_dir,
        |done, total, name| {
            let _ = send(
                &mut pipe,
                &WorkerEvent::Progress {
                    done,
                    total,
                    name: name.to_string(),
                },
            );
        },
    );

    if let Err(e) = crate::stages::uninstall::spawn_finalize(
        Some(&app_dir),
        &data_dir,
        Some(&info.product),
        info.show_uninstall_complete,
    ) {
        let _ = send(
            &mut pipe,
            &WorkerEvent::Error {
                msg: format!("{e:#}"),
            },
        );
        return Ok(());
    }

    let _ = send(&mut pipe, &WorkerEvent::Done);
    Ok(())
}
