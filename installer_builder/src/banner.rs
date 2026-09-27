// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Optional header-banner images: read, validate as PNG, and report their size.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

/// The 8-byte PNG file signature.
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The header banners to embed: `light` (`--banner`) serves every theme unless
/// `dark` (`--banner-dark`) is set, which the WinUI wizard shows in dark mode.
#[derive(Default)]
pub(crate) struct Banners {
    pub light: Option<Vec<u8>>,
    pub dark: Option<Vec<u8>>,
}

impl Banners {
    pub(crate) fn read(light: Option<&Path>, dark: Option<&Path>) -> Result<Self> {
        if dark.is_some() && light.is_none() {
            bail!(
                "--banner-dark needs --banner: the main banner serves the light theme and the \
                 Win32 wizard, the dark one only replaces it in dark mode"
            );
        }
        Ok(Self {
            light: light.map(|p| read_banner_png(p, "Banner")).transpose()?,
            dark: dark
                .map(|p| read_banner_png(p, "Dark banner"))
                .transpose()?,
        })
    }
}

/// Read and validate one header-banner image. Must be a PNG (the
/// runtime decoder and the docs assume PNG); the bytes ride inside the signed
/// payload, so an oversized image bloats every download — we warn past ~512 KB
/// but do not hard-fail (a high-res 2x banner can legitimately be a few hundred
/// KB). Dimensions are read from the IHDR chunk for an informational log line.
/// `label` names it in the log line.
fn read_banner_png(path: &Path, label: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(path).with_context(|| format!("read banner {}", path.display()))?;
    if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
        bail!(
            "banner {} is not a PNG (bad signature); the header banner must be a .png file",
            path.display()
        );
    }
    let dims = png_dimensions(&bytes)
        .map(|(w, h)| format!("{w}x{h}"))
        .unwrap_or_else(|| "unknown size".to_string());
    println!(
        "{label}: {} ({}, {} bytes) from {}",
        dims,
        human_bytes(bytes.len()),
        bytes.len(),
        path.display()
    );
    const WARN_BYTES: usize = 512 * 1024;
    if bytes.len() > WARN_BYTES {
        eprintln!(
            "warning: banner is {} ({} bytes) — it travels inside the signed payload, so a \
             smaller, optimized PNG keeps the installer lean",
            human_bytes(bytes.len()),
            bytes.len()
        );
    }
    Ok(bytes)
}

/// Width/height from a PNG's IHDR chunk (bytes 16..24, big-endian). `None` if
/// the buffer is too short or the first chunk is not IHDR (malformed PNG).
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

/// Compact human-readable byte size for log lines (KB/MB, 1 decimal).
fn human_bytes(n: usize) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 {
        format!("{:.1} MB", n / (1024.0 * 1024.0))
    } else if n >= 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_accepts_png_and_reads_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.png");
        // PNG signature + a minimal IHDR with width=1400, height=144.
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13]); // IHDR length
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1400u32.to_be_bytes());
        bytes.extend_from_slice(&144u32.to_be_bytes());
        std::fs::write(&p, &bytes).unwrap();
        let out = read_banner_png(&p, "Banner").unwrap();
        assert_eq!(out, bytes);
        assert_eq!(png_dimensions(&bytes), Some((1400, 144)));
    }

    #[test]
    fn banners_read_light_and_optional_dark() {
        let dir = tempfile::tempdir().unwrap();
        let light = dir.path().join("light.png");
        let dark = dir.path().join("dark.png");
        std::fs::write(&light, [&PNG_MAGIC[..], b"light"].concat()).unwrap();
        std::fs::write(&dark, [&PNG_MAGIC[..], b"dark"].concat()).unwrap();

        let none = Banners::read(None, None).unwrap();
        assert!(none.light.is_none() && none.dark.is_none());

        let one = Banners::read(Some(&light), None).unwrap();
        assert!(one.light.unwrap().ends_with(b"light"));
        assert!(one.dark.is_none());

        let both = Banners::read(Some(&light), Some(&dark)).unwrap();
        assert!(both.light.unwrap().ends_with(b"light"));
        assert!(both.dark.unwrap().ends_with(b"dark"));
    }

    #[test]
    fn dark_banner_requires_main_banner() {
        let dir = tempfile::tempdir().unwrap();
        let dark = dir.path().join("dark.png");
        std::fs::write(&dark, PNG_MAGIC).unwrap();
        let err = Banners::read(None, Some(&dark)).err().unwrap().to_string();
        assert!(err.contains("--banner-dark needs --banner"), "got: {err}");
    }

    #[test]
    fn png_dimensions_requires_ihdr_tag() {
        // Right length, but the first chunk is not IHDR -> no dimensions.
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"JUNK");
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(png_dimensions(&bytes), None);
    }

    #[test]
    fn banner_rejects_non_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.png");
        std::fs::write(&p, b"GIF89a not really a png").unwrap();
        let err = read_banner_png(&p, "Banner").unwrap_err().to_string();
        assert!(err.contains("not a PNG"), "got: {err}");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
