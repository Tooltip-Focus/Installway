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
use crate::utils::wide;
use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    ReadFile, WriteFile,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::{PCWSTR, s};

/// `CreateFileW` desired-access for the write end of the descriptor pipe.
const GENERIC_WRITE: u32 = 0x4000_0000;

/// ABI version the host speaks; a plugin must report the same.
const ABI_VERSION: u32 = 1;

/// Per-plugin wall-clock budget; the child is killed past this.
const TIMEOUT: Duration = Duration::from_secs(600);

/// Run context, sent to the child as JSON on stdin.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct PluginCtx {
    pub install_dir: String,
    /// Per-user data dir (where `installer_info.json` lives). The place for plugin
    /// state that should persist across upgrades; the uninstaller deletes it.
    #[serde(default)]
    pub data_dir: String,
    pub product: String,
    pub product_id: String,
    pub version: String,
    pub exe: String,
    pub log_path: String,
    /// `up` only: the user's page answers, keyed `"<page_id>.<widget_id>"`.
    #[serde(default)]
    pub inputs_json: String,
}

/// Collected page answers per plugin name; each value becomes that plugin's
/// `ctx.inputs_json`.
pub type InputsByPlugin = std::collections::HashMap<String, crate::models::PluginInputs>;

/// Run `func` (`"up"`/`"down"`) for each plugin in its own child process,
/// passing that plugin's `inputs_json`. With `enforce_required`, a required
/// plugin's failure aborts; otherwise it's logged (uninstall `down` stays
/// best-effort).
pub fn run_each(
    self_exe: &Path,
    base_ctx: &PluginCtx,
    items: &[(PluginEntry, PathBuf, String)],
    func: &str,
    enforce_required: bool,
) -> Result<()> {
    for (entry, dll, inputs_json) in items {
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
        let mut ctx = base_ctx.clone();
        ctx.inputs_json = inputs_json.clone();
        let ctx_json = serde_json::to_string(&ctx)?;
        let (ok, _descriptor) = run_child(self_exe, dll, func, &ctx_json, false, None)?;
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

/// Run `installway_up` for a single plugin during wizard time (called when a
/// plugin page has `buttons: false`). Hash-verified, same child-process
/// isolation as `run_each`. Required failures are errors; non-required are
/// logged. `on_progress` drives a deterministic bar when the page has
/// `marquee: false`.
pub fn run_up_single(
    self_exe: &Path,
    base_ctx: &PluginCtx,
    entry: &PluginEntry,
    dll: &Path,
    inputs_json: &str,
    on_progress: Option<Box<dyn Fn(u32) + Send>>,
) -> Result<()> {
    let bytes = std::fs::read(dll).with_context(|| format!("read plugin dll {}", dll.display()))?;
    if crate::utils::bytes_blake3(&bytes) != entry.blake3 {
        let m = format!("plugin '{}' hash mismatch - refusing to load", entry.name);
        if entry.required {
            bail!("{m}");
        }
        crate::log::warn(m);
        return Ok(());
    }
    crate::log::info(format!("plugin '{}': up (wizard)", entry.name));
    let mut ctx = base_ctx.clone();
    ctx.inputs_json = inputs_json.to_string();
    let ctx_json = serde_json::to_string(&ctx)?;
    let (ok, _) = run_child(self_exe, dll, "up", &ctx_json, false, on_progress)?;
    if !ok {
        let m = format!("plugin '{}' up returned failure", entry.name);
        if entry.required {
            bail!("{m}");
        }
        crate::log::warn(format!("{m} (continuing)"));
    }
    Ok(())
}

/// Run one step of a `ui = true` plugin's wizard: hand it the answers collected
/// so far (`answers_json`, a JSON object) and parse the [`PageStep`] it emits
/// over the pipe. Errors are returned (never panic) so the caller can log and
/// skip — a bad step must not block the wizard.
pub fn query_step(
    self_exe: &Path,
    base_ctx: &PluginCtx,
    entry: &PluginEntry,
    dll: &Path,
    answers_json: &str,
) -> Result<crate::models::PageStep> {
    let bytes = std::fs::read(dll).with_context(|| format!("read plugin dll {}", dll.display()))?;
    if crate::utils::bytes_blake3(&bytes) != entry.blake3 {
        bail!("plugin '{}' hash mismatch - refusing to load", entry.name);
    }
    let mut ctx = base_ctx.clone();
    ctx.inputs_json = answers_json.to_string();
    let ctx_json = serde_json::to_string(&ctx)?;
    let (ok, descriptor) = run_child(self_exe, dll, "pages", &ctx_json, true, None)?;
    if !ok {
        bail!("plugin '{}' page step returned failure", entry.name);
    }
    serde_json::from_str(&descriptor).context("parse plugin page step")
}

/// Spawn the plugin host child, send `ctx_json` on its stdin, and wait (killing
/// it past the timeout). `want_descriptor` sets up the named pipe for page
/// descriptors. `on_progress`, when `Some`, sets up a second pipe for 0–100
/// progress values (called for each `emit_progress` the plugin fires).
/// Returns `(exited 0, descriptor)`.
fn run_child(
    self_exe: &Path,
    dll: &Path,
    func: &str,
    ctx_json: &str,
    want_descriptor: bool,
    on_progress: Option<Box<dyn Fn(u32) + Send>>,
) -> Result<(bool, String)> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let pipe = if want_descriptor {
        Some(make_pages_pipe()?)
    } else {
        None
    };
    let progress_pipe = if on_progress.is_some() {
        Some(make_progress_pipe()?)
    } else {
        None
    };

    let mut cmd = Command::new(self_exe);
    cmd.arg("--run-plugin").arg(dll).arg(func);
    // Always push the pages arg when a progress arg follows, so positional order
    // stays stable (child reads: [idx+3]=pages, [idx+4]=progress).
    if pipe.is_some() || progress_pipe.is_some() {
        cmd.arg(pipe.as_ref().map(|p| p.name.as_str()).unwrap_or(""));
    }
    if let Some(p) = &progress_pipe {
        cmd.arg(&p.name);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("spawn plugin host for {}", dll.display()))?;

    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(ctx_json.as_bytes())
    {
        crate::log::warn(format!("plugin host: stdin write failed: {e}"));
    }

    let pages_server = pipe.as_ref().map(|p| p.server);
    let pages_reader = pipe.map(|p| {
        let raw = p.server.0 as isize;
        std::thread::spawn(move || read_pages_pipe(raw))
    });

    let progress_server = progress_pipe.as_ref().map(|p| p.server);
    let progress_reader = progress_pipe.map(|p| {
        let raw = p.server.0 as isize;
        let cb = on_progress.unwrap();
        std::thread::spawn(move || read_progress_pipe(raw, cb))
    });

    let start = Instant::now();
    let mut timed_out = false;
    let mut success = false;
    loop {
        if let Some(status) = child.try_wait()? {
            success = status.success();
            break;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    for h in [pages_server, progress_server].into_iter().flatten() {
        unsafe {
            let _ = CancelIoEx(h, None);
        }
    }
    let descriptor = if let Some(r) = pages_reader {
        let out = r.join().unwrap_or_default();
        if let Some(h) = pages_server {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
        out
    } else {
        String::new()
    };
    if let Some(r) = progress_reader {
        let _ = r.join();
        if let Some(h) = progress_server {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }

    if timed_out {
        bail!("plugin timed out after {}s", TIMEOUT.as_secs());
    }
    Ok((success, descriptor))
}

struct PagesPipe {
    server: HANDLE,
    name: String,
}

struct ProgressPipe {
    server: HANDLE,
    name: String,
}

/// Create an inbound named pipe with a process-unique name for one `pages` run.
fn make_pages_pipe() -> Result<PagesPipe> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        r"\\.\pipe\installway-pages-{}-{}",
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed)
    );
    let wname = wide(&name);
    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(wname.as_ptr()),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,         // one instance — one child per pipe
            0,         // out buffer (host only reads)
            64 * 1024, // in buffer
            0,         // default timeout (unused without WaitNamedPipe)
            None,
        )
    };
    if server == INVALID_HANDLE_VALUE {
        bail!("create descriptor pipe failed");
    }
    Ok(PagesPipe { server, name })
}

fn make_progress_pipe() -> Result<ProgressPipe> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        r"\\.\pipe\installway-progress-{}-{}",
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed)
    );
    let wname = wide(&name);
    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(wname.as_ptr()),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            0,
            256,
            0,
            None,
        )
    };
    if server == INVALID_HANDLE_VALUE {
        bail!("create progress pipe failed");
    }
    Ok(ProgressPipe { server, name })
}

/// Read 4-byte progress values from the pipe and call `on_progress` for each.
fn read_progress_pipe(server_raw: isize, on_progress: Box<dyn Fn(u32) + Send>) {
    let h = HANDLE(server_raw as *mut core::ffi::c_void);
    unsafe {
        let _ = ConnectNamedPipe(h, None);
        let mut buf = [0u8; 4];
        loop {
            let mut read = 0u32;
            match ReadFile(h, Some(&mut buf), Some(&mut read), None) {
                Ok(()) if read == 4 => on_progress(u32::from_le_bytes(buf)),
                _ => break,
            }
        }
    }
}

/// Accept the child and read the descriptor it writes, to EOF. Returns the bytes
/// as a (lossy) UTF-8 string; empty on any error. The caller (main thread) owns
/// and closes the handle.
fn read_pages_pipe(server_raw: isize) -> String {
    let h = HANDLE(server_raw as *mut core::ffi::c_void);
    unsafe {
        // Ok, ERROR_PIPE_CONNECTED (already connected), or cancelled — read either way.
        let _ = ConnectNamedPipe(h, None);
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut read = 0u32;
            match ReadFile(h, Some(&mut buf), Some(&mut read), None) {
                Ok(()) if read > 0 => out.extend_from_slice(&buf[..read as usize]),
                _ => break, // 0 bytes or broken pipe = EOF
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

// ---- Child side ---------------------------------------------------------

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
static PAGES: Mutex<Option<String>> = Mutex::new(None);
// Write end of the progress pipe as raw isize (HANDLE is !Send); set by `host_main`.
static PROGRESS_PIPE: Mutex<Option<isize>> = Mutex::new(None);

#[repr(C)]
struct CContext {
    abi_version: u32,
    install_dir: *const u16,
    data_dir: *const u16,
    product: *const u16,
    product_id: *const u16,
    version: *const u16,
    exe: *const u16,
    log: extern "system" fn(*const u16, *const u16),
    inputs_json: *const u16,
    emit_pages: extern "system" fn(*const u16),
    /// Null when the parent didn't open a progress pipe (marquee mode).
    emit_progress: Option<extern "system" fn(u32)>,
}

extern "system" fn emit_pages_cb(json: *const u16) {
    if let Ok(mut g) = PAGES.lock() {
        if g.is_some() {
            write_log(
                "WARN",
                "emit_pages called more than once; first descriptor dropped",
            );
        }
        *g = Some(wide_to_string(json));
    }
}

extern "system" fn emit_progress_cb(value: u32) {
    let Ok(g) = PROGRESS_PIPE.lock() else {
        return;
    };
    let Some(raw) = *g else {
        return;
    };

    let h = HANDLE(raw as *mut core::ffi::c_void);
    let bytes = value.clamp(0, 100).to_le_bytes();
    let mut written = 0u32;
    unsafe {
        let _ = WriteFile(h, Some(&bytes), Some(&mut written), None);
    }
}

type AbiFn = unsafe extern "system" fn() -> u32;
type ActionFn = unsafe extern "system" fn(*const CContext) -> i32;

/// Child entry point: read the context JSON from stdin, load `dll`, check its
/// ABI version, call `installway_{up,down,pages}`, and return its exit code.
/// `pipe_name`: descriptor pipe for `pages`. `progress_pipe_name`: progress
/// pipe for `up` with a deterministic bar.
pub fn host_main(
    dll: &Path,
    func: &str,
    pipe_name: Option<&str>,
    progress_pipe_name: Option<&str>,
) -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 101;
    }
    let ctx: PluginCtx = match serde_json::from_str(&input) {
        Ok(c) => c,
        Err(_) => return 102,
    };
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ctx.log_path)
    {
        *LOG.lock().unwrap() = Some(f);
    }
    *PAGES.lock().unwrap() = None;

    let pipe = pipe_name.and_then(open_pages_client);
    *PROGRESS_PIPE.lock().unwrap() = progress_pipe_name
        .and_then(open_pages_client)
        .map(|h| h.0 as isize);

    let code = unsafe { call(dll, func, &ctx) };

    if let Some(h) = pipe {
        let json = PAGES.lock().unwrap().take().unwrap_or_default();
        unsafe {
            if !json.is_empty() {
                let mut written = 0u32;
                let _ = WriteFile(h, Some(json.as_bytes()), Some(&mut written), None);
            }
            let _ = CloseHandle(h);
        }
    }
    if let Some(raw) = PROGRESS_PIPE.lock().unwrap().take() {
        unsafe {
            let _ = CloseHandle(HANDLE(raw as *mut core::ffi::c_void));
        }
    }

    *LOG.lock().unwrap() = None;
    code
}

/// Open the write end of the descriptor pipe by name. `None` if it can't connect
/// (the parent then reads an empty descriptor and skips the plugin's pages).
fn open_pages_client(name: &str) -> Option<HANDLE> {
    let wname = wide(name);
    let h = unsafe {
        CreateFileW(
            PCWSTR(wname.as_ptr()),
            GENERIC_WRITE,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    match h {
        Ok(h) if !h.is_invalid() => Some(h),
        _ => None,
    }
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

    let name = match func {
        "down" => s!("installway_down"),
        "pages" => s!("installway_pages"),
        _ => s!("installway_up"),
    };
    let Some(act_ptr) = (unsafe { GetProcAddress(hmod, name) }) else {
        write_log("ERROR", &format!("plugin missing installway_{func}"));
        return if func == "pages" { 114 } else { 113 };
    };
    let act: ActionFn = unsafe { std::mem::transmute(act_ptr) };

    // Wide strings must outlive the call. A null `inputs_json` signals "no
    // answers" to the plugin (per the SDK header): set only for `up`.
    let install_dir = wide(&ctx.install_dir);
    let data_dir = wide(&ctx.data_dir);
    let product = wide(&ctx.product);
    let product_id = wide(&ctx.product_id);
    let version = wide(&ctx.version);
    let exe = wide(&ctx.exe);
    let inputs = wide(&ctx.inputs_json);
    let inputs_ptr = if ctx.inputs_json.is_empty() {
        std::ptr::null()
    } else {
        inputs.as_ptr()
    };
    let has_progress = PROGRESS_PIPE.lock().map(|g| g.is_some()).unwrap_or(false);
    let c = CContext {
        abi_version: ABI_VERSION,
        install_dir: install_dir.as_ptr(),
        data_dir: data_dir.as_ptr(),
        product: product.as_ptr(),
        product_id: product_id.as_ptr(),
        version: version.as_ptr(),
        exe: exe.as_ptr(),
        log: log_cb,
        inputs_json: inputs_ptr,
        emit_pages: emit_pages_cb,
        emit_progress: if has_progress {
            Some(emit_progress_cb)
        } else {
            None
        },
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
    if let Ok(mut g) = LOG.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = writeln!(f, "{level:<5} [plugin] {msg}");
        let _ = f.flush();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A v1-era context JSON (no `inputs_json`) still parses, defaulting it to
    /// empty. Guards the uninstaller `down` ctx and any older record.
    #[test]
    fn ctx_parses_without_inputs() {
        let j = r#"{"install_dir":"C:\\app","product":"P","product_id":"P_id",
                    "version":"1.0","exe":"C:\\app\\p.exe","log_path":"C:\\t.log"}"#;
        let c: PluginCtx = serde_json::from_str(j).unwrap();
        assert_eq!(c.product, "P");
        assert!(c.inputs_json.is_empty());
    }

    /// `inputs_json` round-trips.
    #[test]
    fn ctx_inputs_round_trip() {
        let c = PluginCtx {
            inputs_json: r#"{"region.country":"FR"}"#.into(),
            ..Default::default()
        };
        let back: PluginCtx = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.inputs_json, c.inputs_json);
    }
}
