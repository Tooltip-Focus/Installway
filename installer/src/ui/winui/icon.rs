// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! This exe's own icon as a PNG on disk, for the compact updater's `Image`.
//! Silent on failure: the updater renders without an icon.

use std::path::PathBuf;
use windows::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromHICON, GdipDisposeImage, GdipSaveImageToFile, GdiplusShutdown,
    GdiplusStartup, GdiplusStartupInput, GpBitmap, GpImage,
};
use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
use windows::core::{GUID, PCWSTR};

/// GDI+ has no by-name encoder lookup, so the PNG encoder CLSID is hardcoded.
const PNG_ENCODER: GUID = GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);

pub(super) struct StagedIcon {
    path: PathBuf,
}

impl StagedIcon {
    pub(super) fn uri(&self) -> String {
        format!("file:///{}", self.path.to_string_lossy().replace('\\', "/"))
    }
}

impl Drop for StagedIcon {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(super) fn stage_own_icon() -> Option<StagedIcon> {
    let hicon = unsafe { crate::ui::helpers::own_icon() };
    if hicon.is_invalid() {
        return None;
    }
    let path = std::env::temp_dir().join(format!("installway-icon-{}.png", std::process::id()));
    let wide = common::utils::wide(&path.to_string_lossy());

    let ok = unsafe {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        if GdiplusStartup(&mut token, &input, std::ptr::null_mut()).0 != 0 {
            let _ = DestroyIcon(hicon);
            return None;
        }
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        let created = GdipCreateBitmapFromHICON(hicon, &mut bitmap).0 == 0 && !bitmap.is_null();
        let saved = created
            && GdipSaveImageToFile(
                bitmap as *mut GpImage,
                PCWSTR(wide.as_ptr()),
                &PNG_ENCODER,
                std::ptr::null(),
            )
            .0 == 0;
        if created {
            let _ = GdipDisposeImage(bitmap as *mut GpImage);
        }
        GdiplusShutdown(token);
        // `own_icon` returns a fresh handle from `ExtractIconW`.
        let _ = DestroyIcon(hicon);
        saved
    };

    ok.then_some(StagedIcon { path })
}
