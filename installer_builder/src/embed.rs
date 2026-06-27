// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, bail};
use std::path::Path;

/// Convert a `Path` to a null-terminated UTF-16 string for Win32 APIs.
pub(crate) fn wide_path(p: &Path) -> Vec<u16> {
    p.to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

/// Magic at the start of the appended payload overlay, so the installer can
/// sanity-check it found the right region.
pub const OVERLAY_MAGIC: &[u8; 8] = b"RIIPLD01";

/// Embed the small resources via the Win32 resource API: signed manifest
/// (id=2), uninstaller (id=3), payload length (id=4) and the optional header
/// banner PNG (id=5). The payload zip itself is appended as a PE overlay by
/// `append_payload` (no size limit, mmap-able).
pub fn embed_resources(
    exe: &Path,
    signed_json: &[u8],
    uninstaller_exe: &[u8],
    payload_len: u64,
    banner_png: Option<&[u8]>,
) -> Result<()> {
    use windows::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };
    use windows::core::PCWSTR;

    let wide = wide_path(exe);

    unsafe {
        let h =
            BeginUpdateResourceW(PCWSTR(wide.as_ptr()), false).context("BeginUpdateResource")?;
        if h.is_invalid() {
            bail!(
                "BeginUpdateResource returned invalid handle for {}",
                exe.display()
            );
        }

        const RT_RCDATA: u16 = 10;
        const LANG_NEUTRAL: u16 = 0;

        let put = |id: u16, data: &[u8], what: &str| -> Result<()> {
            UpdateResourceW(
                h,
                PCWSTR(RT_RCDATA as usize as *const u16),
                PCWSTR(id as usize as *const u16),
                LANG_NEUTRAL,
                Some(data.as_ptr() as *const _),
                data.len() as u32,
            )
            .with_context(|| format!("UpdateResource id={} ({})", id, what))
        };

        put(2, signed_json, "signed manifest")?;
        put(3, uninstaller_exe, "uninstaller")?;
        put(4, &payload_len.to_le_bytes(), "payload length")?;
        if let Some(banner) = banner_png {
            put(5, banner, "header banner")?;
        }

        EndUpdateResourceW(h, false).context("EndUpdateResource")?;
    }
    Ok(())
}

/// Append the payload zip as a PE overlay: `MAGIC || zip`, written straight to
/// the end of the file. Streaming, no resource-size limit. Must run AFTER all
/// `UpdateResource`/version/icon passes (those rewrite the PE and would drop a
/// pre-existing overlay) and BEFORE Authenticode signing (signtool appends its
/// certificate table after the overlay; the installer locates the overlay from
/// the PE section table, not the end of file, so a trailing cert is harmless).
pub fn append_payload(exe: &Path, payload_zip: &[u8]) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut f = OpenOptions::new()
        .append(true)
        .open(exe)
        .with_context(|| format!("open {} for overlay append", exe.display()))?;
    f.write_all(OVERLAY_MAGIC).context("write overlay magic")?;
    f.write_all(payload_zip).context("write overlay payload")?;
    f.flush().ok();
    Ok(())
}
