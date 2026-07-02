// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Copy the icon resources from the packaged exe into the installer and
//! uninstaller .exe so Explorer shows the right thumbnail.
//!
//! Done by copying the whole `RT_GROUP_ICON` and `RT_ICON` resource subtrees
//! verbatim (ids + languages preserved) via the `editpe` crate — no `rc.exe`
//! and no raw Win32 resource calls. Copying both tables together keeps the
//! group directory and the icons it references self-consistent.

use anyhow::{Context, Result};
use editpe::{Image, ResourceDirectory, ResourceEntry, ResourceEntryName};
use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

/// The icon resource subtrees pulled from a source exe, ready to re-embed.
pub struct ExeIcons {
    /// The whole `RT_GROUP_ICON` type table.
    group_icons: ResourceEntry,
    /// The whole `RT_ICON` type table.
    icons: ResourceEntry,
}

impl ExeIcons {
    /// Number of group-icon directories (distinct `RT_GROUP_ICON` ids).
    pub fn group_count(&self) -> usize {
        self.group_icons.as_table().map_or(0, |t| t.entries().len())
    }

    /// Number of individual icon images (distinct `RT_ICON` ids).
    pub fn icon_count(&self) -> usize {
        self.icons.as_table().map_or(0, |t| t.entries().len())
    }

    /// Insert the icon tables into `res`, replacing any existing ones. Kept as an
    /// in-memory mutation (not its own file write) so it can share a single
    /// editpe pass with the other resource edits — sequential editpe writes on
    /// the same file corrupt the PE.
    pub fn apply(&self, res: &mut ResourceDirectory) {
        res.root_mut()
            .insert(ResourceEntryName::ID(RT_ICON), self.icons.clone());
        res.root_mut().insert(
            ResourceEntryName::ID(RT_GROUP_ICON),
            self.group_icons.clone(),
        );
    }
}

/// Read every icon resource from `exe`, preserving identifiers.
/// Returns `Ok(None)` if the source exe has no icons (still a success).
pub fn extract_from_exe(exe: &Path) -> Result<Option<ExeIcons>> {
    let image =
        Image::parse_file(exe).with_context(|| format!("parse {} as PE image", exe.display()))?;
    let Some(resources) = image.resource_directory() else {
        return Ok(None);
    };
    let group_icons = resources.root().get(ResourceEntryName::ID(RT_GROUP_ICON));
    let icons = resources.root().get(ResourceEntryName::ID(RT_ICON));
    // Need both tables to produce a usable, self-consistent icon.
    match (group_icons, icons) {
        (Some(g), Some(i)) => Ok(Some(ExeIcons {
            group_icons: g.clone(),
            icons: i.clone(),
        })),
        _ => Ok(None),
    }
}

/// Copy the extracted icon subtrees into `target` in a single editpe pass,
/// replacing any existing `RT_ICON` / `RT_GROUP_ICON` tables. Use this only
/// when icons are the *only* resource edit for `target` (e.g. the standalone
/// uninstaller); for setup.exe the icons ride along in [`crate::embed::embed_all`].
pub fn embed_icons(target: &Path, icons: &ExeIcons) -> Result<()> {
    let mut image = Image::parse_file(target)
        .with_context(|| format!("parse {} as PE image", target.display()))?;
    let mut resources = image.resource_directory().cloned().unwrap_or_default();
    icons.apply(&mut resources);
    image
        .set_resource_directory(resources)
        .context("set resource directory in image")?;
    image
        .write_file(target)
        .with_context(|| format!("write {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use editpe::{ResourceData, ResourceTable};

    /// Build a `type -> id 1 -> neutral-language -> data` subtree, matching the
    /// shape a real `RT_ICON` / `RT_GROUP_ICON` table has.
    fn type_table_with(data: &[u8]) -> ResourceEntry {
        let mut leaf = ResourceData::default();
        leaf.set_data(data.to_vec());
        let mut lang = ResourceTable::default();
        lang.insert(ResourceEntryName::ID(0), ResourceEntry::Data(leaf));
        let mut name = ResourceTable::default();
        name.insert(ResourceEntryName::ID(1), ResourceEntry::Table(lang));
        ResourceEntry::Table(name)
    }

    fn leaf_bytes(target: &Path, ty: u32) -> Vec<u8> {
        let image = Image::parse_file(target).unwrap();
        image
            .resource_directory()
            .unwrap()
            .root()
            .get(ResourceEntryName::ID(ty))
            .unwrap()
            .as_table()
            .unwrap()
            .get(ResourceEntryName::ID(1))
            .unwrap()
            .as_table()
            .unwrap()
            .get(ResourceEntryName::ID(0))
            .unwrap()
            .as_data()
            .unwrap()
            .data()
            .to_vec()
    }

    /// Seed a copy of the test binary with synthetic icon tables so we can drive
    /// the real extract + embed paths end to end.
    fn seed_source(dir: &Path, group: &[u8], icon: &[u8]) -> std::path::PathBuf {
        let path = dir.join("source.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &path).unwrap();
        let mut image = Image::parse_file(&path).unwrap();
        let mut res = image.resource_directory().cloned().unwrap_or_default();
        res.root_mut()
            .insert(ResourceEntryName::ID(RT_GROUP_ICON), type_table_with(group));
        res.root_mut()
            .insert(ResourceEntryName::ID(RT_ICON), type_table_with(icon));
        image.set_resource_directory(res).unwrap();
        image.write_file(&path).unwrap();
        path
    }

    #[test]
    fn extract_then_embed_preserves_icon_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let group = b"GROUP-ICON-DIRECTORY";
        let icon = b"ICON-IMAGE-PIXELS";
        let source = seed_source(dir.path(), group, icon);

        let extracted = extract_from_exe(&source).unwrap().expect("icons present");
        assert_eq!(extracted.group_count(), 1);
        assert_eq!(extracted.icon_count(), 1);

        let target = dir.path().join("target.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
        embed_icons(&target, &extracted).unwrap();

        // Bytes survive verbatim at the same tree coordinates.
        assert_eq!(leaf_bytes(&target, RT_GROUP_ICON), group);
        assert_eq!(leaf_bytes(&target, RT_ICON), icon);
    }

    #[test]
    fn missing_either_table_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        // A plain copy with no icon resources of its own must extract to None.
        let bare = dir.path().join("bare.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &bare).unwrap();
        // Only add a group table, no RT_ICON -> still None (not self-consistent).
        let mut image = Image::parse_file(&bare).unwrap();
        let mut res = image.resource_directory().cloned().unwrap_or_default();
        res.root_mut()
            .insert(ResourceEntryName::ID(RT_GROUP_ICON), type_table_with(b"g"));
        res.root_mut().remove(ResourceEntryName::ID(RT_ICON)); // guarantee absence
        image.set_resource_directory(res).unwrap();
        image.write_file(&bare).unwrap();

        assert!(extract_from_exe(&bare).unwrap().is_none());
    }
}
