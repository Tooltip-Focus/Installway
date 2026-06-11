// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Native DLL plugin host. A plugin exports the `installway_*` C ABI (see
//! `sdk/installway_plugin.h`) and runs in an **isolated child process** (the
//! installer/uninstaller re-launched with `--run-plugin`), so a crashing or
//! hanging plugin can't take down or stall the install — the child is killed
//! on timeout and a non-zero exit is just a failure.
//!
//! `host_main` is the child side (load DLL, call the function). `run_each` is
//! the parent side (verify each DLL's hash, spawn the child, enforce the
//! required/timeout policy).

use crate::models::PluginEntry;
use anyhow::{Context, Result, bail};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCWSTR, s};

/// ABI version this host speaks. Must match `INSTALLWAY_ABI_VERSION` in the SDK.
const ABI_VERSION: u32 = 1;

/// Per-plugin wall-clock budget; the child is killed past this.
const TIMEOUT: Duration = Duration::from_secs(600);

/// Context handed to a plugin run, serialized to a temp JSON file and passed to
/// the child by path.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct PluginCtx {
    pub install_dir: String,
    pub product: String,
    pub product_id: String,
    pub version: String,
    pub exe: String,
    pub log_path: String,
}

/// Write the context to a temp JSON file; returns its path.
pub fn write_ctx(ctx: &PluginCtx) -> Result<PathBuf> {
    let p = std::env::temp_dir().join(format!("iw-plugin-ctx-{}.json", std::process::id()));
    std::fs::write(&p, serde_json::to_string(ctx)?)
        .with_context(|| format!("write plugin context {}", p.display()))?;
    Ok(p)
}

/// Parent side: for each `(entry, dll-on-disk)`, verify the DLL hash, then run
/// `func` (`"up"` or `"down"`) in a child process. With `enforce_required`, a
/// required plugin's failure is an error; otherwise failures are logged and
/// skipped (used for uninstall `down`, which must stay robust).
pub fn run_each(
    self_exe: &Path,
    ctx_path: &Path,
    items: &[(PluginEntry, PathBuf)],
    func: &str,
    enforce_required: bool,
) -> Result<()> {
    for (entry, dll) in items {
        let bytes =
            std::fs::read(dll).with_context(|| format!("read plugin dll {}", dll.display()))?;
        if crate::utils::bytes_blake3(&bytes) != entry.blake3 {
            let m = format!("plugin '{}' hash mismatch - refusing to load", entry.name);
            if enforce_required && entry.required {
                bail!("{m}");
            }
            crate::log::warn(m);
            continue;
        }
        crate::log::info(format!("plugin '{}': {}", entry.name, func));
        let ok = spawn(self_exe, dll, func, ctx_path)?;
        if !ok {
            let m = format!("plugin '{}' {} returned failure", entry.name, func);
            if enforce_required && entry.required {
                bail!("{m}");
            }
            crate::log::warn(format!("{m} (continuing)"));
        }
    }
    Ok(())
}

/// Spawn `self_exe --run-plugin <dll> <func> <ctx>` and wait, killing it past
/// the timeout. Returns whether it exited 0.
fn spawn(self_exe: &Path, dll: &Path, func: &str, ctx_path: &Path) -> Result<bool> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut child = Command::new(self_exe)
        .arg("--run-plugin")
        .arg(dll)
        .arg(func)
        .arg(ctx_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("spawn plugin host for {}", dll.display()))?;

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            bail!("plugin timed out after {}s", TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- Child side ---------------------------------------------------------

// Log sink for the plugin's `log` callback; set for the duration of one call.
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

#[repr(C)]
struct CContext {
    abi_version: u32,
    install_dir: *const u16,
    product: *const u16,
    product_id: *const u16,
    version: *const u16,
    exe: *const u16,
    log: extern "system" fn(*const u16, *const u16),
}

type AbiFn = unsafe extern "system" fn() -> u32;
type ActionFn = unsafe extern "system" fn(*const CContext) -> i32;

/// Child entry point: load `dll`, check its ABI version, call `installway_up`
/// or `installway_down`, and return its exit code (non-zero on any failure).
pub fn host_main(dll: &Path, func: &str, ctx_path: &Path) -> i32 {
    let ctx: PluginCtx = match std::fs::read_to_string(ctx_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(c) => c,
        None => return 102,
    };
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ctx.log_path)
    {
        *LOG.lock().unwrap() = Some(f);
    }
    let code = unsafe { call(dll, func, &ctx) };
    *LOG.lock().unwrap() = None;
    code
}

unsafe fn call(dll: &Path, func: &str, ctx: &PluginCtx) -> i32 {
    let wdll = wide(&dll.as_os_str().to_string_lossy());
    let hmod = match unsafe { LoadLibraryW(PCWSTR(wdll.as_ptr())) } {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            write_log("ERROR", "LoadLibrary failed");
            return 110;
        }
    };

    let result = unsafe { call_loaded(hmod, func, ctx) };
    unsafe { drop_module(hmod) };
    result
}

unsafe fn call_loaded(hmod: HMODULE, func: &str, ctx: &PluginCtx) -> i32 {
    let Some(abi_ptr) = (unsafe { GetProcAddress(hmod, s!("installway_abi_version")) }) else {
        write_log("ERROR", "plugin missing installway_abi_version");
        return 111;
    };
    let abi_fn: AbiFn = unsafe { std::mem::transmute(abi_ptr) };
    let v = unsafe { abi_fn() };
    if v != ABI_VERSION {
        write_log(
            "ERROR",
            &format!("ABI mismatch: plugin {v}, host {ABI_VERSION}"),
        );
        return 112;
    }

    let name = if func == "down" {
        s!("installway_down")
    } else {
        s!("installway_up")
    };
    let Some(act_ptr) = (unsafe { GetProcAddress(hmod, name) }) else {
        write_log("ERROR", &format!("plugin missing installway_{func}"));
        return 113;
    };
    let act: ActionFn = unsafe { std::mem::transmute(act_ptr) };

    // Wide strings must outlive the call.
    let install_dir = wide(&ctx.install_dir);
    let product = wide(&ctx.product);
    let product_id = wide(&ctx.product_id);
    let version = wide(&ctx.version);
    let exe = wide(&ctx.exe);
    let c = CContext {
        abi_version: ABI_VERSION,
        install_dir: install_dir.as_ptr(),
        product: product.as_ptr(),
        product_id: product_id.as_ptr(),
        version: version.as_ptr(),
        exe: exe.as_ptr(),
        log: log_cb,
    };
    unsafe { act(&c) }
}

unsafe fn drop_module(hmod: HMODULE) {
    let _ = unsafe { FreeLibrary(hmod) };
}

extern "system" fn log_cb(level: *const u16, msg: *const u16) {
    write_log(&wide_to_string(level), &wide_to_string(msg));
}

fn write_log(level: &str, msg: &str) {
    use std::io::Write;
    if let Ok(mut g) = LOG.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = writeln!(f, "{level:<5} [plugin] {msg}");
        let _ = f.flush();
    }
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: callers pass null-terminated wide strings from the SDK.
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
