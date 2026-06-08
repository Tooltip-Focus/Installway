// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use crate::args::PackArgs;
use crate::embed;
use anyhow::{Context, Result, bail};
use common::models::{
    FileAssoc, FileEntry, InstallerPayload, Manifest, PatchInfo, PayloadKind, SignedPayload,
};
use common::utils::{
    bytes_blake3, collect_files, copy_retry, file_blake3, generate_patch, remove_dir_retry,
    remove_file_retry,
};
use ed25519_dalek::{Signer, SigningKey};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const PATCHES_PREFIX: &str = "patches/";
const FULL_PREFIX: &str = "full/";

/// In-zip path for a file's binary patch: `patches/<blake3(rel)>.patch`. The
/// installer reads `PatchInfo.file` verbatim as the in-zip path, so the name
/// recorded in the manifest and the entry name written into the zip must both
/// come from this one function.
fn patch_entry_name(rel: &str) -> String {
    let safe = blake3::hash(rel.as_bytes()).to_hex();
    format!("{PATCHES_PREFIX}{safe}.patch")
}

pub fn run(args: &PackArgs) -> Result<()> {
    let is_patch = args.from_dir.is_some() || args.from_version.is_some();
    if is_patch && (args.from_dir.is_none() || args.from_version.is_none()) {
        bail!("patch mode requires both --from-dir and --from-version");
    }
    if args.publisher.trim().is_empty() {
        bail!("--publisher must not be empty");
    }

    println!("Mode: {}", if is_patch { "PATCH" } else { "FULL" });

    let signing = load_signing_key(&args.priv_key)?;

    // Toolchain-free mode: prebuilt stub + uninstaller supplied, so we never
    // invoke cargo. The stub already has its public key baked in.
    let prebuilt = args.installer_stub.is_some() || args.uninstaller.is_some();
    if prebuilt && (args.installer_stub.is_none() || args.uninstaller.is_none()) {
        bail!("--installer-stub and --uninstaller must be provided together");
    }
    let pub_key_hex: Option<String> = if prebuilt {
        println!("Toolchain-free mode: using prebuilt binaries (no cargo build)");
        None
    } else {
        let p = args
            .pub_key
            .as_ref()
            .context("--pub-key is required (omit it only when using --installer-stub)")?;
        Some(load_pub_key_hex(p)?)
    };

    let zip_bytes;
    let manifest;
    if is_patch {
        let from_dir = args
            .from_dir
            .as_ref()
            .context("patch mode requires --from-dir")?;
        (zip_bytes, manifest) = build_patch(&args.input, from_dir, &args.exe, &args.to_version)?;
    } else {
        (zip_bytes, manifest) = build_full(&args.input, &args.exe, &args.to_version)?;
    }

    let license_text = match &args.license {
        Some(p) => {
            let text = fs::read_to_string(p)
                .with_context(|| format!("read license file {}", p.display()))?;
            println!("License: {} ({} bytes) from {}", trimmed_title(&text), text.len(), p.display());
            Some(text)
        }
        None => None,
    };

    let associations = parse_assocs(&args.assoc, &args.product)?;

    let payload = InstallerPayload {
        kind: if is_patch { PayloadKind::Patch } else { PayloadKind::Full },
        product: args.product.clone(),
        publisher: args.publisher.clone(),
        from_version: args.from_version.clone(),
        to_version: args.to_version.clone(),
        min_installer_version: args.min_installer_version.clone(),
        payload_blake3: bytes_blake3(&zip_bytes),
        created_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default(),
        manifest,
        license_text,
        associations,
        force_reinstall: args.force_reinstall,
        skip_license: args.skip_license,
        skip_path: args.skip_path,
        default_install_dir: args.default_install_dir.clone(),
    };

    let payload_json = serde_json::to_string(&payload).context("serialize payload")?;
    let signature = signing.sign(payload_json.as_bytes());
    let signed = SignedPayload {
        payload_json,
        signature_hex: hex::encode(signature.to_bytes()),
    };
    let signed_json = serde_json::to_string(&signed).context("serialize signed payload")?;

    println!("Payload: {} bytes (zip)", zip_bytes.len());
    println!("Signed manifest: {} bytes", signed_json.len());

    let stub = match &args.installer_stub {
        Some(p) => {
            if !p.exists() {
                bail!("--installer-stub not found: {}", p.display());
            }
            println!("Using prebuilt stub: {}", p.display());
            p.clone()
        }
        None => build_installer_stub(
            pub_key_hex.as_deref().expect("pub_key_hex set in toolchain mode"),
            args.reuse_stub,
        )?,
    };
    println!("Stub: {}", stub.display());

    // Pull the icon from the packaged exe (best-effort).
    let icons = {
        let exe_path = args.input.join(&args.exe);
        if exe_path.exists() {
            match crate::icon::extract_from_exe(&exe_path) {
                Ok(Some(i)) => {
                    println!(
                        "Icon: {} group(s) + {} icon(s) copied from {}",
                        i.groups.len(),
                        i.icons.len(),
                        exe_path.display()
                    );
                    Some(i)
                }
                Ok(None) => {
                    println!("Icon: source exe {} has no icon resources", exe_path.display());
                    None
                }
                Err(e) => {
                    eprintln!("warning: icon extraction failed: {e:#}");
                    None
                }
            }
        } else {
            None
        }
    };

    let uninstaller = match &args.uninstaller {
        Some(p) => {
            if !p.exists() {
                bail!("--uninstaller not found: {}", p.display());
            }
            p.clone()
        }
        None => build_uninstaller(args.reuse_stub)?,
    };
    // Stamp icons on a %TEMP% copy so we don't mutate the cached release artifact.
    let staged_uninstaller = std::env::temp_dir().join(format!(
        "rustinst-uninst-{}.exe",
        std::process::id()
    ));
    copy_retry(&uninstaller, &staged_uninstaller).with_context(|| {
        format!("stage uninstaller {} -> {}", uninstaller.display(), staged_uninstaller.display())
    })?;
    if let Some(i) = &icons {
        if let Err(e) = crate::icon::embed_icons(&staged_uninstaller, i) {
            eprintln!("warning: icon embed into uninstaller failed: {e:#}");
        }
    }
    let uninstaller_bytes = fs::read(&staged_uninstaller)
        .with_context(|| format!("read {}", staged_uninstaller.display()))?;
    let _ = remove_file_retry(&staged_uninstaller);
    println!("Uninstaller: {} bytes (icon-stamped)", uninstaller_bytes.len());

    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&stub, &args.out).with_context(|| format!("copy {} -> {}", stub.display(), args.out.display()))?;
    println!("Copied stub to {}", args.out.display());

    // Small resources via the resource API.
    embed::embed_resources(
        &args.out,
        signed_json.as_bytes(),
        &uninstaller_bytes,
        zip_bytes.len() as u64,
    )?;
    if let Some(i) = &icons {
        if let Err(e) = crate::icon::embed_icons(&args.out, i) {
            eprintln!("warning: icon embed into setup failed: {e:#}");
        }
    }
    // Version resource (Explorer Details tab + SmartScreen reputation).
    if let Err(e) =
        crate::version::embed(&args.out, &args.product, &args.publisher, &args.to_version)
    {
        eprintln!("warning: version-info embed failed: {e:#}");
    }
    // Payload appended as a PE overlay, after all resource passes (so they
    // don't drop it) and before signing. No size ceiling; installer mmaps it.
    embed::append_payload(&args.out, &zip_bytes)?;
    println!(
        "Embedded signed manifest + uninstaller{} + version, appended {}-byte payload overlay into {}",
        if icons.is_some() { " + icon" } else { "" },
        zip_bytes.len(),
        args.out.display()
    );

    println!();
    println!("DONE.");
    println!("Next step (Authenticode): signtool sign /fd SHA256 /tr http://timestamp.digicert.com {}", args.out.display());
    Ok(())
}

fn load_signing_key(path: &Path) -> Result<SigningKey> {
    let hex_data = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let bytes = hex::decode(hex_data.trim())
        .with_context(|| format!("decode hex private key {}", path.display()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?;
    Ok(SigningKey::from_bytes(&arr))
}

fn load_pub_key_hex(path: &Path) -> Result<String> {
    let hex_data = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let hex_data = hex_data.trim().to_string();
    let bytes = hex::decode(&hex_data)
        .with_context(|| format!("decode hex public key {}", path.display()))?;
    if bytes.len() != 32 {
        bail!("public key must be 32 bytes");
    }
    Ok(hex_data)
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

fn build_full(input: &Path, exe: &str, version: &str) -> Result<(Vec<u8>, Manifest)> {
    println!("Scanning {}", input.display());
    let files = collect_files(input)?;
    check_case_collisions(&files)?;
    println!("Found {} files", files.len());

    let total_size = Mutex::new(0u64);
    let entries: HashMap<String, FileEntry> = files
        .par_iter()
        .map(|rel| -> Result<(String, FileEntry, Vec<u8>)> {
            let abs = input.join(rel);
            let bytes = fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
            let hash = bytes_blake3(&bytes);
            let size = bytes.len() as u64;
            {
                let mut t = total_size.lock().unwrap();
                *t += size;
            }
            Ok((rel.clone(), FileEntry { hash, size, patch: None }, bytes))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|(rel, entry, _)| (rel, entry))
        .collect();

    let zip_bytes = write_zip(input, &files, &HashMap::new())?;

    let manifest = Manifest {
        version: version.to_string(),
        exe: exe.to_string(),
        files: entries,
        deleted_files: Vec::new(),
        full_size: *total_size.lock().unwrap(),
        total_patch_size: 0,
    };
    Ok((zip_bytes, manifest))
}

fn build_patch(
    new_input: &Path,
    old_input: &Path,
    exe: &str,
    version: &str,
) -> Result<(Vec<u8>, Manifest)> {
    // Warn up front if hdiffz is missing: patching still works but ships full
    // files instead of HDiffPatch deltas.
    if let Some(exe_dir) =
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
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

    let total_full_size = Mutex::new(0u64);
    let total_patch_size = Mutex::new(0u64);

    // Per-file work: hash new, hash old if present, generate patch if both exist + differ.
    let temp_patches = std::env::temp_dir().join(format!(
        "installway-patches-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_patches)?;

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
            {
                let mut t = total_full_size.lock().unwrap();
                *t += new_size;
            }

            if !old_set.contains(rel) {
                return Ok(WorkOut {
                    rel: rel.clone(),
                    entry: FileEntry { hash: new_hash, size: new_size, patch: None },
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
                    entry: FileEntry { hash: new_hash, size: new_size, patch: None },
                    patch_path: None,
                    full_needed: false,
                });
            }

            let safe_name = blake3::hash(rel.as_bytes()).to_hex().to_string();
            let patch_path = temp_patches.join(format!("{}.patch", safe_name));
            let ok = generate_patch(&old_abs, &new_abs, &patch_path)
                .with_context(|| format!("hdiffz {}", rel))?;
            if ok && patch_path.exists() {
                let psize = fs::metadata(&patch_path)?.len();
                // Heuristic: if patch is bigger than the full file, just ship the full.
                if psize < new_size {
                    {
                        let mut t = total_patch_size.lock().unwrap();
                        *t += psize;
                    }
                    return Ok(WorkOut {
                        rel: rel.clone(),
                        entry: FileEntry {
                            hash: new_hash,
                            size: new_size,
                            patch: Some(PatchInfo {
                                file: patch_entry_name(rel),
                                size: psize,
                            }),
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
                entry: FileEntry { hash: new_hash, size: new_size, patch: None },
                patch_path: None,
                full_needed: true,
            })
        })
        .collect::<Result<Vec<_>>>()?;

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

    let zip_bytes = write_zip(new_input, &full_paths, &patch_paths)?;

    remove_dir_retry(&temp_patches);

    let manifest = Manifest {
        version: version.to_string(),
        exe: exe.to_string(),
        files: entries,
        deleted_files,
        full_size: *total_full_size.lock().unwrap(),
        total_patch_size: *total_patch_size.lock().unwrap(),
    };
    Ok((zip_bytes, manifest))
}

/// Extensions already compressed: zstd gains ~0% and forces a pointless
/// decompress at install time, so they're `Stored` verbatim. Entropy-coded
/// media only - archive containers (.zip/.gz/...) can wrap weakly-compressed
/// data zstd still shrinks, so we let zstd try those.
const ALREADY_COMPRESSED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "heic",
    "mp3", "aac", "ogg", "opus", "flac", "mp4", "m4v", "mov", "avi", "mkv", "webm",
    "woff2", // brotli-compressed internally
];

/// Pick the compression method for one entry by extension: `Stored` for
/// already-compressed formats, `Zstd` for everything else.
fn method_for(name: &str) -> zip::CompressionMethod {
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
fn write_zip(
    input: &Path,
    full_paths: &[String],
    patch_paths: &HashMap<String, PathBuf>,
) -> Result<Vec<u8>> {
    // (entry_name_in_zip, source_path_on_disk) for every file to pack.
    let mut jobs: Vec<(String, PathBuf)> = Vec::with_capacity(full_paths.len() + patch_paths.len());
    for rel in full_paths {
        jobs.push((format!("{}{}", FULL_PREFIX, rel), input.join(rel)));
    }
    for (rel, patch_path) in patch_paths {
        jobs.push((patch_entry_name(rel), patch_path.clone()));
    }

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
    for mini in minis.into_iter() {
        let mut src = zip::ZipArchive::new(Cursor::new(mini))
            .context("reopen worker mini-zip for merge")?;
        let entry = src.by_index_raw(0).context("read mini-zip entry")?;
        zip.raw_copy_file(entry).context("merge entry into payload zip")?;
    }

    Ok(zip.finish()?.into_inner())
}

/// Build (or reuse) the installer stub with the given public key compiled in.
/// Returns the path to the built `.exe`.
fn build_installer_stub(pub_key_hex: &str, reuse: bool) -> Result<PathBuf> {
    let workspace_root = find_workspace_root()?;
    let target_exe = workspace_root
        .join("target")
        .join("release")
        .join("installer.exe");

    if reuse && target_exe.exists() {
        println!("Reusing existing stub at {}", target_exe.display());
        return Ok(target_exe);
    }

    println!("Building installer stub (cargo build -p installer --release)...");
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "installer", "--release"])
        .env("INSTALLER_PUB_KEY", pub_key_hex)
        .current_dir(&workspace_root)
        .status()
        .context("invoke cargo build")?;

    if !status.success() {
        bail!("cargo build failed");
    }
    if !target_exe.exists() {
        bail!("expected stub not found at {}", target_exe.display());
    }
    Ok(target_exe)
}

fn build_uninstaller(reuse: bool) -> Result<PathBuf> {
    let workspace_root = find_workspace_root()?;
    let target_exe = workspace_root
        .join("target")
        .join("release")
        .join("uninstall.exe");

    if reuse && target_exe.exists() {
        return Ok(target_exe);
    }

    println!("Building uninstaller (cargo build -p uninstaller --release)...");
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "uninstaller", "--release"])
        .current_dir(&workspace_root)
        .status()
        .context("invoke cargo build uninstaller")?;
    if !status.success() {
        bail!("uninstaller cargo build failed");
    }
    if !target_exe.exists() {
        bail!("expected uninstaller not found at {}", target_exe.display());
    }
    Ok(target_exe)
}

fn find_workspace_root() -> Result<PathBuf> {
    let mut p: PathBuf = std::env::current_dir()?;
    loop {
        let manifest = p.join("Cargo.toml");
        if manifest.exists() {
            let text = fs::read_to_string(&manifest).unwrap_or_default();
            if text.contains("[workspace]") {
                return Ok(p);
            }
        }
        if !p.pop() {
            bail!("could not locate workspace root from {:?}", std::env::current_dir());
        }
    }
}

/// Parse `--assoc ".ext:Description"` entries into `FileAssoc`s.
/// Extension is normalized to a leading dot; description may contain colons.
fn parse_assocs(raw: &[String], product: &str) -> Result<Vec<FileAssoc>> {
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let (ext, desc) = s
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("bad --assoc '{}': expected \".ext:Description\"", s))?;
        let ext = common::assoc::normalize_ext(ext);
        if ext == "." {
            bail!("bad --assoc '{}': empty extension", s);
        }
        let description = desc.trim().to_string();
        let progid = common::assoc::progid_for(product, &ext);
        println!("Association: {} -> {} ({})", ext, progid, description);
        out.push(FileAssoc { ext, description });
    }
    Ok(out)
}

/// First non-empty line of `s`, truncated to 60 chars - used for log preview.
fn trimmed_title(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() > 60 {
        format!("{}...", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_assocs_valid_and_colon_in_desc() {
        let v = parse_assocs(
            &[".myx:My Doc".to_string(), ".a:b:c".to_string()],
            "Prod",
        )
        .unwrap();
        assert_eq!(v[0].ext, ".myx");
        assert_eq!(v[0].description, "My Doc");
        // split_once on the first ':' -> description keeps the rest.
        assert_eq!(v[1].ext, ".a");
        assert_eq!(v[1].description, "b:c");
    }

    #[test]
    fn parse_assocs_rejects_bad() {
        assert!(parse_assocs(&["noColon".to_string()], "P").is_err());
        assert!(parse_assocs(&[":nodesc".to_string()], "P").is_err()); // empty ext
    }

    #[test]
    fn case_collision_detection() {
        assert!(check_case_collisions(&["A.txt".to_string(), "b.txt".to_string()]).is_ok());
        assert!(check_case_collisions(&["dir/A.txt".to_string(), "dir/a.txt".to_string()]).is_err());
        assert!(check_case_collisions(&["Foo".to_string(), "foo".to_string()]).is_err());
    }

    #[test]
    fn trimmed_title_first_line_truncated() {
        assert_eq!(trimmed_title("\n\nHello\nworld"), "Hello");
        let long = "x".repeat(80);
        let t = trimmed_title(&long);
        assert!(t.ends_with("...") && t.chars().count() == 63);
    }
}
