// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Optional header-banner image, decoded and drawn with GDI+.
//!
//! The banner PNG rides as a dedicated raw resource (id=5, see
//! `installer_builder::embed`) and is loaded here once at window creation. GDI+
//! gives us OS-native PNG decoding (alpha included) plus high-quality scaling,
//! so the same source image stays crisp whether the wizard renders at 100 %,
//! 125 %, 150 % or 200 % — we just draw it stretched into the DPI-scaled header
//! rect. When no banner is packaged the wizard keeps its flat accent strip and
//! this module is never used.

use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::GdiPlus::{
    GdipCreateFromHDC, GdipDeleteGraphics, GdipDisposeImage, GdipDrawImageRectI,
    GdipLoadImageFromStream, GdipSetInterpolationMode, GdipSetPixelOffsetMode, GdiplusShutdown,
    GdiplusStartup, GdiplusStartupInput, GpGraphics, GpImage, InterpolationModeHighQualityBicubic,
    PixelOffsetModeHalf,
};
use windows::Win32::System::Com::IStream;
use windows::Win32::UI::Shell::SHCreateMemStream;

/// A decoded GDI+ image plus the GDI+ token and backing stream that must outlive
/// it. Dropping disposes the image and shuts GDI+ down.
pub(super) struct BannerImage {
    token: usize,
    image: *mut GpImage,
    /// GDI+ keeps a reference to the source stream for the image's lifetime; hold
    /// it here so it cannot be released early.
    _stream: IStream,
}

impl BannerImage {
    /// Decode `png` into a GDI+ image. Returns `None` if GDI+ fails to start or
    /// the bytes don't decode — the caller then keeps the flat accent strip, so a
    /// bad banner never blocks the install.
    pub(super) unsafe fn load(png: &[u8]) -> Option<BannerImage> {
        unsafe {
            let mut token: usize = 0;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            };
            if GdiplusStartup(&mut token, &input, std::ptr::null_mut()).0 != 0 {
                return None;
            }
            // A memory IStream over the PNG bytes (SHCreateMemStream copies them).
            let stream = match SHCreateMemStream(Some(png)) {
                Some(s) => s,
                None => {
                    GdiplusShutdown(token);
                    return None;
                }
            };
            let mut image: *mut GpImage = std::ptr::null_mut();
            if GdipLoadImageFromStream(&stream, &mut image).0 != 0 || image.is_null() {
                GdiplusShutdown(token);
                return None;
            }
            Some(BannerImage {
                token,
                image,
                _stream: stream,
            })
        }
    }

    /// Draw the banner stretched to fill `w`×`h` device pixels at (`x`, `y`),
    /// with high-quality bicubic interpolation so DPI up/down-scaling stays sharp.
    pub(super) unsafe fn draw(&self, hdc: HDC, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let mut g: *mut GpGraphics = std::ptr::null_mut();
            if GdipCreateFromHDC(hdc, &mut g).0 != 0 || g.is_null() {
                return;
            }
            let _ = GdipSetInterpolationMode(g, InterpolationModeHighQualityBicubic);
            let _ = GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
            let _ = GdipDrawImageRectI(g, self.image, x, y, w, h);
            let _ = GdipDeleteGraphics(g);
        }
    }
}

impl Drop for BannerImage {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                let _ = GdipDisposeImage(self.image);
            }
            GdiplusShutdown(self.token);
        }
    }
}
