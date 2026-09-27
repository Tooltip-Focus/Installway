// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! The embedded PakLib payload archive, with `anyhow` context on every error.

use anyhow::{Context, Result};
use std::io::Read;

pub struct PayloadArchive<'a>(common::pak::Archive<'a>);

impl<'a> PayloadArchive<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self(
            common::pak::Archive::open_memory(bytes).context("open embedded payload archive")?,
        ))
    }

    /// Stream one entry; `name` is its path inside the archive.
    pub fn entry(&self, name: &str) -> Result<common::pak::File<'a>> {
        self.0
            .open(name)
            .with_context(|| format!("{name} missing from payload"))
    }

    /// Read one whole entry into memory.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.entry(name)?
            .read_to_end(&mut bytes)
            .with_context(|| format!("read {name} from payload"))?;
        Ok(bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A payload archive holding each `(name, content)` entry.
    pub(crate) fn payload_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("payload.pak");
        let mut writer = common::pak::Writer::create(
            &output,
            common::pak::WriterOptions {
                block_size: 64 * 1024,
                zstd_level: 1,
                minimum_saving: 0.0,
            },
        )
        .unwrap();
        for (index, (name, content)) in entries.iter().enumerate() {
            let source = dir.path().join(index.to_string());
            std::fs::write(&source, content).unwrap();
            writer
                .add_file(&source, name, common::pak::Compression::Automatic)
                .unwrap();
        }
        writer.finalize().unwrap();
        std::fs::read(&output).unwrap()
    }

    #[test]
    fn reads_entries() {
        let bytes = payload_with(&[("full/a.txt", b"hello")]);
        let archive = PayloadArchive::open(&bytes).unwrap();
        assert_eq!(archive.read_entry("full/a.txt").unwrap(), b"hello");
        let err = archive.read_entry("full/missing").unwrap_err();
        assert!(format!("{err:#}").contains("missing from payload"));
    }

    #[test]
    fn rejects_other_formats() {
        let err = PayloadArchive::open(b"PK\x03\x04 not a pak").err().unwrap();
        assert!(format!("{err:#}").contains("open embedded payload archive"));
    }
}
