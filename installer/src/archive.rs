// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! The embedded payload archive: ZIP, or PakLib when the builder was run with
//! `--pak`. The format is detected from the bytes, so callers never branch.

use anyhow::{Context, Result};
use std::io::{Cursor, Read};

pub enum PayloadArchive<'a> {
    Zip(zip::ZipArchive<Cursor<&'a [u8]>>),
    #[cfg(feature = "paklib")]
    Pak(common::pak::Archive<'a>),
}

impl<'a> PayloadArchive<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        if common::pak::is_pak(bytes) {
            #[cfg(feature = "paklib")]
            return Ok(Self::Pak(
                common::pak::Archive::open_memory(bytes).context("open embedded PakLib archive")?,
            ));
            #[cfg(not(feature = "paklib"))]
            anyhow::bail!(
                "payload is a PakLib archive but this installer was built without PakLib"
            );
        }
        Ok(Self::Zip(
            zip::ZipArchive::new(Cursor::new(bytes)).context("open embedded zip")?,
        ))
    }

    /// Stream one entry; `name` is its path inside the archive.
    pub fn entry<'b>(&'b mut self, name: &str) -> Result<Box<dyn Read + 'b>> {
        let missing = || format!("{name} missing from payload");
        Ok(match self {
            Self::Zip(archive) => Box::new(archive.by_name(name).with_context(missing)?),
            #[cfg(feature = "paklib")]
            Self::Pak(archive) => Box::new(archive.open(name).with_context(missing)?),
        })
    }

    /// Read one whole entry into memory.
    pub fn read_entry(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.entry(name)?
            .read_to_end(&mut bytes)
            .with_context(|| format!("read {name} from payload"))?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_zip_entries() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("full/a.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"hello").unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let mut archive = PayloadArchive::open(&bytes).unwrap();
        assert!(matches!(archive, PayloadArchive::Zip(_)));
        assert_eq!(archive.read_entry("full/a.txt").unwrap(), b"hello");
        let err = archive.read_entry("full/missing").unwrap_err();
        assert!(format!("{err:#}").contains("missing from payload"));
    }

    #[cfg(feature = "paklib")]
    #[test]
    fn reads_pak_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let output = dir.path().join("payload.pak");
        let mut writer = common::pak::Writer::create(
            &output,
            common::pak::WriterOptions {
                block_size: 64 * 1024,
                zstd_level: 3,
                minimum_saving: 0.0,
            },
        )
        .unwrap();
        writer
            .add_file(&dir.path().join("a.txt"), "full/a.txt")
            .unwrap();
        writer.finalize().unwrap();
        let bytes = std::fs::read(&output).unwrap();

        let mut archive = PayloadArchive::open(&bytes).unwrap();
        assert!(matches!(archive, PayloadArchive::Pak(_)));
        assert_eq!(archive.read_entry("full/a.txt").unwrap(), b"hello");
        let err = archive.read_entry("full/missing").unwrap_err();
        assert!(format!("{err:#}").contains("missing from payload"));
    }

    #[cfg(not(feature = "paklib"))]
    #[test]
    fn rejects_pak_without_paklib() {
        let err = PayloadArchive::open(b"PAKLIB1\0rest").err().unwrap();
        assert!(format!("{err:#}").contains("built without PakLib"));
    }
}
