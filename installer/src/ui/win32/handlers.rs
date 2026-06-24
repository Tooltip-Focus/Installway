// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Button and worker logic for the wizard. `wndproc` dispatches `WM_COMMAND`
//! here; the install runs on a worker thread that posts progress/done/error
//! back to the UI thread.

use super::{
    BM_GETCHECK, ID_ACCEPT_CHK, ID_BACK_BTN, ID_INSTALL_BTN, ID_LAUNCH_CHK, ID_NEXT_BTN,
    ID_PATH_EDIT, ID_PATH_WARN, ID_PATH_WARN_ICON, ID_PROGRESS, ID_STATUS, PAYLOAD, Phase, STATE,
    WIZARD, apply_phase, message_box, tr,
};
use crate::extract::{InstallCtx, install};
use crate::install as install_mod;
use crate::ui::helpers::{
    self, WM_APP_CANCELLED, WM_APP_DONE, WM_APP_ERROR, WM_APP_PERM_DENIED, WM_APP_PERM_ERROR,
    WM_APP_PLUGIN_PROGRESS, WM_APP_PLUGIN_STEP, WM_APP_PROGRESS, get_window_text, post_wparam,
    scale_progress, set_dlg_text, set_progress,
};
use common::model::install_dir_restriction::InstallDirRestriction;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, MB_ICONWARNING, PostMessageW, SW_HIDE, SW_SHOW, SendMessageW, ShowWindow, WM_CLOSE,
};

fn current_phase() -> Phase {
    STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| st.borrow().phase)
            .unwrap_or(Phase::License)
    })
}

/// Stores the result of a background plugin-step query until the UI thread picks
/// it up from `WM_APP_PLUGIN_STEP`.
static QUERIED_STEP: Mutex<Option<super::plugin_pages::StepOutcome>> = Mutex::new(None);

/// True while a background step query is running. Guards against a second
/// dispatch before the first completes — the Choose-page primary button stays
/// enabled until `WM_APP_PLUGIN_STEP` flips the phase, so a double-click would
/// otherwise spawn two query threads racing on `QUERIED_STEP`.
static QUERY_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Whether the active wizard is the canned (preview) variant, which steps
/// synchronously instead of dispatching a background query.
fn wizard_is_canned() -> bool {
    WIZARD.with(|w| w.borrow().as_ref().map(|z| z.is_canned()).unwrap_or(false))
}

/// Run a synchronous wizard transition `f` (if a wizard exists) and apply the
/// resulting step.
unsafe fn run_wizard_step(
    hwnd: HWND,
    f: impl FnOnce(&mut super::plugin_pages::Wizard, HWND) -> super::plugin_pages::Step,
) {
    let step = WIZARD.with(|w| w.borrow_mut().as_mut().map(|z| f(z, hwnd)));
    if let Some(step) = step {
        unsafe { act_step(hwnd, step) };
    }
}

pub(super) unsafe fn on_next(hwnd: HWND) {
    match current_phase() {
        Phase::License => {
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
            // Advance to the Choose page; or, when it is skipped, into the plugin
            // wizard, or straight to install when there are no plugins.
            if !super::skip_path() {
                unsafe { apply_phase(hwnd, Phase::Choose) };
            } else if super::has_plugin_pages() {
                unsafe { begin_plugin_wizard(hwnd) };
            } else {
                unsafe { on_install(hwnd) };
            }
        }
        // Collect this page's answers, then query the next step (off UI thread).
        Phase::Plugin => {
            if wizard_is_canned() {
                // Canned preview: synchronous path (no subprocess; instant).
                unsafe { run_wizard_step(hwnd, |z, h| z.forward(h)) };
            } else {
                let ok = WIZARD.with(|w| {
                    w.borrow_mut()
                        .as_mut()
                        .map(|z| unsafe { z.collect_page(hwnd) })
                        .unwrap_or(true)
                });
                if ok {
                    unsafe { dispatch_plugin_query(hwnd) };
                }
            }
        }
        _ => {}
    }
}

pub(super) unsafe fn on_back(hwnd: HWND) {
    match current_phase() {
        Phase::Choose if !super::skip_license() => unsafe { apply_phase(hwnd, Phase::License) },
        Phase::Plugin => unsafe { run_wizard_step(hwnd, |z, _| z.back()) },
        _ => {}
    }
}

/// Enter the plugin wizard (its first step). Canned (preview) path is
/// synchronous; real plugins dispatch a background query.
pub(super) unsafe fn begin_plugin_wizard(hwnd: HWND) {
    if wizard_is_canned() {
        unsafe { run_wizard_step(hwnd, |z, h| z.start(h)) };
    } else {
        unsafe { dispatch_plugin_query(hwnd) };
    }
}

/// Spawn a background thread to run the next plugin-page query. Disables the
/// nav buttons for the duration; re-enables them in `on_plugin_step`.
unsafe fn dispatch_plugin_query(hwnd: HWND) {
    let args = WIZARD.with(|w| w.borrow().as_ref().and_then(|z| z.step_args()));
    let Some(args) = args else {
        unsafe { act_step(hwnd, super::plugin_pages::Step::Install) };
        return;
    };
    // Ignore a re-entrant dispatch while a query is already running.
    if QUERY_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe { set_nav_enabled(hwnd, false) };
    let hwnd_isize = hwnd.0 as isize;
    std::thread::spawn(move || {
        let outcome = super::plugin_pages::run_step_query(args);
        *QUERIED_STEP.lock().unwrap() = Some(outcome);
        helpers::post(hwnd_isize, WM_APP_PLUGIN_STEP);
    });
}

/// Enable/disable the wizard nav buttons (Next/Back and the Choose-page primary
/// button, which is labeled "Next" while plugin pages are pending).
unsafe fn set_nav_enabled(hwnd: HWND, on: bool) {
    unsafe {
        for id in [ID_NEXT_BTN, ID_BACK_BTN, ID_INSTALL_BTN] {
            let h = GetDlgItem(Some(hwnd), id as i32).unwrap_or_default();
            let _ = EnableWindow(h, on);
        }
    }
}

/// Called on the UI thread when `WM_APP_PLUGIN_STEP` arrives.
pub(super) unsafe fn on_plugin_step(hwnd: HWND) {
    QUERY_IN_FLIGHT.store(false, Ordering::SeqCst);
    let outcome = QUERIED_STEP.lock().unwrap().take();
    let Some(outcome) = outcome else { return };
    unsafe { set_nav_enabled(hwnd, true) };
    let step = WIZARD.with(|w| {
        w.borrow_mut()
            .as_mut()
            .map(|z| unsafe { z.apply_step_outcome(hwnd, outcome) })
    });
    if let Some(step) = step {
        unsafe { act_step(hwnd, step) };
    }
}

/// Apply a wizard transition. Runs after the `WIZARD` borrow is released, so it
/// may re-borrow it (e.g. `apply_phase` reads the current slot).
unsafe fn act_step(hwnd: HWND, step: super::plugin_pages::Step) {
    use super::plugin_pages::Step;
    match step {
        Step::Show => unsafe { apply_phase(hwnd, Phase::Plugin) },
        Step::Install => unsafe { commit_install(hwnd) },
        Step::Stay => {}
        Step::Exit => unsafe {
            if !super::skip_path() {
                apply_phase(hwnd, Phase::Choose);
            } else if !super::skip_license() {
                apply_phase(hwnd, Phase::License);
            }
        },
        Step::AutoRun { marquee } => unsafe {
            apply_phase(hwnd, Phase::Plugin);
            super::plugin_pages::apply_auto_run(hwnd, marquee);
            dispatch_plugin_run(hwnd, marquee);
        },
    }
}

/// Spawn a background thread that calls the current plugin's `installway_up`
/// then queries the next wizard step. Posts `WM_APP_PLUGIN_STEP` on completion.
/// When `!marquee`, wires a progress callback that posts `WM_APP_PLUGIN_PROGRESS`.
unsafe fn dispatch_plugin_run(hwnd: HWND, marquee: bool) {
    let args = WIZARD.with(|w| w.borrow().as_ref().and_then(|z| z.step_args()));
    let Some(mut args) = args else {
        unsafe { act_step(hwnd, super::plugin_pages::Step::Install) };
        return;
    };
    if QUERY_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    if !marquee {
        let hwnd_isize = hwnd.0 as isize;
        args.on_progress = Some(Box::new(move |v| {
            post_wparam(hwnd_isize, WM_APP_PLUGIN_PROGRESS, v as usize);
        }));
    }
    let hwnd_isize = hwnd.0 as isize;
    std::thread::spawn(move || {
        let outcome = super::plugin_pages::run_plugin_then_step(args);
        *QUERIED_STEP.lock().unwrap() = Some(outcome);
        helpers::post(hwnd_isize, WM_APP_PLUGIN_STEP);
    });
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

/// True when `path` is an existing directory that contains at least one file
/// (at any depth). A missing path, a truly empty folder, or a folder that
/// contains only empty sub-directories are all considered safe — the installer
/// will create or populate them without clobbering user data.
fn dir_has_entries(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    dir_has_files_recursive(Path::new(p))
}

fn dir_has_files_recursive(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let ep = entry.path();
        if ep.is_file() {
            return true;
        }
        if ep.is_dir() && dir_has_files_recursive(&ep) {
            return true;
        }
    }
    false
}

/// Normalize a Windows path for a tolerant equality test: trim, unify slashes.
fn norm_dir(p: &str) -> String {
    p.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// Whether `path` points at the same directory as `default_path`.
fn same_dir(path: &str, default_path: &str) -> bool {
    !path.trim().is_empty() && norm_dir(path) == norm_dir(default_path)
}

/// Whether the install must be blocked because the destination is a non-empty
/// folder. The emptiness guard only applies to a fresh install where the user
/// actually picks the folder (`skip_path` false). When the path is fixed
/// (update/upgrade/patch over an existing install, or a build-time `skip_path`),
/// the destination legitimately already holds the product's own files, so the
/// check is skipped.
///
/// `restriction` (build-time, signed) can relax the guard for apps that install
/// over an existing layout (e.g. replacing a legacy InstallShield/MSI install):
/// `Bypass` allows any non-empty folder; `DefaultDirOnly` allows it only when
/// the chosen folder is still the proposed `default_path`.
fn should_block_nonempty(
    restriction: InstallDirRestriction,
    skip_path: bool,
    default_path: &str,
    path: &str,
) -> bool {
    if skip_path || !dir_has_entries(path) {
        return false;
    }
    match restriction {
        InstallDirRestriction::Bypass => false,
        InstallDirRestriction::DefaultDirOnly => !same_dir(path, default_path),
        InstallDirRestriction::Enforce => true,
    }
}

/// Re-evaluate the chosen folder: show/hide the non-empty warning and
/// enable/disable the Install button accordingly. Called when entering the
/// Choose page and on every edit of the path field.
pub(super) unsafe fn update_path_warning(hwnd: HWND) {
    let edit = unsafe { GetDlgItem(Some(hwnd), ID_PATH_EDIT as i32).unwrap_or_default() };
    let path = unsafe { get_window_text(edit) };
    let danger = should_block_nonempty(
        super::restriction(),
        super::skip_path(),
        &super::default_path(),
        &path,
    );

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

/// Read the path edit, validate it, and store it as the chosen path. Returns the
/// validated path, or `None` (after warning the user) for an empty entry or a
/// non-empty target folder. The edit holds the value even while hidden, so this
/// works from a later plugin page too.
unsafe fn validate_and_store_path(hwnd: HWND) -> Option<PathBuf> {
    let edit = unsafe { GetDlgItem(Some(hwnd), ID_PATH_EDIT as i32).unwrap_or_default() };
    let path = unsafe { get_window_text(edit) };
    if path.trim().is_empty() {
        unsafe { message_box(hwnd, &tr().get("install.err_no_path"), MB_ICONWARNING) };
        return None;
    }
    // Defensive: the Install button is disabled when the folder is non-empty,
    // but a default-button keypress could still reach here. See
    // `should_block_nonempty` for why the guard is skipped on a fixed path.
    if should_block_nonempty(
        super::restriction(),
        super::skip_path(),
        &super::default_path(),
        &path,
    ) {
        unsafe { message_box(hwnd, &tr().get("install.path_not_empty"), MB_ICONWARNING) };
        return None;
    }
    let pb = PathBuf::from(path.trim());
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            st.borrow_mut().chosen_path = Some(pb.clone());
        }
    });
    Some(pb)
}

pub(super) unsafe fn on_install(hwnd: HWND) {
    // Choose page with a plugin wizard pending: validate the path, then enter the
    // wizard instead of installing now.
    if matches!(current_phase(), Phase::Choose) && super::has_plugin_pages() {
        if unsafe { validate_and_store_path(hwnd) }.is_some() {
            unsafe { begin_plugin_wizard(hwnd) };
        }
        return;
    }
    unsafe { commit_install(hwnd) };
}

/// Validate the path and start the install worker, carrying the wizard's answers.
unsafe fn commit_install(hwnd: HWND) {
    let Some(pb) = (unsafe { validate_and_store_path(hwnd) }) else {
        return;
    };

    unsafe { apply_phase(hwnd, Phase::Progress) };

    let shared = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| (st.borrow().cancel.clone(), st.borrow().progress.clone()))
    });
    let Some((cancel, progress_shared)) = shared else {
        return; // STATE not initialized - nothing to do.
    };
    // The wizard's collected answers, routed per plugin (empty with no wizard).
    let plugin_inputs =
        WIZARD.with(|w| w.borrow().as_ref().map(|z| z.inputs()).unwrap_or_default());
    let hwnd_isize = hwnd.0 as isize;
    let translator = tr();

    thread::spawn(move || {
        let mut loaded = match crate::payload::load_and_verify() {
            Ok(l) => l,
            Err(e) => {
                #[cfg(feature = "hintway")]
                crate::analytics::error(crate::analytics::classify_error(&e));
                push_error(hwnd_isize, &format!("{e}"));
                return;
            }
        };
        // Machine-wide iff the target is a shared location (e.g. Program Files);
        // catches an already-admin run that doesn't trip the PermissionDenied
        // elevation path. A non-machine dir that still needs admin is handled by
        // the elevated worker (which forces requires_admin = true).
        let requires_admin = common::paths::is_machine_location(&pb);
        // Resolve feature packs from the wizard's answers and filter the manifest.
        crate::extract::resolve_and_filter(&mut loaded, &pb, requires_admin, &plugin_inputs);
        let progress_cb: common::ProgressFn = {
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
            plugin_inputs: plugin_inputs.clone(),
            requires_admin,
            hwnd_parent: hwnd_isize,
            translator,
        };
        #[cfg(feature = "hintway")]
        crate::analytics::stage("extract");
        if let Err(e) = install(ctx) {
            // A user-confirmed cancel rolled the install back: close cleanly
            // instead of surfacing it as an installation error.
            if cancel.load(Ordering::Relaxed) {
                common::log::info("install cancelled by user");
                helpers::post(hwnd_isize, WM_APP_CANCELLED);
                return;
            }
            if e.downcast_ref::<crate::extract::PermissionDeniedError>()
                .is_some()
            {
                #[cfg(feature = "hintway")]
                crate::analytics::error("permission_denied");
                push_perm_error(hwnd_isize, pb, plugin_inputs);
            } else {
                #[cfg(feature = "hintway")]
                crate::analytics::error(crate::analytics::classify_error(&e));
                push_error(hwnd_isize, &format!("{e}"));
            }
            return;
        }
        #[cfg(feature = "hintway")]
        crate::analytics::stage("finalize");
        if let Err(e) = install_mod::finalize(
            &pb,
            &loaded.payload,
            &loaded.uninstaller_bytes,
            loaded.zip(),
            &plugin_inputs,
            requires_admin,
        ) {
            #[cfg(feature = "hintway")]
            crate::analytics::error(crate::analytics::classify_error(&e));
            push_error(hwnd_isize, &format!("finalize: {e}"));
            return;
        }
        #[cfg(feature = "hintway")]
        crate::analytics::stage("done");
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
        Phase::License | Phase::Choose | Phase::Plugin => {
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
    let ptr = Box::into_raw(Box::new(msg.to_string())) as isize;
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_isize as *mut _)),
            WM_APP_ERROR,
            WPARAM(0),
            LPARAM(ptr),
        )
    };
}

/// Payload carried in `WM_APP_PERM_ERROR` LPARAM.
pub(super) struct PermErrorPayload {
    pub path: PathBuf,
    pub plugin_inputs: common::plugin::InputsByPlugin,
}

fn push_perm_error(
    hwnd_isize: isize,
    path: PathBuf,
    plugin_inputs: common::plugin::InputsByPlugin,
) {
    let ptr = Box::into_raw(Box::new(PermErrorPayload {
        path,
        plugin_inputs,
    })) as isize;
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_isize as *mut _)),
            WM_APP_PERM_ERROR,
            WPARAM(0),
            LPARAM(ptr),
        )
    };
}

/// Spawn the orchestration thread that creates a pipe, triggers UAC, and
/// relays install progress from the elevated worker to the UI.
pub(super) fn start_elevated_install(
    hwnd_isize: isize,
    payload: PermErrorPayload,
    progress_shared: Arc<Mutex<super::ProgressState>>,
) {
    thread::spawn(move || {
        let result = crate::elevation::run_elevated_install(
            &payload.path,
            &payload.plugin_inputs,
            |done, total, name| {
                if let Ok(mut g) = progress_shared.lock() {
                    g.done = done;
                    g.total = total;
                    g.name = name.to_string();
                }
                helpers::post(hwnd_isize, WM_APP_PROGRESS);
            },
        );
        match result {
            Ok(()) => {
                #[cfg(feature = "hintway")]
                crate::analytics::stage("done");
                helpers::post(hwnd_isize, WM_APP_DONE);
            }
            Err(e) if e.is::<crate::elevation::UacCancelledError>() => {
                #[cfg(feature = "hintway")]
                crate::analytics::error("elevation_cancelled");
                let ptr = Box::into_raw(Box::new(payload.path)) as isize;
                let _ = unsafe {
                    PostMessageW(
                        Some(HWND(hwnd_isize as *mut _)),
                        WM_APP_PERM_DENIED,
                        WPARAM(0),
                        LPARAM(ptr),
                    )
                };
            }
            Err(e) => {
                #[cfg(feature = "hintway")]
                crate::analytics::error(crate::analytics::classify_error(&e));
                push_error(hwnd_isize, &format!("{e:#}"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{dir_has_entries, same_dir, should_block_nonempty};
    use common::model::install_dir_restriction::InstallDirRestriction::{
        Bypass, DefaultDirOnly, Enforce,
    };
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
    fn dir_has_entries_false_when_only_empty_subdirs() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        assert!(!dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn dir_has_entries_true_when_file_nested_in_subdir() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), b"x").unwrap();
        assert!(dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn fresh_install_blocks_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        // skip_path = false: the user picked this folder, it must be empty.
        assert!(should_block_nonempty(Enforce, false, "", &path));
    }

    #[test]
    fn update_does_not_block_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("app.exe"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        assert!(!should_block_nonempty(Enforce, true, "", &path));
    }

    #[test]
    fn fresh_install_allows_empty_folder() {
        let dir = tempdir().unwrap();
        assert!(!should_block_nonempty(
            Enforce,
            false,
            "",
            &dir.path().to_string_lossy()
        ));
    }

    #[test]
    fn bypass_allows_any_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("legacy.dll"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        assert!(!should_block_nonempty(Bypass, false, "C:\\Other", &path));
    }

    #[test]
    fn default_dir_only_allows_default_blocks_others() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("legacy.dll"), b"x").unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        // Same folder as the proposed default → allowed.
        assert!(!should_block_nonempty(DefaultDirOnly, false, &path, &path));
        // A different non-empty folder → still blocked.
        assert!(should_block_nonempty(
            DefaultDirOnly,
            false,
            "C:\\Some\\Other\\Dir",
            &path
        ));
    }

    #[test]
    fn same_dir_normalizes_slashes_case_and_trailing_sep() {
        assert!(same_dir("C:/Program Files/App", "c:\\program files\\app\\"));
        assert!(!same_dir("C:\\App", "C:\\Other"));
        assert!(!same_dir("", "C:\\App"));
    }
}
