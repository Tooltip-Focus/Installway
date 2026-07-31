// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Header-banner image, staged to a temp file because WinUI's `Image` takes a
//! URI where the Win32 wizard decodes the same bytes itself. A failure here is
//! non-fatal: the wizard just keeps its flat header.

use std::path::PathBuf;

pub(super) struct StagedBanner {
    path: PathBuf,
}

impl StagedBanner {
    pub(super) fn write(png: &[u8]) -> Option<StagedBanner> {
        // Process-unique: two installers at once must not share the file.
        let path =
            std::env::temp_dir().join(format!("installway-banner-{}.png", std::process::id()));
        match std::fs::write(&path, png) {
            Ok(()) => Some(StagedBanner { path }),
            Err(e) => {
                common::log::warn(format!("stage banner {}: {e}", path.display()));
                None
            }
        }
    }

    pub(super) fn uri(&self) -> String {
        format!("file:///{}", self.path.to_string_lossy().replace('\\', "/"))
    }
}

impl Drop for StagedBanner {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
