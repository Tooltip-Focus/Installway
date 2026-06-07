// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, anyhow};
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use walkdir::WalkDir;

/// Retry budget for filesystem mutations that lose to a transient lock
/// (Defender/other AV scanning a freshly written file, Explorer, the search
/// indexer). 50 × 100 ms ≈ 5 s, which comfortably outlasts a real-time scan of
/// a typical file. Shared by the installer commit and these helpers so the
/// whole product retries with one policy.
pub const FS_RETRIES: usize = 50;
pub const FS_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Rename `src` → `dest`, retrying transient failures. Creates `dest`'s parent
/// and removes an existing `dest` first (Windows `rename` fails if the target
/// exists). The dominant failure this survives: an AV holding a brief lock on a
/// just-created file (especially `.exe`).
pub fn rename_retry(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut last_err = None;
    for _ in 0..FS_RETRIES {
        if dest.exists() {
            let _ = fs::remove_file(dest);
        }
        match fs::rename(src, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(FS_RETRY_DELAY);
            }
        }
    }
    Err(anyhow!(
        "could not move {} -> {} after {} attempts: {}",
        src.display(),
        dest.display(),
        FS_RETRIES,
        last_err.map(|e| e.to_string()).unwrap_or_else(|| "unknown".into())
    ))
}

/// Remove `path`, retrying transient locks. `Ok` if it's already gone.
pub fn remove_file_retry(path: &Path) -> Result<()> {
    let mut last_err = None;
    for _ in 0..FS_RETRIES {
        if !path.exists() {
            return Ok(());
        }
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(FS_RETRY_DELAY);
            }
        }
    }
    Err(anyhow!(
        "could not remove {} after {} attempts: {}",
        path.display(),
        FS_RETRIES,
        last_err.map(|e| e.to_string()).unwrap_or_else(|| "unknown".into())
    ))
}

/// Recursively remove a directory, retrying through transient locks.
pub fn remove_dir_retry(dir: &Path) {
    for _ in 0..FS_RETRIES {
        if !dir.exists() {
            return;
        }
        if fs::remove_dir_all(dir).is_ok() {
            return;
        }
        std::thread::sleep(FS_RETRY_DELAY);
    }
}

/// Copy `src` → `dest`, retrying transient locks. Mainly for copying an `.exe`
/// (the prime AV-scan target) into a scratch location, where a bare `fs::copy`
/// can lose to the real-time scan of the freshly written file.
pub fn copy_retry(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut last_err = None;
    for _ in 0..FS_RETRIES {
        match fs::copy(src, dest) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(FS_RETRY_DELAY);
            }
        }
    }
    Err(anyhow!(
        "could not copy {} -> {} after {} attempts: {}",
        src.display(),
        dest.display(),
        FS_RETRIES,
        last_err.map(|e| e.to_string()).unwrap_or_else(|| "unknown".into())
    ))
}

/// Write `bytes` to `path`, retrying transient locks (a stale `.tmp` from a
/// prior run may still be briefly held by a scanner).
fn write_bytes_retry(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut last_err = None;
    for _ in 0..FS_RETRIES {
        match fs::write(path, bytes) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(FS_RETRY_DELAY);
            }
        }
    }
    Err(anyhow!(
        "could not write {} after {} attempts: {}",
        path.display(),
        FS_RETRIES,
        last_err.map(|e| e.to_string()).unwrap_or_else(|| "unknown".into())
    ))
}

pub fn file_blake3(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn bytes_blake3(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Write a file atomically: write to a sibling `.tmp` then rename over the
/// target. A crash can leave the `.tmp` behind but never a half-written
/// target, so readers always see either the old or the new complete file.
///
/// Both the write and the rename retry transient locks, so this is safe for
/// AV-scanned targets - including freshly written `.exe`s (the uninstaller),
/// which Defender locks the instant they're created.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    write_bytes_retry(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    rename_retry(&tmp, path).with_context(|| format!("commit {}", path.display()))?;
    Ok(())
}

pub fn collect_files(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(files)
}

/// Invoke hdiffz.exe (must be next to the current exe) to produce a binary patch.
pub fn generate_patch(old_file: &Path, new_file: &Path, out_file: &Path) -> Result<bool> {
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().context("failed to get exe dir")?;
    let hdiffz_path = exe_dir.join("hdiffz.exe");

    let status = match Command::new(&hdiffz_path)
        .arg(old_file)
        .arg(new_file)
        .arg(out_file)
        .arg("-c-zstd-21")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // hdiffz.exe not installed - caller falls back to shipping full file.
            return Ok(false);
        }
        Err(e) => return Err(e).with_context(|| format!("execute {}", hdiffz_path.display())),
    };

    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_is_consistent() {
        assert_eq!(bytes_blake3(b"abc"), bytes_blake3(b"abc"));
        assert_ne!(bytes_blake3(b"abc"), bytes_blake3(b"abd"));
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, b"abc").unwrap();
        assert_eq!(file_blake3(&p).unwrap(), bytes_blake3(b"abc"));
    }

    #[test]
    fn write_atomic_overwrites_no_tmp_leftover() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        write_atomic(&p, b"one").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"one");
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let tmp_left = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().map_or(false, |x| x == "tmp"));
        assert!(!tmp_left, "no .tmp should remain");
    }

    #[test]
    fn rename_retry_moves_and_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        let dest = d.path().join("sub").join("dest"); // parent created on demand
        fs::write(&src, b"new").unwrap();
        rename_retry(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!src.exists());

        // Overwrite an existing dest.
        let src2 = d.path().join("src2");
        fs::write(&src2, b"newer").unwrap();
        rename_retry(&src2, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"newer");
    }

    #[test]
    fn copy_retry_copies_and_creates_parent() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.exe");
        fs::write(&src, b"binary").unwrap();
        let dest = d.path().join("scratch").join("out.exe"); // parent created
        copy_retry(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"binary");
        assert!(src.exists()); // copy, not move
    }

    #[test]
    fn remove_file_retry_ok_and_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, b"x").unwrap();
        remove_file_retry(&p).unwrap();
        assert!(!p.exists());
        // Already gone -> still Ok.
        remove_file_retry(&p).unwrap();
    }

    #[test]
    fn collect_files_relative_and_normalized() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("a").join("b")).unwrap();
        fs::write(d.path().join("a").join("b").join("c.txt"), b"x").unwrap();
        fs::write(d.path().join("root.txt"), b"y").unwrap();
        let mut got = collect_files(d.path()).unwrap();
        got.sort();
        assert_eq!(got, vec!["a/b/c.txt".to_string(), "root.txt".to_string()]);
    }
}
