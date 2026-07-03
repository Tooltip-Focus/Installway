// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! `pack` command orchestration. [`run`] drives the phases in order; the heavy
//! lifting lives in the sibling modules ([`crate::payload`] for the zip +
//! manifest, [`crate::embed`] for the PE resources, [`crate::toolchain`] for
//! cargo builds).

use crate::args::{PackArgs, ResolvedPlugin, parse_assocs};
use crate::banner::read_banner_png;
use crate::embed::{self, EmbedSpec};
use crate::icon::ExeIcons;
use crate::keys::{
    load_pub_key_hex, load_signing_key, parse_signing_key_hex, validate_pub_key_hex,
};
use crate::license::{decode_license, trimmed_title};
use crate::payload::{ZipJob, build_full, build_patch};
use crate::toolchain::cargo_build_release;
use anyhow::{Context, Result, bail};
use common::model::file_assoc::FileAssoc;
use common::model::installer_payload::InstallerPayload;
use common::model::manifest::Manifest;
use common::model::payload_kind::PayloadKind;
use common::model::plugin_entry::PluginEntry;
use common::model::signed_payload::SignedPayload;
use common::utils::{bytes_blake3, copy_retry, file_blake3};
use ed25519_dalek::{Signer, SigningKey};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(args: &PackArgs) -> Result<()> {
    let patch_from = match (&args.from_dir, &args.from_version) {
        (Some(dir), Some(_)) => Some(dir.clone()),
        (None, None) => None,
        _ => bail!("patch mode requires both --from-dir and --from-version"),
    };
    println!(
        "Mode: {}",
        if patch_from.is_some() {
            "PATCH"
        } else {
            "FULL"
        }
    );

    let (signing, pub_key_hex) = resolve_keys(args)?;
    let (plugin_entries, plugin_files) = scan_plugins(&args.plugins)?;

    // Payload zip + manifest.
    let (zip_bytes, mut manifest) = match &patch_from {
        Some(from_dir) => build_patch(
            &args.input,
            from_dir,
            &args.exe,
            &args.to_version,
            &plugin_files,
        )?,
        None => build_full(&args.input, &args.exe, &args.to_version, &plugin_files)?,
    };
    // Tag files with their feature pack in the manifest (the zip keeps every file).
    crate::features::apply(&mut manifest, &args.features)?;

    let license_text = load_license(args)?;
    let banner_png = args
        .banner
        .as_ref()
        .map(|p| read_banner_png(p))
        .transpose()?;
    let associations = parse_assocs(&args.assoc, &args.product_id)?;

    let signed_json = sign_payload(
        args,
        &signing,
        &zip_bytes,
        manifest,
        plugin_entries,
        license_text,
        associations,
    )?;
    println!("Payload: {} bytes (zip)", zip_bytes.len());
    println!("Signed manifest: {} bytes", signed_json.len());

    let stub = resolve_stub(args, pub_key_hex.as_deref())?;
    println!("Stub: {}", stub.display());

    let icons = extract_app_icons(args);
    let uninstaller_bytes = prepare_uninstaller(args, icons.as_ref())?;

    assemble_output(
        args,
        &stub,
        &signed_json,
        &uninstaller_bytes,
        &zip_bytes,
        banner_png.as_deref(),
        icons.as_ref(),
    )?;

    println!();
    println!("DONE.");
    println!(
        "Next step (Authenticode): signtool sign /fd SHA256 /tr http://timestamp.digicert.com {}",
        args.out.display()
    );
    Ok(())
}

/// Resolve the Ed25519 signing key and, in toolchain mode, the public-key hex
/// to bake into the stub. In toolchain-free (prebuilt) mode the stub carries
/// its own compiled-in key, so the public key resolves to `None`.
fn resolve_keys(args: &PackArgs) -> Result<(SigningKey, Option<String>)> {
    let signing = match (&args.priv_key, &args.priv_key_literal) {
        (Some(path), _) => load_signing_key(path)?,
        (None, Some(hex)) => parse_signing_key_hex(hex)?,
        (None, None) => unreachable!("validated in PackArgs::resolve"),
    };

    // Toolchain-free mode: prebuilt stub + uninstaller supplied, so we never
    // invoke cargo.
    let prebuilt = args.installer_stub.is_some() || args.uninstaller.is_some();
    if prebuilt && (args.installer_stub.is_none() || args.uninstaller.is_none()) {
        bail!("--installer-stub and --uninstaller must be provided together");
    }
    let pub_key_hex = if prebuilt {
        println!("Toolchain-free mode: using prebuilt binaries (no cargo build)");
        if args.pub_key.is_some() || args.pub_key_literal.is_some() {
            println!(
                "warning: --pub-key / --pub-key-literal is ignored in toolchain-free mode - \
                 the stub (installer.exe) carries its own compiled-in key; \
                 --priv-key / --priv-key-literal must match it"
            );
        }
        None
    } else {
        Some(match (&args.pub_key, &args.pub_key_literal) {
            (Some(path), _) => load_pub_key_hex(path)?,
            (None, Some(hex)) => validate_pub_key_hex(hex)?,
            (None, None) => bail!(
                "--pub-key or --pub-key-literal is required \
                 (omit it only when using --installer-stub)"
            ),
        })
    };
    Ok((signing, pub_key_hex))
}

/// Read each plugin DLL for its hash + in-zip name. The bytes themselves are
/// bundled into the payload zip by `build_full` / `build_patch`.
fn scan_plugins(plugins: &[ResolvedPlugin]) -> Result<(Vec<PluginEntry>, Vec<ZipJob>)> {
    let mut entries = Vec::with_capacity(plugins.len());
    let mut files = Vec::with_capacity(plugins.len());
    for p in plugins {
        let in_zip = format!("plugins/{}.dll", p.name);
        let hash =
            file_blake3(&p.src).with_context(|| format!("read plugin dll {}", p.src.display()))?;
        println!("Plugin: {} ({:?}) <- {}", p.name, p.phase, p.src.display());
        files.push((in_zip.clone(), p.src.clone()));
        entries.push(PluginEntry {
            name: p.name.clone(),
            file: in_zip,
            blake3: hash,
            phase: p.phase,
            required: p.required,
            ui: p.ui,
        });
    }
    Ok((entries, files))
}

/// Read + decode the optional license file (see [`decode_license`] for the
/// encoding rules).
fn load_license(args: &PackArgs) -> Result<Option<String>> {
    let Some(p) = &args.license else {
        return Ok(None);
    };
    let bytes = fs::read(p).with_context(|| format!("read license file {}", p.display()))?;
    let text = decode_license(&bytes).with_context(|| format!("license file {}", p.display()))?;
    println!(
        "License: {} ({} bytes) from {}",
        trimmed_title(&text),
        text.len(),
        p.display()
    );
    Ok(Some(text))
}

/// Assemble the [`InstallerPayload`], serialize it and sign it. Returns the
/// signed-manifest JSON embedded into the setup exe.
fn sign_payload(
    args: &PackArgs,
    signing: &SigningKey,
    zip_bytes: &[u8],
    manifest: Manifest,
    plugins: Vec<PluginEntry>,
    license_text: Option<String>,
    associations: Vec<FileAssoc>,
) -> Result<String> {
    let payload = InstallerPayload {
        kind: if args.from_dir.is_some() {
            PayloadKind::Patch
        } else {
            PayloadKind::Full
        },
        product: args.product.clone(),
        product_id: args.product_id.clone(),
        publisher: args.publisher.clone(),
        from_version: args.from_version.clone(),
        to_version: args.to_version.clone(),
        min_installer_version: args.min_installer_version.clone(),
        payload_blake3: bytes_blake3(zip_bytes),
        created_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default(),
        manifest,
        license_text,
        associations,
        force_reinstall: args.force_reinstall,
        purge_unknown_files: args.purge_unknown_files,
        skip_license: args.skip_license,
        skip_path: args.skip_path,
        install_dir_restriction: args.install_dir_restriction,
        default_install_dir: args.default_install_dir.clone(),
        upgrade_minimal_ui: args.upgrade_minimal_ui,
        show_uninstall_complete: args.show_uninstall_complete,
        launch_option: args.launch_option,
        registry: args.registry.clone(),
        plugins,
        shortcuts: args.shortcuts.clone(),
        active_features: Vec::new(),
    };

    let payload_json = serde_json::to_string(&payload).context("serialize payload")?;
    let signature = signing.sign(payload_json.as_bytes());
    let signed = SignedPayload {
        payload_json,
        signature_hex: hex::encode(signature.to_bytes()),
    };
    serde_json::to_string(&signed).context("serialize signed payload")
}

/// Locate the installer stub: the prebuilt one when supplied, else a cargo
/// build with the public key compiled in.
fn resolve_stub(args: &PackArgs, pub_key_hex: Option<&str>) -> Result<PathBuf> {
    match &args.installer_stub {
        Some(p) => {
            if !p.exists() {
                bail!("--installer-stub not found: {}", p.display());
            }
            println!("Using prebuilt stub: {}", p.display());
            Ok(p.clone())
        }
        None => cargo_build_release(
            "installer",
            "installer.exe",
            Some((
                "INSTALLER_PUB_KEY",
                pub_key_hex.expect("pub_key_hex set in toolchain mode"),
            )),
            args.reuse_stub,
        ),
    }
}

/// Pull the icon resources from the packaged exe (best-effort: a failure only
/// logs a warning and the setup keeps the stub's default icon).
fn extract_app_icons(args: &PackArgs) -> Option<ExeIcons> {
    let exe_path = args.input.join(&args.exe);
    if !exe_path.exists() {
        return None;
    }
    match crate::icon::extract_from_exe(&exe_path) {
        Ok(Some(i)) => {
            println!(
                "Icon: {} group(s) + {} icon(s) copied from {}",
                i.group_count(),
                i.icon_count(),
                exe_path.display()
            );
            Some(i)
        }
        Ok(None) => {
            println!(
                "Icon: source exe {} has no icon resources",
                exe_path.display()
            );
            None
        }
        Err(e) => {
            eprintln!("warning: icon extraction failed: {e:#}");
            None
        }
    }
}

/// Locate (or build) the uninstaller, stamp the app icons on a temp copy so the
/// cached release artifact is never mutated, and return its bytes. The temp
/// dir is cleaned up on every path, including errors.
fn prepare_uninstaller(args: &PackArgs, icons: Option<&ExeIcons>) -> Result<Vec<u8>> {
    let uninstaller = match &args.uninstaller {
        Some(p) => {
            if !p.exists() {
                bail!("--uninstaller not found: {}", p.display());
            }
            p.clone()
        }
        None => cargo_build_release("uninstaller", "uninstall.exe", None, args.reuse_stub)?,
    };

    let staging = tempfile::tempdir().context("create uninstaller staging dir")?;
    let staged = staging.path().join("uninstall.exe");
    copy_retry(&uninstaller, &staged).with_context(|| {
        format!(
            "stage uninstaller {} -> {}",
            uninstaller.display(),
            staged.display()
        )
    })?;
    if let Some(i) = icons
        && let Err(e) = crate::icon::embed_icons(&staged, i)
    {
        eprintln!("warning: icon embed into uninstaller failed: {e:#}");
    }
    let bytes = fs::read(&staged).with_context(|| format!("read {}", staged.display()))?;
    println!("Uninstaller: {} bytes (icon-stamped)", bytes.len());
    Ok(bytes)
}

/// `<out>.tmp` next to the final path, so the last step is a same-volume rename.
fn tmp_out_path(out: &Path) -> PathBuf {
    let mut name = out
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "setup.exe".into());
    name.push(".tmp");
    out.with_file_name(name)
}

/// Assemble the final exe on a `.tmp` path: copy the stub, embed all resources
/// in one pass, append the payload overlay, self-verify, then atomically rename
/// over `args.out`. A failed build never leaves a half-written setup.exe at the
/// output path.
fn assemble_output(
    args: &PackArgs,
    stub: &Path,
    signed_json: &str,
    uninstaller_bytes: &[u8],
    zip_bytes: &[u8],
    banner_png: Option<&[u8]>,
    icons: Option<&ExeIcons>,
) -> Result<()> {
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_out_path(&args.out);

    let build = || -> Result<()> {
        fs::copy(stub, &tmp)
            .with_context(|| format!("copy {} -> {}", stub.display(), tmp.display()))?;

        // All resources (RCDATA blobs, version info, icon) in a single editpe
        // pass — sequential passes over the same PE corrupt it. The version
        // resource feeds the Explorer Details tab + SmartScreen reputation.
        embed::embed_all(
            &tmp,
            &EmbedSpec {
                signed_json: signed_json.as_bytes(),
                uninstaller_exe: uninstaller_bytes,
                payload_len: zip_bytes.len() as u64,
                banner_png,
                product: &args.product,
                publisher: &args.publisher,
                version: &args.to_version,
                icons,
            },
        )?;
        // Payload appended as a PE overlay, after all resource passes (so they
        // don't drop it) and before signing. No size ceiling; installer mmaps it.
        embed::append_payload(&tmp, zip_bytes)?;
        println!(
            "Embedded signed manifest + uninstaller{} + version, appended {}-byte payload overlay",
            if icons.is_some() { " + icon" } else { "" },
            zip_bytes.len(),
        );

        // Self-check: run the produced installer's own `--verify`. Catches a
        // stub whose compiled-in public key doesn't match `--priv-key` (or a
        // keyless stub) at build time, instead of shipping an installer that
        // refuses its own payload at runtime.
        self_verify(&tmp)
    };

    if let Err(e) = build() {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, &args.out)
        .with_context(|| format!("rename {} -> {}", tmp.display(), args.out.display()))?;
    println!("Wrote {}", args.out.display());
    Ok(())
}

/// Run `<setup> --verify` and fail the build if it doesn't pass.
fn self_verify(setup: &Path) -> Result<()> {
    let status = std::process::Command::new(setup)
        .arg("--verify")
        .status()
        .with_context(|| format!("run {} --verify", setup.display()))?;
    if !status.success() {
        bail!(
            "self-verify failed ({} --verify exited {}). The produced installer rejects its \
             own payload — most likely the prebuilt stub's compiled-in public key does not \
             match --priv-key, or the stub (installer.exe) was built without INSTALLER_PUB_KEY. \
             Rebuild installer.exe/uninstall.exe with INSTALLER_PUB_KEY set to the matching \
             pub.key, or drop --installer-stub/--uninstaller to let pack build the stub.",
            setup.display(),
            status.code().unwrap_or(-1)
        );
    }
    println!("Self-verify: OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmp_out_path_appends_tmp() {
        assert_eq!(
            tmp_out_path(Path::new("dist/setup.exe")),
            Path::new("dist/setup.exe.tmp")
        );
    }
}
