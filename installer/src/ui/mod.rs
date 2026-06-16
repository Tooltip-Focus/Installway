// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Installer UIs over a shared set of Win32 helpers ([`helpers`]):
//! the full wizard ([`win32`]) and the compact auto-update window ([`minimal`]).

mod helpers;
pub mod minimal;
pub mod win32;

/// Dev-only sample payload so `--preview` can render a view without a real,
/// signed installer payload. `view` may contain `patch` to preview the patch
/// subheader; otherwise a full install is described.
#[cfg(debug_assertions)]
pub(crate) fn sample_payload(view: &str) -> common::models::InstallerPayload {
    use common::models::{InstallerPayload, Manifest, PayloadKind};
    let is_patch = view.contains("patch");
    InstallerPayload {
        kind: if is_patch {
            PayloadKind::Patch
        } else {
            PayloadKind::Full
        },
        product: "Sample App".to_string(),
        product_id: "SampleApp".to_string(),
        publisher: "Acme Corp".to_string(),
        from_version: is_patch.then(|| "1.1.0".to_string()),
        to_version: "1.2.0".to_string(),
        min_installer_version: "1.0.0".to_string(),
        payload_blake3: String::new(),
        created_at_unix: 0,
        manifest: Manifest {
            version: "1.2.0".to_string(),
            exe: "bin/app.exe".to_string(),
            files: std::collections::HashMap::new(),
            deleted_files: Vec::new(),
            full_size: 12_345_678,
            total_patch_size: 0,
        },
        license_text: None,
        associations: Vec::new(),
        shortcuts: Vec::new(),
        force_reinstall: false,
        purge_unknown_files: false,
        skip_license: false,
        skip_path: false,
        install_dir_restriction: common::models::InstallDirRestriction::Enforce,
        default_install_dir: None,
        upgrade_minimal_ui: false,
        show_uninstall_complete: false,
        registry: Vec::new(),
        plugins: Vec::new(),
    }
}
