// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Append-only text log file with timestamp + level.
//!
//! Process-global via `OnceLock<Logger>`; once initialized further
//! `init()` calls are silently ignored. Safe to call `info/warn/error`
//! before init - those calls become no-ops. Every write is `flush`ed
//! immediately so a crashed install still leaves a complete log.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::utils;

static LOG: OnceLock<Logger> = OnceLock::new();

pub struct Logger {
    file: Mutex<Option<File>>,
    path: PathBuf,
}

#[derive(Copy, Clone)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

/// Open / create the log file. Re-init is silently ignored.
pub fn init(path: impl Into<PathBuf>) {
    let path = path.into();
    if LOG.get().is_some() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    let _ = LOG.set(Logger {
        file: Mutex::new(file),
        path,
    });
}

/// Where the active log is being written. `None` if `init` hasn't run
/// or the open failed.
pub fn current_path() -> Option<PathBuf> {
    LOG.get().map(|l| l.path.clone())
}

pub fn info(msg: impl AsRef<str>) {
    write_line(Level::Info, msg.as_ref());
}
pub fn warn(msg: impl AsRef<str>) {
    write_line(Level::Warn, msg.as_ref());
}
pub fn error(msg: impl AsRef<str>) {
    write_line(Level::Error, msg.as_ref());
}

fn write_line(lvl: Level, msg: &str) {
    let Some(logger) = LOG.get() else { return };
    let mut guard = match logger.file.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(file) = guard.as_mut() else { return };
    let ts = iso_utc(SystemTime::now());
    let line = format!("{} {} {}\n", ts, lvl.tag(), msg);
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` without pulling in chrono.
fn iso_utc(t: SystemTime) -> String {
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let ms = dur.subsec_millis();
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (y, mo, d) = utils::days_to_ymd(days);
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, mo, d, h, m, s, ms
    )
}

#[allow(dead_code)]
pub fn log_path_for_install(install_dir: &Path) -> PathBuf {
    install_dir.join("install.log")
}

/// Installer log in `%TEMP%`, named by product (so support can tell which app
/// failed) + PID (uniqueness across concurrent runs). Used as the *live* log
/// target so diagnostics survive even when the chosen install dir isn't
/// writable. Copied into the install dir on success.
#[allow(dead_code)]
pub fn log_path_installer_temp(product: &str, pid: u32) -> PathBuf {
    let name = crate::paths::sanitize_component(product);
    std::env::temp_dir().join(format!("{}-install-{}.log", name, pid))
}

#[allow(dead_code)]
pub fn log_path_for_uninstall(install_dir: &Path) -> PathBuf {
    install_dir.join("uninstall.log")
}

#[allow(dead_code)]
pub fn log_path_uninstall_temp(product: &str, pid: u32) -> PathBuf {
    let name = crate::paths::sanitize_component(product);
    std::env::temp_dir().join(format!("{}-uninstall-{}.log", name, pid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use crate::utils::days_to_ymd;

    #[test]
    fn days_to_ymd_known() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        assert_eq!(days_to_ymd(31), (1970, 2, 1));
        assert_eq!(days_to_ymd(365), (1971, 1, 1));
        assert_eq!(days_to_ymd(59), (1970, 3, 1)); // 1970 not leap
    }

    #[test]
    fn iso_utc_epoch_and_offset() {
        assert_eq!(iso_utc(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let t = UNIX_EPOCH + Duration::from_millis(1000 * 3661 + 42); // 01:01:01.042
        assert_eq!(iso_utc(t), "1970-01-01T01:01:01.042Z");
    }
}

/// Delete this product's `%TEMP%` install/uninstall logs older than
/// `max_age_days`, so they don't accumulate over a machine's lifetime.
/// Best-effort: any error (locked file, unreadable mtime) is ignored.
#[allow(dead_code)]
pub fn prune_temp_logs(product: &str, max_age_days: u64) {
    let name = crate::paths::sanitize_component(product);
    let install_prefix = format!("{}-install-", name);
    let uninstall_prefix = format!("{}-uninstall-", name);
    let max_age = std::time::Duration::from_secs(max_age_days * 24 * 60 * 60);
    let now = SystemTime::now();

    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        let ours = (fname.starts_with(&install_prefix) || fname.starts_with(&uninstall_prefix))
            && fname.ends_with(".log");
        if !ours {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok());
        if let Some(age) = age {
            if age > max_age {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
