// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use anyhow::{Context, Result, bail};
use common::model::installer_payload::InstallerPayload;
use common::model::manifest::Manifest;
use common::model::payload_kind::PayloadKind;
use hdiffpatch_rs::patchers::HDiff;
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use zip::ZipArchive;

const FULL_PREFIX: &str = "full/";

/// The install dir is not writable due to OS permissions; elevation may help.
#[derive(Debug)]
pub struct PermissionDeniedError;

impl std::fmt::Display for PermissionDeniedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("permission denied")
    }
}

impl std::error::Error for PermissionDeniedError {}

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
    pub on_progress: common::ProgressFn,
    /// Collected plugin-page answers, routed per plugin name. Empty when no UI
    /// plugin contributed pages. Passed to each plugin's `up` via `inputs_json`.
    pub plugin_inputs: common::plugin::InputsByPlugin,
    /// Machine-wide install (shared location, or the elevated worker). Selects
    /// the data dir plugins see (`%ProgramData%` vs `%LOCALAPPDATA%`) and the
    /// lock namespace, matching where `install::finalize` writes the metadata.
    pub requires_admin: bool,
    /// Installer window handle (as isize) for the blocking-process TaskDialog.
    /// Pass 0 for headless / elevated-worker paths (triggers silent force-kill).
    pub hwnd_parent: isize,
    /// UI language for the blocking-process dialog strings.
    pub translator: common::i18n::Translator,
}

pub fn install(ctx: InstallCtx<'_>) -> Result<()> {
    let manifest = &ctx.payload.manifest;

    // Log to %TEMP% so diagnostics survive when the install dir isn't writable.
    // Named by product_id (filesystem-safe, stable across versions).
    common::log::init(common::log::log_path_installer_temp(
        &ctx.payload.product_id,
        std::process::id(),
    ));
    common::log::prune_temp_logs(&ctx.payload.product_id, 14);
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
    let _install_lock = acquire_install_lock(&ctx.install_dir, ctx.requires_admin)?;

    if ctx.payload.force_reinstall {
        common::log::info("force_reinstall set: skipping version check, reinstalling from scratch");
    }

    if ctx.payload.kind == PayloadKind::Patch && !ctx.payload.force_reinstall {
        let expected_from = ctx
            .payload
            .from_version
            .as_deref()
            .context("patch payload missing from_version")?;
        // Current version lives in the data dir (not the app folder), which is
        // machine-wide or per-user depending on how it was installed.
        let current = installed_version(ctx.payload);
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

    // Close any process running from the install dir before writing files.
    {
        let pcb = ctx.on_progress.clone();
        crate::proc::ensure_closed(
            &ctx.install_dir,
            ctx.hwnd_parent,
            ctx.translator,
            &ctx.cancel,
            &move |msg| pcb(0, 0, msg),
        )?;
    }

    // Pre-install plugins run before any file is staged, so a required failure
    // aborts cleanly (live install untouched).
    run_zip_plugins(&ctx, common::model::plugin_phase::PluginPhase::PreInstall)?;

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
    let mut entries: Vec<(&String, &common::model::file_entry::FileEntry)> =
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
                if dest.exists()
                    && !ctx.payload.force_reinstall
                    && let Ok(h) = hash_file(&dest)
                    && h == entry.hash
                {
                    common::log::info(format!("skip (hash match): {}", rel));
                    done.fetch_add(entry.size, Ordering::Relaxed);
                    (ctx.on_progress)(done.load(Ordering::Relaxed), total_bytes, rel);
                    return Ok(None);
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

    // Clean slate: also remove existing files not in this build. Two triggers:
    //   - `force_reinstall` (dev flag), or
    //   - `purge_unknown_files` on a Full payload (opt-in at build time, so an
    //     upgrade/reinstall from a full version drops leftover unknown files).
    // Patches never purge: their manifest is incremental, so "unknown" files are
    // expected. Removals are backed up like any delete, so still rollback-safe.
    let purge_orphans = ctx.payload.force_reinstall
        || (ctx.payload.purge_unknown_files && ctx.payload.kind == PayloadKind::Full);
    if purge_orphans && let Ok(existing) = common::utils::collect_files(&ctx.install_dir) {
        for rel in existing {
            if rel.starts_with(".installer_tmp")
                || manifest.files.contains_key(&rel)
                || deleted.contains(&rel)
                || safe_rel(&rel).is_err()
            {
                continue;
            }
            common::log::info(format!("purge: removing unknown file {}", rel));
            deleted.push(rel);
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
                if ctx.cancel.load(Ordering::Relaxed) {
                    bail!("cancelled by user");
                }
                commit_one(&ctx.install_dir, &staged_dir, &backup_dir, rel)?;
            }
            for rel in &deleted {
                if ctx.cancel.load(Ordering::Relaxed) {
                    bail!("cancelled by user");
                }
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
        common::log::info(format!("verifying {} committed file(s)", to_commit.len()));
        let verify_started = Instant::now();
        let mut corrupt = find_corrupt(&ctx.install_dir, manifest, &to_commit, &ctx.cancel);
        common::log::info(format!(
            "verification finished in {:.1}s",
            verify_started.elapsed().as_secs_f64()
        ));

        // A cancel during the (potentially long) re-hash short-circuits the
        // remaining files inside `find_corrupt`; here we roll back so the live
        // install returns to its previous version rather than half-committed.
        if ctx.cancel.load(Ordering::Relaxed) {
            common::log::warn("cancelled by user during verification - rolling back");
            rollback(&temp_dir, &ctx.install_dir, &to_commit, &deleted);
            cleanup(&temp_dir);
            bail!("cancelled by user");
        }

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
                corrupt = find_corrupt(&ctx.install_dir, manifest, &corrupt, &ctx.cancel);
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

        if !deleted.is_empty() {
            common::utils::prune_empty_dirs(&long_path(&ctx.install_dir));
        }
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
    entry: &common::model::file_entry::FileEntry,
    old: &Path,
    staged_path: &Path,
) -> Result<()> {
    // Patch path: apply hdiff(old, patch) → staged_path.
    if kind == PayloadKind::Patch
        && let Some(patch_info) = &entry.patch
        && old.exists()
    {
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
                common::log::warn(format!("patch unusable, falling back to full: {}", rel));
                let _ = fs::remove_file(staged_path);
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

fn acquire_install_lock(install_dir: &Path, machine: bool) -> Result<InstallLock> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    // Normalize the path so different spellings of the same dir collide.
    let key = install_dir
        .to_string_lossy()
        .to_lowercase()
        .replace('/', "\\");
    let hash = blake3::hash(key.as_bytes()).to_hex();
    // Machine-wide installs use the Global\ namespace so two users can't race on
    // the same shared folder; per-user installs stay in the per-session Local\.
    let scope = if machine { "Global" } else { "Local" };
    let name = format!("{}\\Installway-Install-{}", scope, &hash.as_str()[..32]);
    let wide = common::utils::wide(&name);

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
/// Catches "user picked C:\Program Files" (needs admin) up front — returns
/// `PermissionDeniedError` so the UI can offer UAC elevation instead of a
/// plain error message.
pub(crate) fn check_writable(dir: &Path) -> Result<()> {
    if let Err(e) = fs::create_dir_all(dir) {
        common::log::error(format!("cannot create {}: {}", dir.display(), e));
        if common::elevation::is_permission_denied(&e) {
            return Err(anyhow::Error::new(PermissionDeniedError));
        }
        anyhow::bail!(
            "Cannot create the install folder:\n{}\n\nChoose a folder you can write to (e.g. under your user folder). ({})",
            dir.display(),
            e
        );
    }
    let probe = long_path(&dir.join(".write_test"));
    match File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            common::log::error(format!("not writable: {} ({})", dir.display(), e));
            if common::elevation::is_permission_denied(&e) {
                return Err(anyhow::Error::new(PermissionDeniedError));
            }
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
    let mut f = archive
        .by_name(rel)
        .with_context(|| format!("{} not in zip", rel))?;
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
    hash_file_progress(path, &AtomicU64::new(0))
}

/// Like [`hash_file`] but adds each chunk's byte count to `progress`, so a
/// watcher can tell a slow-but-advancing read from a fully stalled one.
fn hash_file_progress(path: &Path, progress: &AtomicU64) -> Result<String> {
    // Streaming read, not mmap: mmap-hashing a just-written file stalls behind
    // Defender's on-access scan, and a flaky-disk fault surfaces as an
    // un-catchable in-page exception. A plain read returns a normal `io::Error`.
    let mut f = File::open(path).map_err(|e| anyhow::anyhow!("{}", io_msg("opening", path, &e)))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| anyhow::anyhow!("{}", io_msg("reading", path, &e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Hash `path` on a helper thread, giving up if the read makes no progress for
/// `stall`. Progress-based rather than a total timeout, so a huge file on a slow
/// disk is fine as long as bytes keep flowing, while a read wedged behind
/// antivirus isn't. The abandoned thread drains on its own. `None` means stalled.
fn hash_within(path: &Path, stall: Duration) -> Option<Result<String>> {
    use std::sync::mpsc::RecvTimeoutError;
    let (tx, rx) = std::sync::mpsc::channel();
    let progress = Arc::new(AtomicU64::new(0));
    let p = path.to_path_buf();
    let prog = progress.clone();
    std::thread::spawn(move || {
        let _ = tx.send(hash_file_progress(&p, &prog));
    });
    let mut last_bytes = 0u64;
    let mut last_advance = Instant::now();
    let tick = Duration::from_secs(2).min(stall);
    loop {
        match rx.recv_timeout(tick) {
            Ok(r) => return Some(r),
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {
                let now = progress.load(Ordering::Relaxed);
                if now != last_bytes {
                    last_bytes = now;
                    last_advance = Instant::now();
                } else if last_advance.elapsed() >= stall {
                    return None;
                }
            }
        }
    }
}

enum VerifyOutcome {
    Match,
    Mismatch {
        got: String,
    },
    Missing,
    /// Read stalled (no progress within the stall window).
    Slow,
    /// File exists but couldn't be read.
    Error(String),
}

/// Hash one file, abandoning if the read stalls for `stall`, and classify it.
fn verify_one(path: &Path, expected: &str, stall: Duration) -> VerifyOutcome {
    match hash_within(path, stall) {
        Some(Ok(got)) if got == expected => VerifyOutcome::Match,
        Some(Ok(got)) => VerifyOutcome::Mismatch { got },
        Some(Err(_)) if !path.exists() => VerifyOutcome::Missing,
        Some(Err(e)) => VerifyOutcome::Error(format!("{e:#}")),
        None => VerifyOutcome::Slow,
    }
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
        move_retry(&dest, &backup).with_context(|| format!("backup {} before overwrite", rel))?;
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
    let content = adds
        .iter()
        .chain(deletes.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let jp = journal_path(temp_dir);
    let tmp = jp.with_extension("journal.tmp");
    fs::write(&tmp, content).context("write journal")?;
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

/// Recorded installed version, checking the machine-wide data dir
/// (`%ProgramData%`) first, then the per-user one (`%LOCALAPPDATA%`). A
/// machine-wide install records its version under ProgramData, so a per-user-only
/// lookup would wrongly report "not installed" and refuse every patch. `None` if
/// neither holds a version. Mirrors `main::previous_install_dir`'s precedence.
fn installed_version(payload: &InstallerPayload) -> Option<String> {
    for machine in [true, false] {
        if let Some(dir) =
            common::paths::uninstall_dir_for(&payload.publisher, &payload.product_id, machine)
            && let Some(v) = read_local_version(&dir)
        {
            return Some(v);
        }
    }
    None
}

/// Extract the `phase` plugins from the payload zip to `%TEMP%`, run their `up`
/// in isolated child processes, then clean up. Used for the pre-install phase.
fn run_zip_plugins(
    ctx: &InstallCtx,
    phase: common::model::plugin_phase::PluginPhase,
) -> Result<()> {
    let plugins: Vec<common::model::plugin_entry::PluginEntry> = ctx
        .payload
        .plugins
        .iter()
        .filter(|p| p.phase == phase)
        .cloned()
        .collect();
    if plugins.is_empty() {
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("iw-plugins-{}", std::process::id()));
    fs::create_dir_all(&tmp)?;
    let mut archive =
        ZipArchive::new(Cursor::new(ctx.zip_bytes)).context("open payload zip for plugins")?;
    let mut items = Vec::with_capacity(plugins.len());
    for p in plugins {
        let mut buf = Vec::new();
        {
            let mut f = archive
                .by_name(&p.file)
                .with_context(|| format!("plugin {} missing from payload", p.file))?;
            f.read_to_end(&mut buf)?;
        }
        let dst = tmp.join(format!("{}.dll", p.name));
        fs::write(&dst, &buf)?;
        let inputs_json = match ctx.plugin_inputs.get(&p.name) {
            Some(m) => serde_json::to_string(m)?,
            None => String::new(),
        };
        items.push((p, dst, inputs_json));
    }
    let pctx = plugin_ctx(ctx.payload, &ctx.install_dir, ctx.requires_admin);
    let self_exe = std::env::current_exe()?;
    let res = common::plugin::run_each(&self_exe, &pctx, &items, "up", true);
    let _ = fs::remove_dir_all(&tmp);
    res
}

/// Build the plugin context from a payload + chosen install dir. Shared with
/// the post-install (finalize) and uninstall paths. `requires_admin` selects the
/// data dir (`%ProgramData%` vs `%LOCALAPPDATA%`) so plugins see the same dir
/// `install::finalize` writes to and the uninstaller later reads from.
pub fn plugin_ctx(
    payload: &InstallerPayload,
    install_dir: &Path,
    requires_admin: bool,
) -> common::plugin::PluginCtx {
    let data_dir =
        common::paths::uninstall_dir_for(&payload.publisher, &payload.product_id, requires_admin)
            .unwrap_or_else(|| install_dir.to_path_buf());
    common::plugin::PluginCtx {
        install_dir: install_dir.to_string_lossy().into_owned(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        product: payload.product.clone(),
        product_id: payload.product_id.clone(),
        version: payload.to_version.clone(),
        exe: install_dir
            .join(&payload.manifest.exe)
            .to_string_lossy()
            .replace('/', "\\"),
        log_path: common::log::current_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        lang: common::i18n::current_lang().to_string(),
        ..Default::default()
    }
}

/// Removes its temp dir on drop. Keeps the extracted `ui` plugin DLLs alive for
/// the duration of the wizard.
pub struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Everything the plugin-page wizard needs: the `ui = true` plugins (entry +
/// extracted DLL, in config order), the base context, and this exe's path (to
/// spawn the plugin host). The temp dir of DLLs lives as long as `tmp`.
pub struct UiPlugins {
    pub plugins: Vec<(common::model::plugin_entry::PluginEntry, PathBuf)>,
    pub base_ctx: common::plugin::PluginCtx,
    pub self_exe: PathBuf,
    pub tmp: TempDirGuard,
}

/// Extract every `ui = true` plugin DLL from the payload to a temp dir kept alive
/// for the wizard. The wizard queries each plugin's `installway_pages` step by
/// step. `install_dir` here is the default (pages describe choices, independent of
/// the final folder — that reaches the plugin's `up` later). `None` when there are
/// no UI plugins or extraction fails.
pub fn extract_ui_plugins(
    payload: &InstallerPayload,
    install_dir: &Path,
    self_exe: &Path,
    zip_bytes: &[u8],
) -> Option<UiPlugins> {
    let ui: Vec<&common::model::plugin_entry::PluginEntry> =
        payload.plugins.iter().filter(|p| p.ui).collect();
    if ui.is_empty() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("iw-plugin-ui-{}", std::process::id()));
    if let Err(e) = fs::create_dir_all(&dir) {
        common::log::warn(format!("plugin pages: temp dir failed: {e:#}"));
        return None;
    }
    let tmp = TempDirGuard(dir.clone());
    let mut archive = match ZipArchive::new(Cursor::new(zip_bytes)) {
        Ok(a) => a,
        Err(e) => {
            common::log::warn(format!("plugin pages: open payload zip failed: {e:#}"));
            return None;
        }
    };
    let mut plugins = Vec::new();
    for p in ui {
        let dst = dir.join(format!("{}.dll", p.name));
        let extracted = (|| -> Result<()> {
            let mut buf = Vec::new();
            let mut f = archive
                .by_name(&p.file)
                .with_context(|| format!("plugin {} missing from payload", p.file))?;
            f.read_to_end(&mut buf)?;
            fs::write(&dst, &buf)?;
            Ok(())
        })();
        match extracted {
            Ok(()) => plugins.push((p.clone(), dst)),
            Err(e) => {
                common::log::warn(format!("plugin '{}' pages: extract failed: {e:#}", p.name))
            }
        }
    }
    if plugins.is_empty() {
        return None;
    }
    Some(UiPlugins {
        plugins,
        // UI pages run in the main process before elevation is decided; they
        // describe choices and don't persist state, so the per-user data dir is
        // fine here.
        base_ctx: plugin_ctx(payload, install_dir, false),
        self_exe: self_exe.to_path_buf(),
        tmp,
    })
}

/// Read the recorded installed version from `version.json` in the data dir.
fn read_local_version(data_dir: &Path) -> Option<String> {
    let s = fs::read_to_string(data_dir.join("version.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v["version"].as_str().map(|s| s.to_string())
}

fn cleanup(temp_dir: &Path) {
    common::utils::remove_dir_retry(temp_dir);
}

/// No-progress window before pass 1 gives up on a file and defers it to pass 2.
/// A stall window, not a size-dependent total: a large file streaming off a slow
/// disk keeps advancing and never trips it; a read stuck behind antivirus does.
const VERIFY_STALL: Duration = Duration::from_secs(30);
/// More patient stall window for the pass-2 re-test.
const VERIFY_RETRY_STALL: Duration = Duration::from_secs(90);

/// Short hash prefix for log lines.
fn short(h: &str) -> &str {
    &h[..16.min(h.len())]
}

enum Verdict {
    Ok,
    Corrupt,
    Unconfirmed,
}

/// Classify a verify outcome, logging the reason. `retry` selects the pass-2
/// wording (a still-unconfirmed file is a non-fatal "proceeding", not "retry").
fn classify(rel: &str, expected: &str, outcome: VerifyOutcome, retry: bool) -> Verdict {
    match outcome {
        VerifyOutcome::Match => Verdict::Ok,
        VerifyOutcome::Mismatch { got } => {
            common::log::warn(format!(
                "{rel} corrupt after writing (expected {}, got {})",
                short(expected),
                short(&got)
            ));
            Verdict::Corrupt
        }
        VerifyOutcome::Missing => {
            common::log::warn(format!("{rel} missing after writing"));
            Verdict::Corrupt
        }
        VerifyOutcome::Slow if retry => {
            common::log::warn(format!(
                "{rel} could not be verified (antivirus or slow disk) - proceeding without re-verification"
            ));
            Verdict::Unconfirmed
        }
        VerifyOutcome::Slow => {
            common::log::warn(format!(
                "{rel} stalled while verifying (antivirus or slow disk) - will retry"
            ));
            Verdict::Unconfirmed
        }
        VerifyOutcome::Error(e) if retry => {
            common::log::warn(format!(
                "{rel} could not be verified ({e}) - proceeding without re-verification"
            ));
            Verdict::Unconfirmed
        }
        VerifyOutcome::Error(e) => {
            common::log::warn(format!("{rel} unreadable ({e}) - will retry"));
            Verdict::Unconfirmed
        }
    }
}

/// Re-hash each committed file and return those that don't match the manifest
/// (corrupt, missing, or unreadable). Parallel; used inside the transaction.
///
/// `cancel` lets a user-requested cancel short-circuit the remaining files: the
/// re-hash reads every installed byte again and can be slow (AV scanning a fresh
/// `.exe`, a slow/network disk), so once cancel is set, pending files are skipped
/// rather than hashed. The caller re-checks the flag and rolls back.
fn find_corrupt(
    install_dir: &Path,
    manifest: &Manifest,
    committed: &[String],
    cancel: &AtomicBool,
) -> Vec<String> {
    let outcomes: Vec<(String, VerifyOutcome, Duration)> = committed
        .par_iter()
        .filter_map(|rel| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let entry = manifest.files.get(rel)?;
            let path = long_path(&install_dir.join(rel));
            let t = Instant::now();
            let outcome = verify_one(&path, &entry.hash, VERIFY_STALL);
            Some((rel.clone(), outcome, t.elapsed()))
        })
        .collect();

    let mut corrupt = Vec::new();
    let mut deferred = Vec::new();
    for (rel, outcome, elapsed) in outcomes {
        let expected = manifest
            .files
            .get(&rel)
            .map(|e| e.hash.as_str())
            .unwrap_or("");
        match classify(&rel, expected, outcome, false) {
            Verdict::Ok if elapsed >= Duration::from_secs(5) => {
                common::log::info(format!("verified {rel} ({:.1}s)", elapsed.as_secs_f64()));
            }
            Verdict::Ok => common::log::info(format!("verified {rel}")),
            Verdict::Corrupt => corrupt.push(rel),
            Verdict::Unconfirmed => deferred.push(rel),
        }
    }

    for rel in deferred {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(entry) = manifest.files.get(&rel) else {
            continue;
        };
        let path = long_path(&install_dir.join(&rel));
        let outcome = verify_one(&path, &entry.hash, VERIFY_RETRY_STALL);
        match classify(&rel, &entry.hash, outcome, true) {
            Verdict::Ok => common::log::info(format!("verified {rel} on retry")),
            Verdict::Corrupt => corrupt.push(rel),
            Verdict::Unconfirmed => {}
        }
    }

    corrupt
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
                move_retry(&staged, &dest).with_context(|| format!("repair-install {}", rel))?;
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
    let info_data = fs::read_to_string(&info_path)
        .with_context(|| format!("read {} - is this product installed?", info_path.display()))?;
    let info: common::model::install_info::InstallInfo =
        serde_json::from_str(&info_data).context("parse installer_info.json")?;

    let manifest_path = data_dir.join("installer_manifest.json");
    let mdata = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&mdata).context("parse installer_manifest.json")?;

    let app_dir = PathBuf::from(&info.install_dir);

    let mut rels: Vec<(&String, &common::model::file_entry::FileEntry)> =
        manifest.files.iter().collect();
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
        let other = std::io::Error::other("boom");
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
            common::model::file_entry::FileEntry {
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

        let no_cancel = AtomicBool::new(false);
        let corrupt = find_corrupt(&app, &manifest, &["foo.txt".to_string()], &no_cancel);
        assert_eq!(corrupt, vec!["foo.txt".to_string()]);

        repair_corrupt(
            &zip,
            PayloadKind::Full,
            &manifest,
            &staged,
            &backup,
            &app,
            &corrupt,
        )
        .unwrap();

        assert!(find_corrupt(&app, &manifest, &["foo.txt".to_string()], &no_cancel).is_empty());
        assert_eq!(fs::read(app.join("foo.txt")).unwrap(), good);
    }

    fn bytes_hash(b: &[u8]) -> String {
        blake3::hash(b).to_hex().to_string()
    }
}
