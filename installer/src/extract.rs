// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, bail};
use common::models::{InstallerPayload, Manifest, PayloadKind};
use hdiffpatch_rs::patchers::HDiff;
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zip::ZipArchive;

const FULL_PREFIX: &str = "full/";

/// A patch was run against the wrong installed version. The install was not
/// modified.
#[derive(Debug)]
pub struct VersionMismatch {
    pub expected_from: String,
    pub found: String,
    pub to_version: String,
}

impl std::fmt::Display for VersionMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let found = if self.found.is_empty() {
            "no version".to_string()
        } else {
            format!("version {}", self.found)
        };
        write!(
            f,
            "This update applies to version {}, but {} is installed. \
             Run the full {} installer instead.",
            self.expected_from, found, self.to_version
        )
    }
}

impl std::error::Error for VersionMismatch {}

/// Reject manifest paths that could escape the install directory. Defense in
/// depth behind the Ed25519 signature: only plain, relative components allowed.
fn safe_rel(rel: &str) -> Result<()> {
    if rel.is_empty() {
        bail!("empty path in manifest");
    }
    let p = Path::new(rel);
    if p.is_absolute() || rel.contains(':') {
        bail!("unsafe absolute path in manifest: {}", rel);
    }
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            _ => bail!("unsafe path component in manifest: {}", rel),
        }
    }
    Ok(())
}

/// Prefix `\\?\` to lift the 260-char `MAX_PATH` limit. Requires an absolute,
/// backslash-only path, so we normalize first. No-op if already prefixed or
/// unresolvable.
fn long_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s.starts_with(r"\\?\") {
        return p.to_path_buf();
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(p),
            Err(_) => return p.to_path_buf(),
        }
    };
    let norm = abs.to_string_lossy().replace('/', "\\");
    if norm.starts_with(r"\\") {
        // UNC: \\server\share -> \\?\UNC\server\share
        PathBuf::from(format!(r"\\?\UNC\{}", norm.trim_start_matches('\\')))
    } else {
        PathBuf::from(format!(r"\\?\{}", norm))
    }
}

/// Turn an IO error into a user-friendly message, calling out a full disk.
fn io_msg(action: &str, path: &Path, e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    // ERROR_DISK_FULL = 112, ERROR_HANDLE_DISK_FULL = 39.
    let raw = e.raw_os_error().unwrap_or(0);
    if e.kind() == ErrorKind::StorageFull || raw == 112 || raw == 39 {
        format!(
            "The disk became full while {} {}. Free up space and try again.",
            action,
            path.display()
        )
    } else {
        format!("Failed {} {}: {}", action, path.display(), e)
    }
}

pub struct InstallCtx<'a> {
    pub install_dir: PathBuf,
    pub payload: &'a InstallerPayload,
    pub zip_bytes: &'a [u8],
    pub cancel: Arc<AtomicBool>,
    pub on_progress: Arc<dyn Fn(u64, u64, &str) + Send + Sync>,
}

pub fn install(ctx: InstallCtx<'_>) -> Result<()> {
    let manifest = &ctx.payload.manifest;

    // Log to %TEMP% so diagnostics survive when the install dir isn't writable.
    common::log::init(common::log::log_path_installer_temp(
        &ctx.payload.product,
        std::process::id(),
    ));
    common::log::prune_temp_logs(&ctx.payload.product, 14);
    let started = std::time::Instant::now();
    common::log::info(format!(
        "install start: product={} version={} kind={:?} install_dir={}",
        ctx.payload.product,
        ctx.payload.to_version,
        ctx.payload.kind,
        ctx.install_dir.display()
    ));
    common::log::info(format!(
        "payload {} bytes, {} files, deleted {}",
        ctx.zip_bytes.len(),
        manifest.files.len(),
        manifest.deleted_files.len()
    ));

    // Single-instance lock per install dir, so two runs can't race on the temp
    // dirs. OS frees it on exit or crash.
    let _install_lock = acquire_install_lock(&ctx.install_dir)?;

    if ctx.payload.force_reinstall {
        common::log::info("force_reinstall set: skipping version check, reinstalling from scratch");
    }

    if ctx.payload.kind == PayloadKind::Patch && !ctx.payload.force_reinstall {
        let expected_from = ctx
            .payload
            .from_version
            .as_deref()
            .context("patch payload missing from_version")?;
        // Current version lives in the per-user data dir (not the app folder).
        let current = data_dir_of(ctx.payload).and_then(|d| read_local_version(&d));
        let current_ref = current.as_deref().unwrap_or("");
        if current_ref != expected_from {
            common::log::error(format!(
                "patch refused: expected from_version={} found={}",
                expected_from, current_ref
            ));
            // Pre-flight refusal, nothing touched. Typed error so the caller
            // can return a distinct exit code.
            return Err(anyhow::Error::new(VersionMismatch {
                expected_from: expected_from.to_string(),
                found: current_ref.to_string(),
                to_version: ctx.payload.to_version.clone(),
            }));
        }
    }

    check_writable(&ctx.install_dir)?;

    check_disk_space(&ctx.install_dir, manifest, ctx.payload.kind)?;

    // Close any running copy of the target app first (WM_CLOSE, never killed).
    {
        let pcb = ctx.on_progress.clone();
        crate::proc::ensure_closed(
            &ctx.install_dir,
            &manifest.exe,
            &ctx.cancel,
            &move |msg| pcb(0, 0, msg),
        )?;
    }

    let temp_dir = ctx.install_dir.join(".installer_tmp");

    // Roll back a commit interrupted by a previous run before doing anything.
    recover_if_interrupted(&temp_dir, &ctx.install_dir);

    // Fresh staging + backup areas. A leftover temp with no journal means a
    // previous run was interrupted during staging (live install untouched), so
    // discard it and start over; correct files are hash-skipped below.
    let staged_dir = temp_dir.join("staged");
    let backup_dir = temp_dir.join("backup");
    if temp_dir.exists() {
        common::log::warn("discarding leftover staging from a previous incomplete run");
    }
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&staged_dir).context("create staging dir")?;
    fs::create_dir_all(&backup_dir).context("create backup dir")?;

    let total_bytes: u64 = manifest.files.values().map(|e| e.size).sum();
    let done = Arc::new(AtomicU64::new(0));

    // Validate the embedded zip up front (clean error if corrupt) before the
    // parallel workers each open their own view of it.
    ZipArchive::new(Cursor::new(ctx.zip_bytes)).context("open embedded zip")?;

    // Deterministic order - easier UX and reproducible.
    let mut entries: Vec<(&String, &common::models::FileEntry)> =
        manifest.files.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    // ---- PHASE 1: STAGE (parallel) ------------------------------------
    // Build every new/changed file in `staged/`, verified by hash. The live
    // install is not touched, so cancelling/crashing here leaves it intact.
    // Files are independent, so staging fans out across cores; `map_init` gives
    // each worker its own `ZipArchive` view over the shared mmap slice (one
    // central-directory parse per core, not per file).
    let staged: Vec<Result<Option<String>>> = entries
        .par_iter()
        .map_init(
            || ZipArchive::new(Cursor::new(ctx.zip_bytes)),
            |archive, &(rel, entry)| -> Result<Option<String>> {
                if ctx.cancel.load(Ordering::Relaxed) {
                    bail!("cancelled by user");
                }

                safe_rel(rel).inspect_err(|e| {
                    common::log::error(format!("rejected path: {e:#}"));
                })?;

                let dest = long_path(&ctx.install_dir.join(rel));
                (ctx.on_progress)(done.load(Ordering::Relaxed), total_bytes, rel);

                // Hash-skip if already correct (disabled in force_reinstall).
                if dest.exists() && !ctx.payload.force_reinstall {
                    if let Ok(h) = hash_file(&dest) {
                        if h == entry.hash {
                            common::log::info(format!("skip (hash match): {}", rel));
                            done.fetch_add(entry.size, Ordering::Relaxed);
                            (ctx.on_progress)(done.load(Ordering::Relaxed), total_bytes, rel);
                            return Ok(None);
                        }
                    }
                }

                let archive = archive
                    .as_mut()
                    .map_err(|e| anyhow::anyhow!("open embedded zip: {e}"))?;
                let staged_path = staged_dir.join(staged_name(rel));
                stage_file(archive, ctx.payload.kind, rel, entry, &dest, &staged_path)?;

                done.fetch_add(entry.size, Ordering::Relaxed);
                (ctx.on_progress)(done.load(Ordering::Relaxed), total_bytes, rel);
                Ok(Some(rel.clone()))
            },
        )
        .collect();

    // Surface the first staging error (cancel included); live install untouched.
    let mut to_commit: Vec<String> = Vec::new();
    for r in staged {
        match r {
            Ok(Some(rel)) => to_commit.push(rel),
            Ok(None) => {}
            Err(e) => {
                common::log::warn(format!("staging failed/cancelled: {e:#}"));
                cleanup(&temp_dir);
                return Err(e);
            }
        }
    }
    to_commit.sort(); // parallel completion order is nondeterministic

    // ---- PHASE 2: COMMIT ----------------------------------------------
    // Swap staged files into place. A journal records every touched path so an
    // interruption can be rolled back.
    let mut deleted: Vec<String> = Vec::new();
    for rel in &manifest.deleted_files {
        if safe_rel(rel).is_err() {
            common::log::warn(format!("skipping unsafe deleted_files entry: {}", rel));
            continue;
        }
        if long_path(&ctx.install_dir.join(rel)).exists() {
            deleted.push(rel.clone());
        }
    }

    // force_reinstall: also remove existing files not in this build (clean
    // slate). Backed up like any delete, so still rollback-safe.
    if ctx.payload.force_reinstall {
        if let Ok(existing) = common::utils::collect_files(&ctx.install_dir) {
            for rel in existing {
                if rel.starts_with(".installer_tmp")
                    || manifest.files.contains_key(&rel)
                    || deleted.contains(&rel)
                    || safe_rel(&rel).is_err()
                {
                    continue;
                }
                common::log::info(format!("force_reinstall: removing orphan {}", rel));
                deleted.push(rel);
            }
        }
    }

    if to_commit.is_empty() && deleted.is_empty() {
        common::log::info("nothing to commit (already up to date)");
    } else {
        common::log::info(format!(
            "committing {} file(s), deleting {}",
            to_commit.len(),
            deleted.len()
        ));
        (ctx.on_progress)(total_bytes, total_bytes, "Finalizing...");
        write_journal(&temp_dir, &to_commit, &deleted)?;

        let commit_result = (|| -> Result<()> {
            for rel in &to_commit {
                commit_one(&ctx.install_dir, &staged_dir, &backup_dir, rel)?;
            }
            for rel in &deleted {
                backup_then_remove(&ctx.install_dir, &backup_dir, rel)?;
            }
            Ok(())
        })();

        if let Err(e) = commit_result {
            common::log::error(format!("commit failed: {e:#} - rolling back"));
            rollback(&temp_dir, &ctx.install_dir, &to_commit, &deleted);
            cleanup(&temp_dir);
            return Err(e).context("install failed and was rolled back");
        }

        // Re-read each committed file from disk to catch corruption from the
        // write/rename itself (bad sector, FS glitch). Still inside the
        // transaction, backups intact.
        (ctx.on_progress)(total_bytes, total_bytes, "Verifying...");
        let mut corrupt = find_corrupt(&ctx.install_dir, manifest, &to_commit);

        // Repair before a full rollback: corrupt content is reproducible from
        // the payload, and rewriting to a fresh location dodges transient
        // glitches. Backups stay untouched so rollback remains possible.
        if !corrupt.is_empty() {
            common::log::warn(format!(
                "{} file(s) failed post-install verification - attempting repair from payload",
                corrupt.len()
            ));
            for attempt in 1..=VERIFY_REPAIR_ATTEMPTS {
                (ctx.on_progress)(total_bytes, total_bytes, "Repairing...");
                let repair = repair_corrupt(
                    ctx.zip_bytes,
                    ctx.payload.kind,
                    manifest,
                    &staged_dir,
                    &backup_dir,
                    &ctx.install_dir,
                    &corrupt,
                );
                if let Err(e) = repair {
                    common::log::error(format!("repair attempt {} failed: {e:#}", attempt));
                    break;
                }
                corrupt = find_corrupt(&ctx.install_dir, manifest, &corrupt);
                if corrupt.is_empty() {
                    common::log::info(format!("repair succeeded on attempt {}", attempt));
                    break;
                }
                common::log::warn(format!(
                    "{} file(s) still corrupt after repair attempt {}",
                    corrupt.len(),
                    attempt
                ));
            }
        }

        // Repair exhausted and still corrupt - roll back to the previous version.
        if !corrupt.is_empty() {
            common::log::error(format!(
                "post-install verification failed for {} file(s) after repair - rolling back",
                corrupt.len()
            ));
            rollback(&temp_dir, &ctx.install_dir, &to_commit, &deleted);
            cleanup(&temp_dir);
            bail!(
                "{} installed file(s) failed verification and could not be repaired; \
                 the install was rolled back to the previous version",
                corrupt.len()
            );
        }
        common::log::info(format!("verified {} committed file(s)", to_commit.len()));

        // Verified - drop the journal so recovery won't fire.
        let _ = fs::remove_file(journal_path(&temp_dir));
    }

    // Installer metadata is written to the per-user data dir by
    // `install::finalize`, not into the app folder.
    cleanup(&temp_dir);

    common::log::info(format!(
        "install complete in {}ms",
        started.elapsed().as_millis()
    ));

    (ctx.on_progress)(total_bytes, total_bytes, "done");
    Ok(())
}

/// Build the final content for `rel` into `staged_path`, verified by BLAKE3.
/// Tries an in-place patch (against the existing `old` file) first, falls back
/// to the full file from the zip. Does not touch `old`.
///
/// `old` is normally the live install target (staging), but the repair path
/// passes the *backup* copy of the previous version instead - both are valid
/// patch inputs, since the patch was diffed against that previous version.
fn stage_file(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    kind: PayloadKind,
    rel: &str,
    entry: &common::models::FileEntry,
    old: &Path,
    staged_path: &Path,
) -> Result<()> {
    // Patch path: apply hdiff(old, patch) → staged_path.
    if kind == PayloadKind::Patch {
        if let Some(patch_info) = &entry.patch {
            if old.exists() {
                // The builder always writes `patch_info.file` already prefixed
                // with `patches/` (see installer_builder::pack), so it can be
                // used as the in-zip path as-is.
                let patch_rel = &patch_info.file;
                if let Ok(patch_bytes) = read_from_zip(archive, patch_rel) {
                    let patch_tmp = staged_path.with_extension("patch");
                    if fs::write(&patch_tmp, &patch_bytes).is_ok() {
                        let ok = run_hdiff(old, &patch_tmp, staged_path);
                        let _ = fs::remove_file(&patch_tmp);
                        if ok && hash_file(staged_path).ok().as_deref() == Some(&entry.hash) {
                            common::log::info(format!("staged (patch): {}", rel));
                            return Ok(());
                        }
                        common::log::warn(format!(
                            "patch unusable, falling back to full: {}",
                            rel
                        ));
                        let _ = fs::remove_file(staged_path);
                    }
                }
            }
        }
    }

    // Full file from zip, streamed in ~1 MB chunks and hashed inline.
    let zip_rel = format!("{}{}", FULL_PREFIX, rel);
    let mut entry_rdr = archive
        .by_name(&zip_rel)
        .with_context(|| format!("{} not in zip", zip_rel))?;
    let mut out = File::create(staged_path)
        .map_err(|e| anyhow::anyhow!("{}", io_msg("creating", staged_path, &e)))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = entry_rdr
            .read(&mut buf)
            .with_context(|| format!("read {} from embedded zip", zip_rel))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| anyhow::anyhow!("{}", io_msg("writing", staged_path, &e)))?;
    }
    drop(out);
    let actual = hasher.finalize().to_hex().to_string();
    if actual != entry.hash {
        common::log::error(format!(
            "zip vs manifest hash mismatch: {} (zip={} manifest={})",
            rel, actual, entry.hash
        ));
        let _ = fs::remove_file(staged_path);
        bail!("hash mismatch for {} (zip vs manifest)", rel);
    }
    common::log::info(format!("staged (full): {} ({} bytes)", rel, entry.size));
    Ok(())
}

/// RAII single-instance lock for one install dir, backed by a named mutex. The
/// OS destroys it when the last handle closes (exit or crash), so it can never
/// go stale.
struct InstallLock(windows::Win32::Foundation::HANDLE);

impl Drop for InstallLock {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn acquire_install_lock(install_dir: &Path) -> Result<InstallLock> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    // Normalize the path so different spellings of the same dir collide.
    let key = install_dir.to_string_lossy().to_lowercase().replace('/', "\\");
    let hash = blake3::hash(key.as_bytes()).to_hex();
    // Local\ namespace = per-session, which matches our per-user installs.
    let name = format!("Local\\Installway-Install-{}", &hash.as_str()[..32]);
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr()))
            .context("create install lock mutex")?;
        // Read last error immediately, before any other Win32 call clobbers it.
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        if handle.is_invalid() {
            bail!("could not create install lock");
        }
        if already {
            let _ = CloseHandle(handle);
            common::log::warn("refused: another installer is already running for this folder");
            bail!("Another installation for this folder is already in progress.");
        }
        Ok(InstallLock(handle))
    }
}

/// Pre-flight: make sure we can create the install dir and write into it.
/// Catches "user picked C:\Program Files" (needs admin) up front with a clear
/// message instead of a mid-install permission error.
fn check_writable(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| {
        common::log::error(format!("cannot create {}: {}", dir.display(), e));
        anyhow::anyhow!(
            "Cannot create the install folder:\n{}\n\nChoose a folder you can write to (e.g. under your user folder). ({})",
            dir.display(),
            e
        )
    })?;
    let probe = long_path(&dir.join(".write_test"));
    match File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            common::log::error(format!("not writable: {} ({})", dir.display(), e));
            bail!(
                "No permission to write to:\n{}\n\nThis location may require administrator rights. Choose another folder (e.g. under your user folder). ({})",
                dir.display(),
                e
            )
        }
    }
}

/// Safety margin on top of the estimated payload size.
const SPACE_BUFFER: u64 = 100 * 1024 * 1024; // 100 MB

/// Verify the install volume has enough free space before writing anything.
///
/// Peak extra space = total install size + buffer, for both full and patch:
/// staging writes the reconstructed full content of every changed file (a patch
/// stages the full file, not the small blob), and commit only renames in-place.
fn check_disk_space(install_dir: &Path, manifest: &Manifest, kind: PayloadKind) -> Result<()> {
    let total_file_bytes: u64 = manifest.files.values().map(|e| e.size).sum();
    let required = total_file_bytes.saturating_add(SPACE_BUFFER);

    let available = fs4::available_space(install_dir)
        .with_context(|| format!("query free space on {}", install_dir.display()))?;

    common::log::info(format!(
        "disk space: required ~{} ({}, staged worst-case), available {} on {}",
        human_bytes(required),
        match kind {
            PayloadKind::Full => "full",
            PayloadKind::Patch => "patch",
        },
        human_bytes(available),
        install_dir.display()
    ));

    if available < required {
        common::log::error(format!(
            "insufficient disk space: need {} but only {} free",
            human_bytes(required),
            human_bytes(available)
        ));
        bail!(
            "Not enough disk space. Need about {} free on the install drive, but only {} is available.",
            human_bytes(required),
            human_bytes(available)
        );
    }
    Ok(())
}

fn human_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if b >= GB {
        format!("{:.2} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1} MB", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.1} KB", b as f64 / KB as f64)
    } else {
        format!("{} B", b)
    }
}

fn read_from_zip(archive: &mut ZipArchive<Cursor<&[u8]>>, rel: &str) -> Result<Vec<u8>> {
    let mut f = archive.by_name(rel).with_context(|| format!("{} not in zip", rel))?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

fn run_hdiff(old: &Path, patch: &Path, out: &Path) -> bool {
    let old_s = old.to_string_lossy().to_string();
    let patch_s = patch.to_string_lossy().to_string();
    let out_s = out.to_string_lossy().to_string();
    let mut p = HDiff::new(old_s, patch_s, out_s);
    p.apply()
}

fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

// ---- Two-phase commit primitives --------------------------------------

/// Flat, collision-free staged/backup file name for a relative path.
fn staged_name(rel: &str) -> String {
    blake3::hash(rel.as_bytes()).to_hex().to_string()
}

fn journal_path(temp_dir: &Path) -> PathBuf {
    temp_dir.join("commit.journal")
}

/// Move with retry, to survive transient locks (AV/Explorer/indexer).
fn move_retry(src: &Path, dest: &Path) -> Result<()> {
    common::utils::rename_retry(src, dest)
}

/// Back up the existing file (if any) then move the staged file into place.
fn commit_one(install_dir: &Path, staged_dir: &Path, backup_dir: &Path, rel: &str) -> Result<()> {
    let dest = long_path(&install_dir.join(rel));
    let staged = long_path(&staged_dir.join(staged_name(rel)));
    if dest.exists() {
        let backup = long_path(&backup_dir.join(staged_name(rel)));
        move_retry(&dest, &backup)
            .with_context(|| format!("backup {} before overwrite", rel))?;
    }
    move_retry(&staged, &dest).with_context(|| format!("install {}", rel))?;
    Ok(())
}

/// Back up then remove an obsolete file (so rollback can restore it).
fn backup_then_remove(install_dir: &Path, backup_dir: &Path, rel: &str) -> Result<()> {
    let dest = long_path(&install_dir.join(rel));
    if dest.exists() {
        let backup = long_path(&backup_dir.join(staged_name(rel)));
        move_retry(&dest, &backup).with_context(|| format!("backup {} before delete", rel))?;
    }
    Ok(())
}

/// Record every path the commit will touch, so an interrupted commit can be
/// rolled back on the next launch.
fn write_journal(temp_dir: &Path, adds: &[String], deletes: &[String]) -> Result<()> {
    let mut lines = Vec::with_capacity(adds.len() + deletes.len());
    for r in adds {
        lines.push(r.as_str());
    }
    for r in deletes {
        lines.push(r.as_str());
    }
    // Write to .tmp then rename so the journal itself appears atomically.
    let jp = journal_path(temp_dir);
    let tmp = jp.with_extension("journal.tmp");
    fs::write(&tmp, lines.join("\n")).context("write journal")?;
    fs::rename(&tmp, &jp).context("commit journal")?;
    Ok(())
}

/// Roll the live install back to its pre-commit state using the backups.
/// For each touched path: if a backup exists restore it, else the path was
/// newly added so remove it.
fn rollback(temp_dir: &Path, install_dir: &Path, adds: &[String], deletes: &[String]) {
    let backup_dir = temp_dir.join("backup");
    let restore = |rel: &str| {
        let dest = long_path(&install_dir.join(rel));
        let backup = long_path(&backup_dir.join(staged_name(rel)));
        if backup.exists() {
            if let Err(e) = move_retry(&backup, &dest) {
                common::log::error(format!("rollback restore failed for {}: {e:#}", rel));
            }
        } else {
            // Newly added file with no prior version - remove it.
            let _ = fs::remove_file(&dest);
        }
    };
    for rel in adds {
        restore(rel);
    }
    for rel in deletes {
        restore(rel);
    }
    common::log::warn("rolled back to pre-install state");
}

/// On startup: if a commit journal is present, a previous run was interrupted
/// mid-commit (e.g. power loss). Roll back to the pre-install state.
fn recover_if_interrupted(temp_dir: &Path, install_dir: &Path) {
    let jp = journal_path(temp_dir);
    let Ok(content) = fs::read_to_string(&jp) else {
        return;
    };
    common::log::warn("found interrupted commit journal - rolling back");
    let backup_dir = temp_dir.join("backup");
    for rel in content.lines().filter(|l| !l.trim().is_empty()) {
        // Ignore anything that wouldn't be a safe relative path.
        if safe_rel(rel).is_err() {
            continue;
        }
        let dest = long_path(&install_dir.join(rel));
        let backup = long_path(&backup_dir.join(staged_name(rel)));
        if backup.exists() {
            let _ = move_retry(&backup, &dest);
        } else {
            let _ = fs::remove_file(&dest);
        }
    }
    let _ = fs::remove_dir_all(temp_dir);
    common::log::warn("recovery complete: install rolled back to previous state");
}

/// Per-user data dir for this payload (where version.json / manifest / info /
/// uninstall.exe / log live). `None` only if %LOCALAPPDATA% can't be resolved.
fn data_dir_of(payload: &InstallerPayload) -> Option<PathBuf> {
    common::paths::uninstall_dir(&payload.publisher, &payload.product)
}

/// Read the recorded installed version from `version.json` in the data dir.
fn read_local_version(data_dir: &Path) -> Option<String> {
    let s = fs::read_to_string(data_dir.join("version.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v["version"].as_str().map(|s| s.to_string())
}

fn cleanup(temp_dir: &Path) {
    let _ = common::utils::remove_dir_retry(temp_dir);
}

/// Re-hash each committed file and return those that don't match the manifest
/// (corrupt, missing, or unreadable). Parallel; used inside the transaction.
fn find_corrupt(install_dir: &Path, manifest: &Manifest, committed: &[String]) -> Vec<String> {
    committed
        .par_iter()
        .filter(|rel| {
            let Some(entry) = manifest.files.get(*rel) else {
                return false;
            };
            let path = long_path(&install_dir.join(rel));
            match hash_file(&path) {
                Ok(got) if got == entry.hash => false,
                Ok(got) => {
                    common::log::warn(format!(
                        "{} corrupt after writing (expected {}, got {})",
                        rel,
                        &entry.hash[..16.min(entry.hash.len())],
                        &got[..16.min(got.len())]
                    ));
                    true
                }
                Err(e) => {
                    common::log::warn(format!("{} unreadable after writing: {e:#}", rel));
                    true
                }
            }
        })
        .cloned()
        .collect()
}

/// Repair passes over the corrupt set before falling back to a full rollback.
const VERIFY_REPAIR_ATTEMPTS: usize = 2;

/// Re-stage each corrupt file from the payload and move it back into place,
/// leaving the backups intact so rollback stays possible. Patch entries are
/// re-applied against the backed-up previous version; full/new files come from
/// `full/<rel>` in the zip.
fn repair_corrupt(
    zip_bytes: &[u8],
    kind: PayloadKind,
    manifest: &Manifest,
    staged_dir: &Path,
    backup_dir: &Path,
    install_dir: &Path,
    corrupt: &[String],
) -> Result<()> {
    corrupt
        .par_iter()
        .map_init(
            || ZipArchive::new(Cursor::new(zip_bytes)),
            |archive, rel| -> Result<()> {
                let Some(entry) = manifest.files.get(rel) else {
                    return Ok(());
                };
                let archive = archive
                    .as_mut()
                    .map_err(|e| anyhow::anyhow!("open embedded zip: {e}"))?;

                // Patch input = the backup from commit time. Absent for new
                // files, which ship in full and fall through to the zip.
                let old = long_path(&backup_dir.join(staged_name(rel)));
                let staged_path = staged_dir.join(staged_name(rel));
                let _ = fs::remove_file(&staged_path);
                stage_file(archive, kind, rel, entry, &old, &staged_path)
                    .with_context(|| format!("re-stage {} for repair", rel))?;

                // Overwrite the corrupt file; backup left intact for rollback.
                let dest = long_path(&install_dir.join(rel));
                let staged = long_path(&staged_path);
                move_retry(&staged, &dest)
                    .with_context(|| format!("repair-install {}", rel))?;
                common::log::info(format!("repaired {}", rel));
                Ok(())
            },
        )
        .collect::<Result<Vec<()>>>()
        .map(|_| ())
}

/// Diagnostic: re-hash every installed file and report missing / corrupted
/// files. `data_dir` holds the manifest + info (per-user data dir); the actual
/// files are checked under `info.install_dir` (the app folder). Returns `Err`
/// if anything is missing or corrupt (exit code 1 for scripts).
pub fn verify_install(data_dir: &Path) -> Result<()> {
    let info_path = data_dir.join("installer_info.json");
    let info_data = fs::read_to_string(&info_path).with_context(|| {
        format!("read {} - is this product installed?", info_path.display())
    })?;
    let info: common::models::InstallInfo =
        serde_json::from_str(&info_data).context("parse installer_info.json")?;

    let manifest_path = data_dir.join("installer_manifest.json");
    let mdata = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&mdata).context("parse installer_manifest.json")?;

    let app_dir = PathBuf::from(&info.install_dir);

    let mut rels: Vec<(&String, &common::models::FileEntry)> = manifest.files.iter().collect();
    rels.sort_by(|a, b| a.0.cmp(b.0));

    let mut missing = 0usize;
    let mut corrupt = 0usize;
    let mut ok = 0usize;

    for (rel, entry) in rels {
        if safe_rel(rel).is_err() {
            println!("SKIP  {} (unsafe path)", rel);
            continue;
        }
        let path = long_path(&app_dir.join(rel));
        if !path.exists() {
            println!("MISSING  {}", rel);
            missing += 1;
            continue;
        }
        match hash_file(&path) {
            Ok(h) if h == entry.hash => ok += 1,
            Ok(_) => {
                println!("CORRUPT  {}", rel);
                corrupt += 1;
            }
            Err(e) => {
                println!("UNREADABLE  {} ({})", rel, e);
                corrupt += 1;
            }
        }
    }

    println!(
        "verify {}: {} OK, {} missing, {} corrupt (version {})",
        app_dir.display(),
        ok,
        missing,
        corrupt,
        manifest.version
    );

    if missing == 0 && corrupt == 0 {
        Ok(())
    } else {
        bail!(
            "verification failed: {} missing, {} corrupt - reinstall or repair",
            missing,
            corrupt
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_accepts_and_rejects() {
        assert!(safe_rel("bin/app.exe").is_ok());
        assert!(safe_rel("a/b/c.txt").is_ok());
        assert!(safe_rel("").is_err());
        assert!(safe_rel("../x").is_err());
        assert!(safe_rel("a/../b").is_err());
        assert!(safe_rel("/abs").is_err());
        assert!(safe_rel("C:/x").is_err());
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert!(human_bytes(1024 * 1024).ends_with("MB"));
        assert!(human_bytes(2 * 1024 * 1024 * 1024).ends_with("GB"));
    }

    #[test]
    fn io_msg_flags_disk_full() {
        let full = std::io::Error::from_raw_os_error(112);
        assert!(io_msg("writing", Path::new("x"), &full).contains("disk"));
        let other = std::io::Error::new(std::io::ErrorKind::Other, "boom");
        assert!(io_msg("writing", Path::new("x"), &other).contains("Failed"));
    }

    #[test]
    fn staged_name_is_stable_and_distinct() {
        assert_eq!(staged_name("a/b.txt"), staged_name("a/b.txt"));
        assert_ne!(staged_name("a"), staged_name("b"));
    }

    // Power-loss recovery: a commit interrupted with a journal present must
    // roll back to the pre-install state on the next launch.
    #[test]
    fn recover_rolls_back_from_journal() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        let temp = app.join(".installer_tmp");
        let backup = temp.join("backup");
        fs::create_dir_all(&backup).unwrap();

        // foo.txt: an existing file that was overwritten -> backup holds the old.
        fs::write(app.join("foo.txt"), b"NEW").unwrap();
        fs::write(backup.join(staged_name("foo.txt")), b"OLD").unwrap();
        // bar.txt: a brand-new file (no backup) -> must be removed.
        fs::write(app.join("bar.txt"), b"NEWBAR").unwrap();

        fs::write(journal_path(&temp), "foo.txt\nbar.txt\n").unwrap();

        recover_if_interrupted(&temp, &app);

        assert_eq!(fs::read(app.join("foo.txt")).unwrap(), b"OLD"); // restored
        assert!(!app.join("bar.txt").exists()); // new file removed
        assert!(!temp.exists()); // temp cleaned
    }

    // Build a one-entry payload zip with `full/<rel>` = `content`, the way the
    // installer expects to read it back.
    fn full_zip(rel: &str, content: &[u8]) -> Vec<u8> {
        use zip::write::SimpleFileOptions;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            format!("{}{}", FULL_PREFIX, rel),
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, content).unwrap();
        zip.finish().unwrap().into_inner()
    }

    // A file that verifies as corrupt after commit is rewritten from the payload
    // (full file in the zip) instead of triggering a rollback.
    #[test]
    fn repair_rewrites_corrupt_file_from_payload() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        let temp = app.join(".installer_tmp");
        let staged = temp.join("staged");
        let backup = temp.join("backup");
        fs::create_dir_all(&staged).unwrap();
        fs::create_dir_all(&backup).unwrap();

        let good = b"GOOD-CONTENT";
        let zip = full_zip("foo.txt", good);

        // Committed file landed corrupt; manifest expects the good hash.
        fs::write(app.join("foo.txt"), b"CORRUPTED").unwrap();
        let mut files = std::collections::HashMap::new();
        files.insert(
            "foo.txt".to_string(),
            common::models::FileEntry {
                hash: bytes_hash(good),
                size: good.len() as u64,
                patch: None,
            },
        );
        let manifest = Manifest {
            version: "1".into(),
            exe: String::new(),
            files,
            deleted_files: Vec::new(),
            full_size: good.len() as u64,
            total_patch_size: 0,
        };

        let corrupt = find_corrupt(&app, &manifest, &["foo.txt".to_string()]);
        assert_eq!(corrupt, vec!["foo.txt".to_string()]);

        repair_corrupt(&zip, PayloadKind::Full, &manifest, &staged, &backup, &app, &corrupt)
            .unwrap();

        assert!(find_corrupt(&app, &manifest, &["foo.txt".to_string()]).is_empty());
        assert_eq!(fs::read(app.join("foo.txt")).unwrap(), good);
    }

    fn bytes_hash(b: &[u8]) -> String {
        blake3::hash(b).to_hex().to_string()
    }
}

