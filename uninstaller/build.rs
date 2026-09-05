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

    // Keep Reactor's post-1703 bootstrap imports out of the normal load path,
    // so unsupported systems can reach the Win32 fallback.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:api-ms-win-appmodel-runtime-l1-1-5.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
}
