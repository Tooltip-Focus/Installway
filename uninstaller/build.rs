// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let m = new_manifest("Installway.Uninstaller")
            .requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(m).expect("embed uninstaller manifest");
    }
}
