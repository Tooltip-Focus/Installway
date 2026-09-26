// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Safe, minimal Rust ownership layer over PakLib's C ABI (`pak/CAbi.h`).
//!
//! Only what Installway needs: open an archive from memory, stream entries
//! out of it, and write an archive from files on disk.

use std::ffi::{CStr, c_char, c_void};
use std::io::{self, Read};
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;

/// `PAK_COMPRESSION_AUTOMATIC`: zstd per block, stored raw when a
/// block saves less than the writer's `minimum_saving`.
const COMPRESSION_AUTOMATIC: u8 = 2;

/// Data alignment of each entry inside the archive, in bytes.
const DATA_ALIGNMENT: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PakError {
    code: u16,
    reserved: u16,
    native_code: u32,
    file_offset: u64,
}

#[repr(C)]
struct PakWriterOptions {
    block_size: u32,
    zstd_level: i32,
    minimum_saving: f32,
    data_alignment: u32,
    checksum_entries: u8,
    reserved: [u8; 3],
}

enum ArchiveHandle {}
enum FileHandle {}
enum WriterHandle {}

unsafe extern "C" {
    fn pak_archive_open_memory(
        data: *const c_void,
        size: usize,
        out_archive: *mut *mut ArchiveHandle,
        out_error: *mut PakError,
    ) -> u16;
    fn pak_archive_destroy(archive: *mut ArchiveHandle);
    fn pak_archive_open_file(
        archive: *mut ArchiveHandle,
        path_utf8: *const c_char,
        path_size: usize,
        out_file: *mut *mut FileHandle,
        out_size: *mut u64,
        out_error: *mut PakError,
    ) -> u16;
    fn pak_file_destroy(file: *mut FileHandle);
    fn pak_file_read(
        file: *mut FileHandle,
        offset: u64,
        destination: *mut c_void,
        destination_size: usize,
        out_read: *mut usize,
        out_error: *mut PakError,
    ) -> u16;
    fn pak_writer_create(
        output_path: *const u16,
        options: *const PakWriterOptions,
        out_writer: *mut *mut WriterHandle,
        out_error: *mut PakError,
    ) -> u16;
    fn pak_writer_add_file(
        writer: *mut WriterHandle,
        source_path: *const u16,
        archive_path_utf8: *const c_char,
        archive_path_size: usize,
        compression_policy: u8,
        out_error: *mut PakError,
    ) -> u16;
    fn pak_writer_finalize(writer: *mut WriterHandle, out_error: *mut PakError) -> u16;
    fn pak_writer_destroy(writer: *mut WriterHandle);
    fn pak_error_message(code: u16) -> *const c_char;
}

/// Turn a C ABI status into `Ok(())` or a descriptive `io::Error`.
fn check(status: u16, detail: PakError) -> io::Result<()> {
    if status == 0 {
        return Ok(());
    }
    // SAFETY: PakLib returns a static NUL-terminated string, or null.
    let message = unsafe {
        let ptr = pak_error_message(status);
        if ptr.is_null() {
            "unknown PakLib error".into()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    };
    Err(io::Error::other(format!(
        "{message} (code={status}, native={}, offset={})",
        detail.native_code, detail.file_offset
    )))
}

fn non_null<T>(raw: *mut T) -> io::Result<NonNull<T>> {
    NonNull::new(raw).ok_or_else(|| io::Error::other("PakLib returned a null handle"))
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// A read-only archive over caller-owned bytes (typically an mmap slice).
pub struct Archive<'a> {
    raw: NonNull<ArchiveHandle>,
    _bytes: PhantomData<&'a [u8]>,
}

impl<'a> Archive<'a> {
    /// Parse the archive index. `bytes` are borrowed, not copied.
    pub fn open_memory(bytes: &'a [u8]) -> io::Result<Self> {
        let mut raw = std::ptr::null_mut();
        let mut detail = PakError::default();
        // SAFETY: `bytes` outlives the archive and every file opened from it,
        // which the `'a` lifetime on `Archive` and `File` enforces.
        let status = unsafe {
            pak_archive_open_memory(bytes.as_ptr().cast(), bytes.len(), &mut raw, &mut detail)
        };
        check(status, detail)?;
        Ok(Self {
            raw: non_null(raw)?,
            _bytes: PhantomData,
        })
    }

    /// Open one entry for streaming reads.
    pub fn open(&self, path: &str) -> io::Result<File<'a>> {
        let mut raw = std::ptr::null_mut();
        let mut size = 0;
        let mut detail = PakError::default();
        // SAFETY: `path` is passed with its explicit length (no NUL needed).
        let status = unsafe {
            pak_archive_open_file(
                self.raw.as_ptr(),
                path.as_ptr().cast(),
                path.len(),
                &mut raw,
                &mut size,
                &mut detail,
            )
        };
        check(status, detail)?;
        Ok(File {
            raw: non_null(raw)?,
            offset: 0,
            size,
            _bytes: PhantomData,
        })
    }
}

impl Drop for Archive<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `pak_archive_open_memory` and is dropped once.
        unsafe { pak_archive_destroy(self.raw.as_ptr()) }
    }
}

/// One archive entry, read sequentially through [`Read`]. It keeps the
/// archive's index alive on its own but still borrows the archive bytes.
pub struct File<'a> {
    raw: NonNull<FileHandle>,
    offset: u64,
    size: u64,
    _bytes: PhantomData<&'a [u8]>,
}

impl Read for File<'_> {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        let remaining = self.size - self.offset;
        let capacity = destination
            .len()
            .min(remaining.try_into().unwrap_or(usize::MAX));
        if capacity == 0 {
            return Ok(0);
        }
        let mut read = 0;
        let mut detail = PakError::default();
        // SAFETY: `destination` is valid for `capacity` bytes.
        let status = unsafe {
            pak_file_read(
                self.raw.as_ptr(),
                self.offset,
                destination.as_mut_ptr().cast(),
                capacity,
                &mut read,
                &mut detail,
            )
        };
        check(status, detail)?;
        self.offset += read as u64;
        Ok(read)
    }
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `pak_archive_open_file` and is dropped once.
        unsafe { pak_file_destroy(self.raw.as_ptr()) }
    }
}

/// Compression settings for [`Writer::create`].
#[derive(Clone, Copy, Debug)]
pub struct WriterOptions {
    /// Uncompressed size of each independent zstd block (PakLib max: 64 MiB).
    pub block_size: u32,
    pub zstd_level: i32,
    /// A block is stored raw unless compression saves at least this fraction.
    pub minimum_saving: f32,
}

/// Writes a new archive to disk; nothing is readable until [`Writer::finalize`].
pub struct Writer {
    raw: NonNull<WriterHandle>,
}

impl Writer {
    pub fn create(output: &Path, options: WriterOptions) -> io::Result<Self> {
        let output = wide(output);
        let options = PakWriterOptions {
            block_size: options.block_size,
            zstd_level: options.zstd_level,
            minimum_saving: options.minimum_saving,
            data_alignment: DATA_ALIGNMENT,
            checksum_entries: 1,
            reserved: [0; 3],
        };
        let mut raw = std::ptr::null_mut();
        let mut detail = PakError::default();
        // SAFETY: `output` is NUL-terminated UTF-16 and `options` is repr(C).
        let status = unsafe { pak_writer_create(output.as_ptr(), &options, &mut raw, &mut detail) };
        check(status, detail)?;
        Ok(Self {
            raw: non_null(raw)?,
        })
    }

    /// Append the file at `source` as `archive_path` (automatic compression).
    pub fn add_file(&mut self, source: &Path, archive_path: &str) -> io::Result<()> {
        let source = wide(source);
        let mut detail = PakError::default();
        // SAFETY: `source` is NUL-terminated; `archive_path` has its length.
        let status = unsafe {
            pak_writer_add_file(
                self.raw.as_ptr(),
                source.as_ptr(),
                archive_path.as_ptr().cast(),
                archive_path.len(),
                COMPRESSION_AUTOMATIC,
                &mut detail,
            )
        };
        check(status, detail)
    }

    /// Write the index and footer. Dropping without finalizing discards the
    /// partial output.
    pub fn finalize(self) -> io::Result<()> {
        let mut detail = PakError::default();
        // SAFETY: `raw` is a live writer; `Drop` still destroys it afterwards.
        let status = unsafe { pak_writer_finalize(self.raw.as_ptr(), &mut detail) };
        check(status, detail)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `pak_writer_create` and is dropped once.
        unsafe { pak_writer_destroy(self.raw.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::super::is_pak;
    use super::*;

    #[test]
    fn round_trips_files_through_memory() {
        let dir = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i / 7 % 26) as u8 + b'a').collect();
        std::fs::write(dir.path().join("big.txt"), &big).unwrap();
        std::fs::write(dir.path().join("empty.bin"), b"").unwrap();

        let output = dir.path().join("out.pak");
        let mut writer = Writer::create(
            &output,
            WriterOptions {
                block_size: 64 * 1024,
                zstd_level: 3,
                minimum_saving: 0.0,
            },
        )
        .unwrap();
        writer
            .add_file(&dir.path().join("big.txt"), "a/big.txt")
            .unwrap();
        writer
            .add_file(&dir.path().join("empty.bin"), "empty.bin")
            .unwrap();
        writer.finalize().unwrap();

        let bytes = std::fs::read(&output).unwrap();
        assert!(is_pak(&bytes));
        let archive = Archive::open_memory(&bytes).unwrap();

        // Odd-sized reads cross block boundaries and hit the decoded-block cache.
        let mut file = archive.open("a/big.txt").unwrap();
        let mut out = Vec::new();
        let mut chunk = [0u8; 10_007];
        loop {
            let n = file.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(out, big);

        let mut empty = Vec::new();
        archive
            .open("empty.bin")
            .unwrap()
            .read_to_end(&mut empty)
            .unwrap();
        assert!(empty.is_empty());
        assert!(archive.open("missing").is_err());
    }
}
