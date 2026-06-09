// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let m =
            new_manifest("Installway.Builder").requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(m).expect("embed builder manifest");
    }

    if let Ok(out_dir) = env::var("OUT_DIR") {
        let out_path = PathBuf::from(out_dir);
        if let Some(target_dir) = out_path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let hdiffz_src = PathBuf::from("../vendor/hdiffpatch/hdiffz.exe");
            let hdiffz_dst = target_dir.join("hdiffz.exe");
            if hdiffz_src.exists() {
                let _ = fs::copy(&hdiffz_src, &hdiffz_dst);
            }
        }
    }
}
