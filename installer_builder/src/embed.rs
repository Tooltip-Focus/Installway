// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use crate::icon::ExeIcons;
use anyhow::{Context, Result};
use editpe::{Image, ResourceData, ResourceEntry, ResourceEntryName, ResourceTable};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

const RT_RCDATA: u32 = 10;
const LANG_NEUTRAL: u32 = 0;

/// Magic at the start of the appended payload overlay, so the installer can
/// sanity-check it found the right region.
pub const OVERLAY_MAGIC: &[u8; 8] = b"RIIPLD01";

/// Wrap `data` as a `RT_RCDATA` name-level entry: a language sub-table holding
/// one neutral-language data leaf (matching what the installer's `FindResource`
/// lookup expects).
fn rcdata_entry(data: &[u8]) -> ResourceEntry {
    let mut leaf = ResourceData::default();
    leaf.set_data(data.to_vec());
    let mut lang_table = ResourceTable::default();
    lang_table.insert(
        ResourceEntryName::ID(LANG_NEUTRAL),
        ResourceEntry::Data(leaf),
    );
    ResourceEntry::Table(lang_table)
}

/// Everything [`embed_all`] writes into the setup exe's resource directory.
pub struct EmbedSpec<'a> {
    /// Signed manifest JSON (RCDATA id=2).
    pub signed_json: &'a [u8],
    /// Uninstaller exe bytes (RCDATA id=3).
    pub uninstaller_exe: &'a [u8],
    /// Byte length of the payload overlay (RCDATA id=4).
    pub payload_len: u64,
    /// Optional header banner PNG (RCDATA id=5; absent when `None`).
    pub banner_png: Option<&'a [u8]>,
    /// Product display name (RT_VERSION).
    pub product: &'a str,
    /// Publisher name (RT_VERSION).
    pub publisher: &'a str,
    /// Version string (RT_VERSION).
    pub version: &'a str,
    /// App icon tables to copy in (RT_ICON / RT_GROUP_ICON), when present.
    pub icons: Option<&'a ExeIcons>,
}

/// Embed every PE resource into `exe` in a **single** editpe pass, then leave
/// the file ready for [`append_payload`]:
///
/// * `RT_RCDATA` blobs read by the installer via `FindResource`, all under
///   neutral language: signed manifest (id=2), uninstaller (id=3), payload
///   length (id=4) and the optional header banner PNG (id=5);
/// * the `RT_VERSION` version-info block (Explorer Details tab + SmartScreen);
/// * the app's `RT_ICON` / `RT_GROUP_ICON` tables, when present.
///
/// Everything goes through one `parse_file` / `write_file` on purpose: editpe
/// relocates the resource section on write, so a *second* editpe pass over its
/// own output produces a PE the loader rejects (`ERROR_BAD_EXE_FORMAT`). One
/// pass keeps the binary valid.
pub fn embed_all(exe: &Path, spec: &EmbedSpec) -> Result<()> {
    let original = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "setup.exe".to_string());

    let mut image =
        Image::parse_file(exe).with_context(|| format!("parse {} as PE image", exe.display()))?;
    let mut resources = image.resource_directory().cloned().unwrap_or_default();

    // RT_RCDATA blobs. Reuse the existing type table if present, else start fresh.
    let mut rcdata = match resources.root().get(ResourceEntryName::ID(RT_RCDATA)) {
        Some(ResourceEntry::Table(t)) => t.clone(),
        _ => ResourceTable::default(),
    };
    rcdata.insert(ResourceEntryName::ID(2), rcdata_entry(spec.signed_json));
    rcdata.insert(ResourceEntryName::ID(3), rcdata_entry(spec.uninstaller_exe));
    rcdata.insert(
        ResourceEntryName::ID(4),
        rcdata_entry(&spec.payload_len.to_le_bytes()),
    );
    if let Some(banner) = spec.banner_png {
        rcdata.insert(ResourceEntryName::ID(5), rcdata_entry(banner));
    }
    resources.root_mut().insert(
        ResourceEntryName::ID(RT_RCDATA),
        ResourceEntry::Table(rcdata),
    );

    // RT_VERSION.
    resources
        .set_version_info(&crate::version::build(
            spec.product,
            spec.publisher,
            spec.version,
            &original,
        ))
        .context("set version-info resource")?;

    // RT_ICON / RT_GROUP_ICON (optional).
    if let Some(icons) = spec.icons {
        icons.apply(&mut resources);
    }

    image
        .set_resource_directory(resources)
        .context("set resource directory in image")?;
    image
        .write_file(exe)
        .with_context(|| format!("write {}", exe.display()))?;
    Ok(())
}

/// Append the payload zip as a PE overlay: `MAGIC || zip`, written straight to
/// the end of the file. Streaming, no resource-size limit. Must run AFTER
/// [`embed_all`] (that rewrites the PE and would drop a pre-existing overlay)
/// and BEFORE Authenticode signing (signtool appends its
/// certificate table after the overlay; the installer locates the overlay from
/// the PE section table, not the end of file, so a trailing cert is harmless).
pub fn append_payload(exe: &Path, payload_zip: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .append(true)
        .open(exe)
        .with_context(|| format!("open {} for overlay append", exe.display()))?;
    f.write_all(OVERLAY_MAGIC).context("write overlay magic")?;
    f.write_all(payload_zip).context("write overlay payload")?;
    f.flush().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk `RT_RCDATA -> id -> neutral-language` and return the leaf bytes,
    /// i.e. exactly the coordinates the installer's `FindResource(id, RCDATA)`
    /// resolves at runtime.
    fn read_rcdata(image: &Image, id: u32) -> Option<Vec<u8>> {
        Some(
            image
                .resource_directory()?
                .root()
                .get(ResourceEntryName::ID(RT_RCDATA))?
                .as_table()?
                .get(ResourceEntryName::ID(id))?
                .as_table()?
                .get(ResourceEntryName::ID(LANG_NEUTRAL))?
                .as_data()?
                .data()
                .to_vec(),
        )
    }

    fn stub_copy(dir: &Path) -> std::path::PathBuf {
        let exe = dir.join("stub.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        exe
    }

    #[test]
    fn embeds_rcdata_under_neutral_language() {
        let dir = tempfile::tempdir().unwrap();
        let exe = stub_copy(dir.path());

        let signed = b"SIGNED-MANIFEST-JSON";
        let uninst = b"UNINSTALLER-EXE-BYTES";
        let banner = b"PNG-BANNER";
        let len = 0x1122_3344_5566_7788u64;
        embed_all(
            &exe,
            &EmbedSpec {
                signed_json: signed,
                uninstaller_exe: uninst,
                payload_len: len,
                banner_png: Some(banner),
                product: "Prod",
                publisher: "Pub",
                version: "1.2.3",
                icons: None,
            },
        )
        .unwrap();

        let image = Image::parse_file(&exe).unwrap();
        assert_eq!(read_rcdata(&image, 2).as_deref(), Some(&signed[..]));
        assert_eq!(read_rcdata(&image, 3).as_deref(), Some(&uninst[..]));
        assert_eq!(
            read_rcdata(&image, 4).as_deref(),
            Some(&len.to_le_bytes()[..])
        );
        assert_eq!(read_rcdata(&image, 5).as_deref(), Some(&banner[..]));
        // Version info landed in the same pass.
        assert!(
            image
                .resource_directory()
                .unwrap()
                .get_version_info()
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn banner_is_optional() {
        let dir = tempfile::tempdir().unwrap();
        let exe = stub_copy(dir.path());
        embed_all(
            &exe,
            &EmbedSpec {
                signed_json: b"a",
                uninstaller_exe: b"b",
                payload_len: 7,
                banner_png: None,
                product: "P",
                publisher: "Q",
                version: "1.0",
                icons: None,
            },
        )
        .unwrap();

        let image = Image::parse_file(&exe).unwrap();
        assert!(read_rcdata(&image, 2).is_some());
        assert!(read_rcdata(&image, 5).is_none()); // no banner -> id 5 absent
    }
}
