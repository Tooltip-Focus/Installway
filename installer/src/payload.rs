// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, bail};
use common::model::installer_payload::InstallerPayload;
use common::model::signed_payload::SignedPayload;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

include!(concat!(env!("OUT_DIR"), "/pub_key.rs"));

/// Must match `installer_builder::embed::OVERLAY_MAGIC`.
const OVERLAY_MAGIC: &[u8; 8] = b"RIIPLD01";

pub struct LoadedPayload {
    pub payload: InstallerPayload,
    pub uninstaller_bytes: Vec<u8>,
    /// Optional header-banner PNG bytes (resource id=5), drawn across the wizard
    /// header. `None` when the installer was packed without `--banner`; the UI
    /// then keeps its flat accent strip.
    pub banner_png: Option<Vec<u8>>,
    /// The whole exe, memory-mapped; the payload zip is a slice into it, so
    /// multi-GB payloads stay demand-paged instead of copied into RAM.
    map: memmap2::Mmap,
    zip_off: usize,
    zip_len: usize,
}

impl LoadedPayload {
    /// Borrowed view of the payload zip (mmap-backed, no heap copy).
    pub fn zip(&self) -> &[u8] {
        &self.map[self.zip_off..self.zip_off + self.zip_len]
    }
}

pub fn load_and_verify() -> Result<LoadedPayload> {
    let signed_bytes = read_resource(2)?;
    let uninstaller_bytes = read_resource(3)?;
    let payload_len = read_resource(4)?;
    if payload_len.len() != 8 {
        bail!("payload-length resource malformed");
    }
    let zip_len = u64::from_le_bytes(payload_len[..8].try_into().unwrap()) as usize;

    let signed: SignedPayload =
        serde_json::from_slice(&signed_bytes).context("parse signed payload JSON")?;

    verify_signature(&signed)?;

    let payload: InstallerPayload =
        serde_json::from_str(&signed.payload_json).context("parse inner payload JSON")?;

    // Locate the overlay from the PE section table (robust to a trailing
    // Authenticode certificate appended after it).
    let exe = std::env::current_exe().context("locate own exe")?;
    let file = std::fs::File::open(&exe).with_context(|| format!("open {}", exe.display()))?;
    let map = unsafe { memmap2::Mmap::map(&file) }.context("mmap own exe")?;

    let overlay_start = pe_overlay_offset(&map).context("locate payload overlay in PE")?;
    let magic_end = overlay_start + OVERLAY_MAGIC.len();
    if map.len() < magic_end || &map[overlay_start..magic_end] != OVERLAY_MAGIC {
        bail!("payload overlay missing or corrupt (bad magic)");
    }
    let zip_off = magic_end;
    if map.len() < zip_off + zip_len {
        bail!(
            "payload overlay truncated: need {} bytes from offset {}, file is {}",
            zip_len,
            zip_off,
            map.len()
        );
    }

    // Verify BLAKE3 of the payload, over the mmap (not copied).
    let actual_hash = blake3::hash(&map[zip_off..zip_off + zip_len])
        .to_hex()
        .to_string();
    if actual_hash != payload.payload_blake3 {
        bail!(
            "payload hash mismatch: manifest declared {} but overlay hashes to {}",
            payload.payload_blake3,
            actual_hash
        );
    }

    check_min_installer_version(&payload.min_installer_version)?;

    // The header banner (id=5) is optional branding, not signed; its absence is
    // normal and never fails the load.
    let banner_png = read_resource(5).ok();

    Ok(LoadedPayload {
        payload,
        uninstaller_bytes,
        banner_png,
        map,
        zip_off,
        zip_len,
    })
}

/// Offset where the PE image ends on disk = max(PointerToRawData + SizeOfRawData)
/// over all sections. Anything after that (our overlay, then optionally an
/// Authenticode cert table) is appended data.
fn pe_overlay_offset(data: &[u8]) -> Result<usize> {
    if data.len() < 0x40 || &data[0..2] != b"MZ" {
        bail!("not a PE (no MZ)");
    }
    let e_lfanew = u32::from_le_bytes(data[0x3C..0x40].try_into().unwrap()) as usize;
    if data.len() < e_lfanew + 24 || &data[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        bail!("not a PE (no PE signature)");
    }
    let coff = e_lfanew + 4;
    let num_sections = u16::from_le_bytes(data[coff + 2..coff + 4].try_into().unwrap()) as usize;
    let opt_size = u16::from_le_bytes(data[coff + 16..coff + 18].try_into().unwrap()) as usize;
    let sect_start = coff + 20 + opt_size;
    let mut end = 0usize;
    for i in 0..num_sections {
        let s = sect_start + i * 40;
        if data.len() < s + 40 {
            bail!("section header out of range");
        }
        let size_raw = u32::from_le_bytes(data[s + 16..s + 20].try_into().unwrap()) as usize;
        let ptr_raw = u32::from_le_bytes(data[s + 20..s + 24].try_into().unwrap()) as usize;
        end = end.max(ptr_raw.saturating_add(size_raw));
    }
    if end == 0 || end > data.len() {
        bail!(
            "computed overlay offset {} invalid (file {})",
            end,
            data.len()
        );
    }
    Ok(end)
}

fn verify_signature(signed: &SignedPayload) -> Result<()> {
    if PUB_KEY == [0u8; 32] {
        bail!("installer was built without INSTALLER_PUB_KEY - refusing to install");
    }
    let key = VerifyingKey::from_bytes(&PUB_KEY).context("invalid embedded public key")?;
    let sig_bytes = hex::decode(&signed.signature_hex).context("decode signature hex")?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let sig = Signature::from_bytes(&sig_arr);
    key.verify(signed.payload_json.as_bytes(), &sig)
        .context("Ed25519 signature verification failed")?;
    Ok(())
}

fn check_min_installer_version(required: &str) -> Result<()> {
    let installer = env!("CARGO_PKG_VERSION");
    if compare_semver(installer, required) < 0 {
        bail!(
            "installer version {} is too old (payload requires {})",
            installer,
            required
        );
    }
    Ok(())
}

fn compare_semver(a: &str, b: &str) -> i32 {
    // Leading digits per segment, position preserved: a non-numeric segment
    // ("x", "0-beta") counts as 0 instead of silently shifting the ones after
    // it (which turned "1.x.2" into "1.2").
    let seg = |s: &str| -> u64 {
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().unwrap_or(0)
    };
    let pa: Vec<u64> = a.split('.').map(seg).collect();
    let pb: Vec<u64> = b.split('.').map(seg).collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x < y {
            return -1;
        }
        if x > y {
            return 1;
        }
    }
    0
}

fn read_resource(id: u16) -> Result<Vec<u8>> {
    use windows::Win32::System::LibraryLoader::{
        FindResourceW, GetModuleHandleW, LoadResource, LockResource, SizeofResource,
    };
    use windows::core::PCWSTR;

    const RT_RCDATA: u16 = 10;

    unsafe {
        let module = GetModuleHandleW(PCWSTR::null()).context("GetModuleHandle")?;
        let hres = FindResourceW(
            Some(module),
            PCWSTR(id as usize as *const u16),
            PCWSTR(RT_RCDATA as usize as *const u16),
        );
        if hres.is_invalid() {
            bail!("FindResource id={} failed (resource missing?)", id);
        }
        let size = SizeofResource(Some(module), hres);
        if size == 0 {
            bail!("resource id={} has size 0", id);
        }
        let hglobal = LoadResource(Some(module), hres).context("LoadResource")?;
        let ptr = LockResource(hglobal);
        if ptr.is_null() {
            bail!("LockResource id={} returned null", id);
        }
        let slice = std::slice::from_raw_parts(ptr as *const u8, size as usize);
        Ok(slice.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_ordering() {
        assert_eq!(compare_semver("1.0.0", "1.0.0"), 0);
        assert_eq!(compare_semver("1.0", "1.0.0"), 0); // missing parts = 0
        assert!(compare_semver("1.2.0", "1.10.0") < 0); // numeric, not lexical
        assert!(compare_semver("2.0", "1.9") > 0);
        assert!(compare_semver("1.0.1", "1.0.0") > 0);
        // Non-numeric segments count as 0 in place, no positional shift.
        assert!(compare_semver("1.x.2", "1.1.0") < 0);
        assert_eq!(compare_semver("1.0.0-beta", "1.0.0"), 0);
        assert!(compare_semver("1.2rc1.0", "1.1.9") > 0); // leading digits used
    }

    #[test]
    fn pe_overlay_offset_minimal() {
        // Minimal PE: MZ, e_lfanew=0x40, "PE\0\0", 1 section, opt header size 0.
        let mut d = vec![0u8; 0x150];
        d[0] = b'M';
        d[1] = b'Z';
        d[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes()); // e_lfanew
        d[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        d[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // NumberOfSections
        d[coff + 16..coff + 18].copy_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
        let sect = coff + 20; // 0x58
        d[sect + 16..sect + 20].copy_from_slice(&0x50u32.to_le_bytes()); // SizeOfRawData
        d[sect + 20..sect + 24].copy_from_slice(&0x100u32.to_le_bytes()); // PointerToRawData
        assert_eq!(pe_overlay_offset(&d).unwrap(), 0x150);
        assert!(pe_overlay_offset(b"not a pe").is_err());
    }
}
