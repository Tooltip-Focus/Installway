// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Button and worker logic for the wizard. `wndproc` dispatches `WM_COMMAND`
//! here; the install runs on a worker thread that posts progress/done/error
//! back to the UI thread.

use super::{
    BM_GETCHECK, ID_ACCEPT_CHK, ID_INSTALL_BTN, ID_LAUNCH_CHK, ID_PATH_EDIT, ID_PATH_WARN,
    ID_PATH_WARN_ICON, ID_PROGRESS, ID_STATUS, PAYLOAD, Phase, STATE, apply_phase, message_box, tr,
};
use crate::extract::{InstallCtx, install};
use crate::install as install_mod;
use crate::ui::helpers::{
    self, WM_APP_DONE, WM_APP_ERROR, WM_APP_PROGRESS, get_window_text, scale_progress,
    set_dlg_text, set_progress,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, MB_ICONWARNING, PostMessageW, SW_HIDE, SW_SHOW, SendMessageW, ShowWindow, WM_CLOSE,
};

pub(super) unsafe fn on_next(hwnd: HWND) {
    let phase = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| st.borrow().phase)
            .unwrap_or(Phase::License)
    });
    if phase == Phase::License {
        let accepted = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .map(|st| st.borrow().license_accepted)
                .unwrap_or(false)
        });
        if !accepted {
            unsafe { message_box(hwnd, &tr().get("install.must_accept"), MB_ICONWARNING) };
            return;
        }
        // No Choose page: "Next" installs straight away to the default path.
        if super::skip_path() {
            unsafe { on_install(hwnd) };
        } else {
            unsafe { apply_phase(hwnd, Phase::Choose) };
        }
    }
}

pub(super) unsafe fn on_back(hwnd: HWND) {
    let phase = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| st.borrow().phase)
            .unwrap_or(Phase::License)
    });
    if phase == Phase::Choose && !super::skip_license() {
        unsafe { apply_phase(hwnd, Phase::License) };
    }
}

pub(super) unsafe fn on_accept_toggle(hwnd: HWND) {
    let h = unsafe { GetDlgItem(Some(hwnd), ID_ACCEPT_CHK as i32).unwrap_or_default() };
    let state = unsafe { SendMessageW(h, BM_GETCHECK, None, None) };
    let checked = state.0 as u32 == BST_CHECKED.0;
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            st.borrow_mut().license_accepted = checked;
        }
    });
}

pub(super) unsafe fn on_browse(hwnd: HWND) {
    unsafe {
        if let Some(picked) = pick_folder_com(hwnd) {
            set_dlg_text(hwnd, ID_PATH_EDIT, &with_product_subdir(&picked));
        }
    }
}

/// Append the product name as a subfolder to a browsed parent folder
fn with_product_subdir(picked: &str) -> String {
    let product = PAYLOAD.with(|p| {
        p.borrow()
            .as_ref()
            .map(|p| p.product.trim().to_string())
            .unwrap_or_default()
    });
    if product.is_empty() {
        return picked.to_string();
    }
    let pb = PathBuf::from(picked);
    let already = pb
        .file_name()
        .map(|n| n.eq_ignore_ascii_case(product.as_str()))
        .unwrap_or(false);
    if already {
        picked.to_string()
    } else {
        pb.join(&product).to_string_lossy().into_owned()
    }
}

unsafe fn pick_folder_com(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
        CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SIGDN_FILESYSPATH,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let dialog: IFileOpenDialog =
            match CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) {
                Ok(d) => d,
                Err(_) => {
                    CoUninitialize();
                    return None;
                }
            };
        let _ = dialog.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        let _ = dialog.Show(Some(hwnd));
        let item_res: windows::core::Result<IShellItem> = dialog.GetResult();
        let result = match item_res {
            Ok(item) => match item.GetDisplayName(SIGDN_FILESYSPATH) {
                Ok(pwstr) => {
                    let s = pwstr.to_string().ok();
                    windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as *const _));
                    s
                }
                Err(_) => None,
            },
            Err(_) => None,
        };
        CoUninitialize();
        result
    }
}

/// True when `path` is an existing directory that already holds entries. A
/// missing path (the installer will create it) or an empty folder is safe.
fn dir_has_entries(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    std::fs::read_dir(Path::new(p))
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// Whether the install must be blocked because the destination is a non-empty
/// folder. The emptiness guard only applies to a fresh install where the user
/// actually picks the folder (`skip_path` false). When the path is fixed
/// (update/upgrade/patch over an existing install, or a build-time `skip_path`),
/// the destination legitimately already holds the product's own files, so the
/// check is skipped.
fn should_block_nonempty(skip_path: bool, path: &str) -> bool {
    !skip_path && dir_has_entries(path)
}

/// Re-evaluate the chosen folder: show/hide the non-empty warning and
/// enable/disable the Install button accordingly. Called when entering the
/// Choose page and on every edit of the path field.
pub(super) unsafe fn update_path_warning(hwnd: HWND) {
    let edit = unsafe { GetDlgItem(Some(hwnd), ID_PATH_EDIT as i32).unwrap_or_default() };
    let path = unsafe { get_window_text(edit) };
    let danger = dir_has_entries(&path);

    let warn = unsafe { GetDlgItem(Some(hwnd), ID_PATH_WARN as i32).unwrap_or_default() };
    let warn_icon = unsafe { GetDlgItem(Some(hwnd), ID_PATH_WARN_ICON as i32).unwrap_or_default() };
    let install_btn = unsafe { GetDlgItem(Some(hwnd), ID_INSTALL_BTN as i32).unwrap_or_default() };
    if danger {
        unsafe { set_dlg_text(hwnd, ID_PATH_WARN, &tr().get("install.path_not_empty")) };
    }
    unsafe {
        let vis = if danger { SW_SHOW } else { SW_HIDE };
        let _ = ShowWindow(warn, vis);
        let _ = ShowWindow(warn_icon, vis);
        let _ = EnableWindow(install_btn, !danger);
    }
}

pub(super) unsafe fn on_install(hwnd: HWND) {
    let edit = unsafe { GetDlgItem(Some(hwnd), ID_PATH_EDIT as i32).unwrap_or_default() };
    let path = unsafe { get_window_text(edit) };
    if path.trim().is_empty() {
        unsafe { message_box(hwnd, &tr().get("install.err_no_path"), MB_ICONWARNING) };
        return;
    }
    // Defensive: the Install button is disabled when the folder is non-empty,
    // but a default-button keypress could still reach here. See
    // `should_block_nonempty` for why the guard is skipped on a fixed path.
    if should_block_nonempty(super::skip_path(), &path) {
        unsafe { message_box(hwnd, &tr().get("install.path_not_empty"), MB_ICONWARNING) };
        return;
    }
    let pb = PathBuf::from(path.trim());

    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            st.borrow_mut().chosen_path = Some(pb.clone());
        }
    });

    unsafe { apply_phase(hwnd, Phase::Progress) };

    let shared = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| (st.borrow().cancel.clone(), st.borrow().progress.clone()))
    });
    let Some((cancel, progress_shared)) = shared else {
        return; // STATE not initialized - nothing to do.
    };
    let hwnd_isize = hwnd.0 as isize;

    thread::spawn(move || {
        let loaded = match crate::payload::load_and_verify() {
            Ok(l) => l,
            Err(e) => {
                push_error(hwnd_isize, &format!("{e}"));
                return;
            }
        };
        let progress_cb: Arc<dyn Fn(u64, u64, &str) + Send + Sync> = {
            let progress_shared = progress_shared.clone();
            Arc::new(move |done, total, name| {
                if let Ok(mut guard) = progress_shared.lock() {
                    guard.done = done;
                    guard.total = total;
                    guard.name = name.to_string();
                }
                helpers::post(hwnd_isize, WM_APP_PROGRESS);
            })
        };
        let ctx = InstallCtx {
            install_dir: pb.clone(),
            payload: &loaded.payload,
            zip_bytes: loaded.zip(),
            cancel: cancel.clone(),
            on_progress: progress_cb,
        };
        if let Err(e) = install(ctx) {
            push_error(hwnd_isize, &format!("{e}"));
            return;
        }
        if let Err(e) = install_mod::finalize(
            &pb,
            &loaded.payload,
            &loaded.uninstaller_bytes,
            loaded.zip(),
        ) {
            push_error(hwnd_isize, &format!("finalize: {e}"));
            return;
        }
        helpers::post(hwnd_isize, WM_APP_DONE);
    });
}

pub(super) unsafe fn on_cancel(hwnd: HWND) {
    let phase = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| st.borrow().phase)
            .unwrap_or(Phase::License)
    });
    match phase {
        Phase::License | Phase::Choose => {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        Phase::Progress => {
            STATE.with(|s| {
                if let Some(state) = s.borrow().as_ref() {
                    state.borrow().cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        _ => {}
    }
}

pub(super) unsafe fn on_finish(hwnd: HWND) {
    // If "Run now" is checked, launch the product before closing.
    let h = unsafe { GetDlgItem(Some(hwnd), ID_LAUNCH_CHK as i32).unwrap_or_default() };
    let checked = unsafe { SendMessageW(h, BM_GETCHECK, None, None) }.0 as u32 == BST_CHECKED.0;
    if checked {
        let path = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .and_then(|st| st.borrow().chosen_path.clone())
        });
        let exe = PAYLOAD.with(|p| {
            p.borrow()
                .as_ref()
                .map(|p| p.manifest.exe.clone())
                .unwrap_or_default()
        });
        if let Some(pb) = path {
            let _ = crate::install::launch_product(&pb, &exe);
        }
    }
    let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}

pub(super) unsafe fn update_progress(hwnd: HWND) {
    STATE.with(|s| {
        let Some(state) = s.borrow().as_ref().cloned() else {
            return;
        };
        let st = state.borrow();
        let (done, total, name) = match st.progress.lock() {
            Ok(guard) => (guard.done, guard.total, guard.name.clone()),
            Err(_) => (0, 0, String::new()),
        };
        let scaled = scale_progress(done, total);
        unsafe { set_progress(hwnd, ID_PROGRESS, scaled) };
        let pct = scaled / 100;
        let txt = if total > 0 {
            format!("{}%   ({} / {} bytes)\n{}", pct, done, total, name)
        } else {
            name
        };
        unsafe { set_dlg_text(hwnd, ID_STATUS, &txt) };
    });
}

fn push_error(hwnd_isize: isize, msg: &str) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            state.borrow_mut().error_text = msg.to_string();
        }
    });
    helpers::post(hwnd_isize, WM_APP_ERROR);
}

#[cfg(test)]
mod tests {
    use super::{dir_has_entries, should_block_nonempty};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn dir_has_entries_false_for_missing_path() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(!dir_has_entries(&missing.to_string_lossy()));
    }

    #[test]
    fn dir_has_entries_false_for_empty_or_blank() {
        let dir = tempdir().unwrap();
        assert!(!dir_has_entries(&dir.path().to_string_lossy()));
        assert!(!dir_has_entries(""));
        assert!(!dir_has_entries("   "));
    }

    #[test]
    fn dir_has_entries_true_when_populated() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        assert!(dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn fresh_install_blocks_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        // skip_path = false: the user picked this folder, it must be empty.
        assert!(should_block_nonempty(false, &path));
    }

    #[test]
    fn update_does_not_block_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("app.exe"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        assert!(!should_block_nonempty(true, &path));
    }

    #[test]
    fn fresh_install_allows_empty_folder() {
        let dir = tempdir().unwrap();
        assert!(!should_block_nonempty(false, &dir.path().to_string_lossy()));
    }
}
