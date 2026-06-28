// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! In-process elevation via a hidden worker subprocess.
//!
//! When a file operation fails with `ERROR_ACCESS_DENIED` (code 5, not a
//! transient AV/indexer lock which is `ERROR_SHARING_VIOLATION` code 32), the
//! same executable is relaunched as an elevated subprocess through UAC
//! (`runas`). The subprocess communicates via a named pipe: the main process
//! writes a JSON command; the worker streams JSON event lines back.

use crate::utils::wide;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, NAMED_PIPE_MODE, WaitNamedPipeW,
};
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::HSTRING;
use windows::core::PCWSTR;

// ─── permission helpers ───────────────────────────────────────────────────────

/// `ERROR_ACCESS_DENIED` (5) — a real permission refusal, distinct from
/// `ERROR_SHARING_VIOLATION` (32) which AV / indexers cause on locked files.
pub fn is_permission_denied(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(5)
}

/// `true` when the current process token has administrator privileges.
pub fn is_already_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

// ─── pipe naming ─────────────────────────────────────────────────────────────

pub fn pipe_name(pid: u32) -> String {
    format!(r"\\.\pipe\installway-elevation-{}", pid)
}

// ─── wire protocol ───────────────────────────────────────────────────────────

/// Command the main process sends to the elevated installer worker.
#[derive(Serialize, Deserialize)]
pub struct InstallWorkerCommand {
    pub install_dir: PathBuf,
    /// Plugin-page answers (`HashMap<plugin_name, BTreeMap<field, value>>`).
    pub plugin_inputs: HashMap<String, BTreeMap<String, String>>,
}

/// Events the elevated worker streams back to the main process.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    Progress { done: u64, total: u64, name: String },
    Done,
    Error { msg: String },
}

/// Serialize `value` as a single JSON line into `writer`.
pub fn send<T: Serialize, W: Write>(writer: &mut W, value: &T) -> Result<()> {
    let line = serde_json::to_string(value)?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Deserialize one JSON line from `reader`. Returns `None` on EOF.
pub fn recv<T: for<'de> Deserialize<'de>, R: BufRead>(reader: &mut R) -> Result<Option<T>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(line.trim_end())?))
}

// ─── server side (main process) ───────────────────────────────────────────────

/// Create a named-pipe server instance (byte-stream, blocking, duplex).
pub fn create_pipe_server(name: &str) -> Result<windows::Win32::Foundation::HANDLE> {
    // PIPE_ACCESS_DUPLEX = 3, byte-stream blocking mode.
    let name_w = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name_w.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(3), // PIPE_ACCESS_DUPLEX
            NAMED_PIPE_MODE(0),           // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
            1,
            8192,
            8192,
            5000,
            None,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        bail!(
            "CreateNamedPipeW failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(handle)
}

/// Block until the elevated worker connects (or the pipe errors out).
pub fn wait_for_client(handle: windows::Win32::Foundation::HANDLE) -> Result<()> {
    unsafe { ConnectNamedPipe(handle, None)? };
    Ok(())
}

// ─── client side (elevated worker) ───────────────────────────────────────────

/// Connect to the named-pipe server created by the main process.
pub fn connect_pipe_client(name: &str) -> Result<windows::Win32::Foundation::HANDLE> {
    let name_w = wide(name);
    // Wait up to 5 s for the server to be ready (process startup latency).
    let _ = unsafe { WaitNamedPipeW(PCWSTR(name_w.as_ptr()), 5000) };

    // GENERIC_READ | GENERIC_WRITE = 0xC0000000
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name_w.as_ptr()),
            0xC000_0000u32,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )?
    };
    Ok(handle)
}

// ─── handle → std::fs::File ───────────────────────────────────────────────────

/// Wrap a raw Windows pipe `HANDLE` into a `std::fs::File`.
///
/// # Safety
/// The handle must be valid, open, and owned by the caller. Ownership
/// transfers to the returned `File`; the handle is closed on drop.
pub fn open_pipe_handle(handle: windows::Win32::Foundation::HANDLE) -> std::fs::File {
    let owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    owned.into()
}

// ─── spawn elevated worker ────────────────────────────────────────────────────

/// Re-launch this executable as an elevated worker via UAC (`runas`).
///
/// The OS presents the UAC dialog. Returns `Ok(())` if the prompt was
/// approved and the worker process started. Returns `Err` if the user
/// cancelled UAC or an OS error occurred.
pub fn spawn_elevated_worker(pipe_name: &str) -> Result<()> {
    let exe = std::env::current_exe()?;
    let args = format!("--elevated-worker \"{}\"", pipe_name);

    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("runas"),
            &HSTRING::from(exe.to_str().unwrap_or("")),
            &HSTRING::from(args.as_str()),
            None,
            SW_SHOWNORMAL,
        )
    };

    if result.0 as usize > 32 {
        Ok(())
    } else {
        bail!("elevation cancelled or failed (code {})", result.0 as usize)
    }
}

// ─── byte-by-byte line reader for worker command parsing ─────────────────────

/// Read one `\n`-terminated line from `file` byte-by-byte.
/// Avoids buffering that could consume bytes intended for later reads.
pub fn read_line_unbuffered(file: &mut std::fs::File) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if buf[0] == b'\n' {
            break;
        }
        bytes.push(buf[0]);
    }
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Wrap `file` in a `BufReader` for reading streamed events.
pub fn event_reader(file: std::fs::File) -> BufReader<std::fs::File> {
    BufReader::new(file)
}
