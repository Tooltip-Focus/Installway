// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Payload construction: scan the input tree, hash every file, generate
//! HDiffPatch deltas in patch mode, and pack everything into the in-memory
//! payload zip alongside its [`Manifest`].

use anyhow::{Context, Result, bail};
use common::model::file_entry::FileEntry;
use common::model::manifest::Manifest;
use common::model::patch_info::PatchInfo;
use common::utils::{bytes_blake3, collect_files, file_blake3, generate_patch};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const PATCHES_PREFIX: &str = "patches/";
const FULL_PREFIX: &str = "full/";

/// A payload-zip entry to bundle verbatim: `(in-zip name, source path)`.
pub(crate) type ZipJob = (String, PathBuf);

/// In-zip path for a file's binary patch: `patches/<blake3(rel)>.patch`. The
/// installer reads `PatchInfo.file` verbatim as the in-zip path, so the name
/// recorded in the manifest and the entry name written into the zip must both
/// come from this one function.
fn patch_entry_name(rel: &str) -> String {
    let safe = blake3::hash(rel.as_bytes()).to_hex();
    format!("{PATCHES_PREFIX}{safe}.patch")
}

/// Reject two paths differing only by case: on case-insensitive NTFS they'd map
/// to the same file and clobber. (Matters for cross-OS builds.)
fn check_case_collisions(files: &[String]) -> Result<()> {
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in files {
        let lower = f.to_lowercase();
        if let Some(prev) = seen.get(&lower) {
            bail!(
                "case-only filename collision: '{}' and '{}' resolve to the same \
                 file on Windows. Rename one before packing.",
                prev,
                f
            );
        }
        seen.insert(lower, f.clone());
    }
    Ok(())
}

/// Build a full payload: every file under `input`, hashed and zipped.
pub(crate) fn build_full(
    input: &Path,
    exe: Option<&str>,
    version: &str,
    plugins: &[ZipJob],
) -> Result<(Vec<u8>, Manifest)> {
    println!("Scanning {}", input.display());
    let files = collect_files(input)?;
    check_case_collisions(&files)?;
    println!("Found {} files", files.len());

    let entries: HashMap<String, FileEntry> = files
        .par_iter()
        .map(|rel| -> Result<(String, FileEntry)> {
            let abs = input.join(rel);
            let bytes = fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
            Ok((
                rel.clone(),
                FileEntry {
                    hash: bytes_blake3(&bytes),
                    size: bytes.len() as u64,
                    patch: None,
                    feature: None,
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .collect();

    let full_size: u64 = entries.values().map(|e| e.size).sum();
    let zip_bytes = write_zip(input, &files, &HashMap::new(), plugins)?;

    let manifest = Manifest {
        version: version.to_string(),
        exe: exe.map(|s| s.to_string()),
        files: entries,
        deleted_files: Vec::new(),
        full_size,
        total_patch_size: 0,
        features: Vec::new(),
        default_features: Vec::new(),
        feature_mode: Default::default(),
    };
    Ok((zip_bytes, manifest))
}

/// Build a patch payload: HDiffPatch deltas for changed files (when smaller
/// than the full file), full copies for new files, deletions recorded.
pub(crate) fn build_patch(
    new_input: &Path,
    old_input: &Path,
    exe: Option<&str>,
    version: &str,
    plugins: &[ZipJob],
) -> Result<(Vec<u8>, Manifest)> {
    // Warn up front if hdiffz is missing: patching still works but ships full
    // files instead of HDiffPatch deltas.
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let hd = exe_dir.join("hdiffz.exe");
        if !hd.exists() {
            eprintln!(
                "warning: {} not found - patch payload will ship full files instead of HDiffPatch deltas",
                hd.display()
            );
        }
    }

    println!("Scanning new {}", new_input.display());
    let new_files = collect_files(new_input)?;
    check_case_collisions(&new_files)?;
    println!("Scanning old {}", old_input.display());
    let old_files = collect_files(old_input)?;

    let new_set: HashSet<&String> = new_files.iter().collect();
    let old_set: HashSet<&String> = old_files.iter().collect();

    let mut deleted_files: Vec<String> = old_files
        .iter()
        .filter(|p| !new_set.contains(*p))
        .cloned()
        .collect();
    deleted_files.sort();

    // Per-file work: hash new, hash old if present, generate patch if both
    // exist + differ. The temp dir is RAII-cleaned on every path (drops after
    // write_zip has consumed the patch files).
    let temp_patches = tempfile::tempdir().context("create temp patch dir")?;

    struct WorkOut {
        rel: String,
        entry: FileEntry,
        patch_path: Option<PathBuf>,
        full_needed: bool,
    }

    let work: Vec<WorkOut> = new_files
        .par_iter()
        .map(|rel| -> Result<WorkOut> {
            let new_abs = new_input.join(rel);
            let new_hash = file_blake3(&new_abs)?;
            let new_size = fs::metadata(&new_abs)?.len();

            if !old_set.contains(rel) {
                return Ok(WorkOut {
                    rel: rel.clone(),
                    entry: FileEntry {
                        hash: new_hash,
                        size: new_size,
                        patch: None,
                        feature: None,
                    },
                    patch_path: None,
                    full_needed: true,
                });
            }

            let old_abs = old_input.join(rel);
            let old_hash = file_blake3(&old_abs)?;
            if old_hash == new_hash {
                // Unchanged - no payload entry needed.
                return Ok(WorkOut {
                    rel: rel.clone(),
                    entry: FileEntry {
                        hash: new_hash,
                        size: new_size,
                        patch: None,
                        feature: None,
                    },
                    patch_path: None,
                    full_needed: false,
                });
            }

            let safe_name = blake3::hash(rel.as_bytes()).to_hex().to_string();
            let patch_path = temp_patches.path().join(format!("{}.patch", safe_name));
            let ok = generate_patch(&old_abs, &new_abs, &patch_path)
                .with_context(|| format!("hdiffz {}", rel))?;
            if ok && patch_path.exists() {
                let psize = fs::metadata(&patch_path)?.len();
                // Heuristic: if patch is bigger than the full file, just ship the full.
                if psize < new_size {
                    return Ok(WorkOut {
                        rel: rel.clone(),
                        entry: FileEntry {
                            hash: new_hash,
                            size: new_size,
                            patch: Some(PatchInfo {
                                file: patch_entry_name(rel),
                                size: psize,
                            }),
                            feature: None,
                        },
                        patch_path: Some(patch_path),
                        full_needed: false,
                    });
                }
                // Patch wasn't smaller - fall through to full.
                let _ = fs::remove_file(&patch_path);
            }

            Ok(WorkOut {
                rel: rel.clone(),
                entry: FileEntry {
                    hash: new_hash,
                    size: new_size,
                    patch: None,
                    feature: None,
                },
                patch_path: None,
                full_needed: true,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let total_full_size: u64 = work.iter().map(|w| w.entry.size).sum();
    let total_patch_size: u64 = work
        .iter()
        .filter_map(|w| w.entry.patch.as_ref())
        .map(|p| p.size)
        .sum();

    let mut entries: HashMap<String, FileEntry> = HashMap::new();
    let mut full_paths: Vec<String> = Vec::new();
    let mut patch_paths: HashMap<String, PathBuf> = HashMap::new();
    for w in work {
        if w.full_needed {
            full_paths.push(w.rel.clone());
        }
        if let Some(p) = &w.patch_path {
            patch_paths.insert(w.rel.clone(), p.clone());
        }
        entries.insert(w.rel, w.entry);
    }

    let zip_bytes = write_zip(new_input, &full_paths, &patch_paths, plugins)?;

    let manifest = Manifest {
        version: version.to_string(),
        exe: exe.map(|s| s.to_string()),
        files: entries,
        deleted_files,
        full_size: total_full_size,
        total_patch_size,
        features: Vec::new(),
        default_features: Vec::new(),
        feature_mode: Default::default(),
    };
    Ok((zip_bytes, manifest))
}

/// Extensions already compressed: zstd gains ~0% and forces a pointless
/// decompress at install time, so they're `Stored` verbatim. Entropy-coded
/// media only - archive containers (.zip/.gz/...) can wrap weakly-compressed
/// data zstd still shrinks, so we let zstd try those.
const ALREADY_COMPRESSED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "heic", "mp3", "aac", "ogg", "opus", "flac",
    "mp4", "m4v", "mov", "avi", "mkv", "webm", "woff2", // brotli-compressed internally
];

/// Pick the compression method for one entry: `Stored` for hdiffz patches
/// (already zstd-compressed by `-c-zstd-21`) and already-compressed media
/// formats, `Zstd` for everything else.
fn method_for(name: &str) -> zip::CompressionMethod {
    if name.starts_with(PATCHES_PREFIX) {
        return zip::CompressionMethod::Stored;
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some(e) if ALREADY_COMPRESSED.contains(&e) => zip::CompressionMethod::Stored,
        _ => zip::CompressionMethod::Zstd,
    }
}

/// Compress one file into a standalone single-entry zip in memory. Run from
/// many rayon workers in parallel, each owning its own `ZipWriter`. The chosen
/// method is recorded in the entry header so merge + installer read it back.
fn compress_entry(entry_name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let method = method_for(entry_name);
    let mut opts = SimpleFileOptions::default()
        .compression_method(method)
        .large_file(bytes.len() as u64 >= u32::MAX as u64);
    if method == zip::CompressionMethod::Zstd {
        // Level 19: high ratio, sits before the 20+ compress-time cliff;
        // decompress speed is level-independent.
        opts = opts.compression_level(Some(19));
    }
    let cap = bytes.len() / 2 + 64;
    let mut zip = ZipWriter::new(Cursor::new(Vec::with_capacity(cap)));
    zip.start_file(entry_name, opts)?;
    zip.write_all(bytes)?;
    Ok(zip.finish()?.into_inner())
}

/// Build a zip in memory. `full_paths` go under `full/<rel>`; `patch_paths`
/// under their recorded path. Compression runs one rayon worker per file (each
/// a standalone single-entry zip), then the outputs are merged by raw byte copy
/// (`raw_copy_file`, no recompression) to saturate every core.
///
/// Peak memory is roughly the compressed payload twice over (minis + merged
/// zip) plus each worker's current file uncompressed — fine for app-sized
/// payloads; a streaming merge is the upgrade path if multi-GB inputs appear.
fn write_zip(
    input: &Path,
    full_paths: &[String],
    patch_paths: &HashMap<String, PathBuf>,
    extra: &[ZipJob],
) -> Result<Vec<u8>> {
    // (entry_name_in_zip, source_path_on_disk) for every file to pack.
    let mut jobs: Vec<ZipJob> =
        Vec::with_capacity(full_paths.len() + patch_paths.len() + extra.len());
    for rel in full_paths {
        jobs.push((format!("{}{}", FULL_PREFIX, rel), input.join(rel)));
    }
    for (rel, patch_path) in patch_paths {
        jobs.push((patch_entry_name(rel), patch_path.clone()));
    }
    // Extra verbatim entries (e.g. `plugins/<name>.dll`), already named.
    jobs.extend(extra.iter().cloned());

    // PHASE 1 (parallel): read + compress each file into its own mini-zip.
    let minis: Vec<Vec<u8>> = jobs
        .par_iter()
        .map(|(name, path)| {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            compress_entry(name, &bytes)
        })
        .collect::<Result<Vec<_>>>()?;

    // PHASE 2 (sequential): merge each mini-zip's entry by raw copy (already
    // compressed, so just memcpy + header rewrite). `into_iter` frees each mini
    // as consumed to keep peak memory down.
    let mut zip = ZipWriter::new(Cursor::new(Vec::with_capacity(16 * 1024 * 1024)));
    for mini in minis {
        let mut src =
            zip::ZipArchive::new(Cursor::new(mini)).context("reopen worker mini-zip for merge")?;
        let entry = src.by_index_raw(0).context("read mini-zip entry")?;
        zip.raw_copy_file(entry)
            .context("merge entry into payload zip")?;
    }

    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_collision_detection() {
        assert!(check_case_collisions(&["A.txt".to_string(), "b.txt".to_string()]).is_ok());
        assert!(
            check_case_collisions(&["dir/A.txt".to_string(), "dir/a.txt".to_string()]).is_err()
        );
        assert!(check_case_collisions(&["Foo".to_string(), "foo".to_string()]).is_err());
    }

    #[test]
    fn method_for_stores_patches_and_media() {
        // hdiffz output is already zstd -> stored verbatim.
        assert_eq!(
            method_for("patches/abc.patch"),
            zip::CompressionMethod::Stored
        );
        // Entropy-coded media -> stored; everything else -> zstd.
        assert_eq!(method_for("full/img.png"), zip::CompressionMethod::Stored);
        assert_eq!(method_for("full/app.dll"), zip::CompressionMethod::Zstd);
        assert_eq!(method_for("plugins/x.dll"), zip::CompressionMethod::Zstd);
    }
}
