// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Single-instance locks keyed by a folder, backed by named mutexes. The OS
//! destroys a mutex once its last handle closes (exit or crash), so a lock can
//! never go stale.

use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::PCWSTR;

/// Kernel object namespace of the mutex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Per logon session: only collides within the same session.
    Local,
    /// Machine-wide: collides across users and elevation levels.
    Global,
}

/// RAII handle on one named mutex.
pub struct NamedLock(HANDLE);

// The mutex handle is only closed on drop; safe to move across threads.
unsafe impl Send for NamedLock {}

impl Drop for NamedLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl NamedLock {
    /// Take the lock for `dir`. `None` when another process already holds it -
    /// including one whose mutex this process may not open (e.g. created by an
    /// elevated process).
    pub fn acquire(scope: Scope, kind: &str, dir: &Path) -> Option<Self> {
        let name = crate::utils::wide(&lock_name(scope, kind, dir));
        unsafe {
            let handle = match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
                Ok(h) if !h.is_invalid() => h,
                Ok(_) => return None,
                Err(e) => {
                    crate::log::warn(format!("lock {kind} unavailable: {e}"));
                    return None;
                }
            };
            // Read last error immediately, before any other Win32 call clobbers it.
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self(handle))
        }
    }

    /// Join the lock whether or not it is already held, keeping it alive across
    /// a hand-off: the receiving process joins before the holder exits, so no
    /// other process can slip in between.
    pub fn join(scope: Scope, kind: &str, dir: &Path) -> Option<Self> {
        let name = crate::utils::wide(&lock_name(scope, kind, dir));
        unsafe {
            CreateMutexW(None, false, PCWSTR(name.as_ptr()))
                .ok()
                .filter(|h| !h.is_invalid())
                .map(Self)
        }
    }
}

/// Mutex name for one folder. The path is normalized so different spellings of
/// the same folder collide, then hashed: mutex names can't contain `\` past the
/// namespace prefix and are capped at MAX_PATH.
fn lock_name(scope: Scope, kind: &str, dir: &Path) -> String {
    let key = dir
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase();
    let hash = crate::utils::bytes_blake3(key.as_bytes());
    let scope = match scope {
        Scope::Local => "Local",
        Scope::Global => "Global",
    };
    format!("{scope}\\Installway-{kind}-{}", &hash[..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_name_ignores_case_separators_and_trailing_slash() {
        let a = lock_name(Scope::Global, "Uninstall", Path::new(r"C:\Data\MyApp"));
        assert_eq!(
            a,
            lock_name(Scope::Global, "Uninstall", Path::new("c:/data/myapp/"))
        );
        assert_ne!(
            a,
            lock_name(Scope::Global, "Uninstall", Path::new(r"C:\Data\Other"))
        );
        assert_ne!(
            a,
            lock_name(Scope::Global, "Install", Path::new(r"C:\Data\MyApp"))
        );
        assert!(a.starts_with("Global\\Installway-Uninstall-"));
        assert!(
            lock_name(Scope::Local, "Install", Path::new(r"C:\Data\MyApp"))
                .starts_with("Local\\Installway-Install-")
        );
    }

    #[test]
    fn second_acquire_is_refused_until_the_last_handle_drops() {
        let dir = tempfile::tempdir().unwrap();
        let first = NamedLock::acquire(Scope::Local, "Test", dir.path()).expect("first acquire");
        assert!(NamedLock::acquire(Scope::Local, "Test", dir.path()).is_none());
        // A joined handle keeps the mutex alive after the original drops.
        let joined = NamedLock::join(Scope::Local, "Test", dir.path()).expect("join");
        drop(first);
        assert!(NamedLock::acquire(Scope::Local, "Test", dir.path()).is_none());
        drop(joined);
        assert!(NamedLock::acquire(Scope::Local, "Test", dir.path()).is_some());
    }
}
