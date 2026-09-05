// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=INSTALLER_PUB_KEY");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let m = new_manifest("Installway.Installer")
            .requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(m).expect("embed installer manifest");
    }

    stage_winui_runtime();

    let pub_key_hex = env::var("INSTALLER_PUB_KEY").unwrap_or_else(|_| {
        // Zero key = dev mode. Stub refuses to verify any payload at runtime.
        eprintln!("warning: INSTALLER_PUB_KEY not set - building dev stub (rejects all payloads)");
        "0".repeat(64)
    });

    let bytes = match hex::decode_lower(&pub_key_hex) {
        Some(b) if b.len() == 32 => b,
        _ => panic!("INSTALLER_PUB_KEY must be 64 lowercase hex chars (32 bytes)"),
    };

    let arr: String = bytes
        .iter()
        .map(|b| format!("0x{:02x}", b))
        .collect::<Vec<_>>()
        .join(", ");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("pub_key.rs");
    let body = format!("pub const PUB_KEY: [u8; 32] = [{}];\n", arr);
    fs::write(&dest, body).expect("write pub_key.rs");
}

fn stage_winui_runtime() {
    // Reactor's inline bootstrap imports Dynamic Dependencies APIs introduced
    // after Windows 10 1703. Delay-load the API set so the executable itself
    // still starts there and the runtime probe can select the Win32 fallback.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:api-ms-win-appmodel-runtime-l1-1-5.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }

    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the exe is three up.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    if let Some(target_dir) = out_dir.ancestors().nth(3) {
        let _ = fs::remove_file(target_dir.join("microsoft.windowsappruntime.bootstrap.dll"));
        let _ = fs::remove_file(target_dir.join("resources.pri"));
    }
}

mod hex {
    pub fn decode_lower(s: &str) -> Option<Vec<u8>> {
        let s = s.trim().to_ascii_lowercase();
        if !s.len().is_multiple_of(2) {
            return None;
        }
        let mut out = Vec::with_capacity(s.len() / 2);
        let mut bytes = s.bytes();
        while let (Some(h), Some(l)) = (bytes.next(), bytes.next()) {
            let h = val(h)?;
            let l = val(l)?;
            out.push((h << 4) | l);
        }
        Some(out)
    }
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    }
}
