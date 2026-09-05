// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! WinUI 3 needs the Windows App SDK runtime. If not present, fallback to win32.

mod banner;
mod icon;
mod minimal;
mod model;
mod plugin_page;
mod wizard;
mod wizard_state;
mod worker;

use anyhow::Result;
use model::{
    BANNER_URI, DEFAULT_PATH, LAUNCH_FLAG, PAYLOAD, RESTRICTION, SIGNAL_SINK, SKIP_LICENSE,
    SKIP_PATH, Signal, WIZARD,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use winui_support::window::WindowHandle;

thread_local! {
    /// Holds staged temp files (banner, icon) for the whole run.
    static STAGED: RefCell<Vec<Box<dyn std::any::Any>>> = const { RefCell::new(Vec::new()) };
    static ICON_URI: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Published once the window is up, for UAC parenting and the close guard.
static MAIN_HWND: WindowHandle = WindowHandle::new();

static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Two X presses must not compare equal, or the second write is a no-op.
static CLOSE_SEQ: AtomicU64 = AtomicU64::new(0);

const SUBCLASS_ID: usize = 0x1_5A11;

pub use winui_support::available;
use winui_support::launch_app;

/// `Ok(false)` means no Windows App SDK runtime; run the Win32 wizard instead.
pub fn run(
    loaded: crate::payload::LoadedPayload,
    default_path: PathBuf,
    launch_flag: bool,
    already_installed: bool,
    translator: common::i18n::Translator,
    ui_plugins: Option<crate::extract::UiPlugins>,
) -> Result<bool> {
    // An existing install fixes the target folder: a patch must land there and a
    // reinstall must not make a second copy. So Choose is always skipped then.
    let skip_license = loaded.payload.skip_license;
    let skip_path = loaded.payload.skip_path || already_installed;

    let title = translator.fmt(
        "install.window_title",
        &[
            ("product", &loaded.payload.product),
            ("version", &loaded.payload.to_version),
        ],
    );

    seed_config(
        &loaded,
        &default_path,
        launch_flag,
        skip_license,
        skip_path,
        translator,
    );

    // Shared with every background query, so a detached one can still read it.
    let _ui_guard = ui_plugins.map(|u| {
        let tmp = Arc::new(u.tmp);
        WIZARD.with(|w| {
            *w.borrow_mut() = Some(wizard_state::Wizard::new(
                u.plugins,
                u.base_ctx,
                u.self_exe,
                tmp.clone(),
            ));
        });
        tmp
    });

    let _ = title;
    launch_app::<wizard::WizardApp>(())
}

/// Run the compact auto-update UI. `Ok(false)` = fall back to Win32.
pub fn run_minimal(
    loaded: crate::payload::LoadedPayload,
    install_dir: PathBuf,
    launch: bool,
    translator: common::i18n::Translator,
) -> Result<bool> {
    let title = translator.get("install.minimal_title");
    seed_config(&loaded, &install_dir, launch, true, true, translator);
    stage_product_icon();
    minimal::set_job(loaded, install_dir, launch);
    let _ = title;
    launch_app::<minimal::Minimal>(())
}

/// Reactor runs on the calling thread, so these thread-locals are the ones the
/// render function reads.
fn seed_config(
    loaded: &crate::payload::LoadedPayload,
    default_path: &std::path::Path,
    launch_flag: bool,
    skip_license: bool,
    skip_path: bool,
    translator: common::i18n::Translator,
) {
    PAYLOAD.with(|p| *p.borrow_mut() = Some(loaded.payload.clone()));
    LAUNCH_FLAG.with(|l| *l.borrow_mut() = launch_flag);
    SKIP_LICENSE.with(|s| *s.borrow_mut() = skip_license);
    SKIP_PATH.with(|s| *s.borrow_mut() = skip_path);
    RESTRICTION.with(|r| *r.borrow_mut() = loaded.payload.install_dir_restriction);
    DEFAULT_PATH.with(|d| *d.borrow_mut() = default_path.to_string_lossy().into_owned());
    model::set_translator(translator);
    stage_banner(loaded.banner_png.as_deref());
}

fn stage_banner(png: Option<&[u8]>) {
    let Some(png) = png.filter(|b| !b.is_empty()) else {
        return;
    };
    if let Some(staged) = banner::StagedBanner::write(png) {
        BANNER_URI.with(|b| *b.borrow_mut() = Some(staged.uri()));
        STAGED.with(|s| s.borrow_mut().push(Box::new(staged)));
    }
}

fn stage_product_icon() {
    if let Some(staged) = icon::stage_own_icon() {
        ICON_URI.with(|i| *i.borrow_mut() = Some(staged.uri()));
        STAGED.with(|s| s.borrow_mut().push(Box::new(staged)));
    }
}

pub(super) fn staged_icon_uri() -> Option<String> {
    ICON_URI.with(|i| i.borrow().clone())
}

/// Called from the first render, by which point the window exists.
pub(in crate::ui::winui) fn attach_window(sink: model::AsyncValue<Signal>) {
    if let Ok(mut slot) = SIGNAL_SINK.lock() {
        *slot = Some(sink);
    }
    let raw = active_hwnd();
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = SetWindowSubclass(HWND(raw as *mut _), Some(close_guard), SUBCLASS_ID, 0);
    }
}

/// Swallow `WM_CLOSE` while installing so the X cannot kill a half-applied
/// install; the wizard confirms, then cancels cooperatively.
unsafe extern "system" fn close_guard(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_CLOSE && INSTALL_RUNNING.load(Ordering::Relaxed) {
        let seq = CLOSE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
        model::raise(Signal::CloseRequested(seq));
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

pub(super) fn set_install_running(on: bool) {
    INSTALL_RUNNING.store(on, Ordering::Relaxed);
}

/// Parent for the UAC prompt and the folder picker; `0` before the window exists.
pub(super) fn active_hwnd() -> isize {
    MAIN_HWND.active()
}

/// Clears the guard first, so a deliberate close is never swallowed.
pub(super) fn close_window() {
    set_install_running(false);
    MAIN_HWND.close();
}

pub(super) fn pick_folder() -> Option<String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SIGDN_FILESYSPATH,
    };

    // Reactor already put this thread in an STA, so no CoInitialize pair here.
    let parent = active_hwnd();
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        let owner = (parent != 0).then_some(HWND(parent as *mut _));
        if dialog.Show(owner).is_err() {
            return None;
        }
        let item: IShellItem = dialog.GetResult().ok()?;
        let pwstr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0 as *const _));
        s
    }
}

/// Dev-only: render one view with sample data, no payload needed.
#[cfg(debug_assertions)]
pub fn preview(view: &str, translator: common::i18n::Translator) -> Result<bool> {
    let payload = crate::ui::sample_payload(view);
    let title = translator.fmt(
        "install.window_title",
        &[
            ("product", &payload.product),
            ("version", &payload.to_version),
        ],
    );
    PAYLOAD.with(|p| *p.borrow_mut() = Some(payload));
    LAUNCH_FLAG.with(|l| *l.borrow_mut() = true);
    DEFAULT_PATH.with(|d| *d.borrow_mut() = r"C:\Program Files\Sample App".to_string());
    model::set_translator(translator);

    // Iterate on a banner without packing a full installer.
    if let Some(path) = std::env::var_os("INSTALLWAY_PREVIEW_BANNER") {
        match std::fs::read(&path) {
            Ok(bytes) => stage_banner(Some(&bytes)),
            Err(e) => eprintln!(
                "preview banner {}: {e}",
                std::path::Path::new(&path).display()
            ),
        }
    }

    if view == "minimal" {
        stage_product_icon();
        let _ = title;
        return launch_app::<minimal::Minimal>(());
    }

    // A canned page exercises the dynamic renderer with no real plugin.
    if view.starts_with("plugin") {
        WIZARD.with(|w| *w.borrow_mut() = Some(wizard_state::Wizard::canned(vec![sample_step()])));
    }

    wizard::set_preview_phase(view);
    let _ = title;
    launch_app::<wizard::WizardApp>(())
}

#[cfg(debug_assertions)]
fn sample_step() -> common::model::page_step::PageStep {
    use common::model::choice_option::ChoiceOption;
    use common::model::choice_style::ChoiceStyle;
    use common::model::page_step::PageStep;
    use common::model::plugin_page::PluginPage;
    use common::model::plugin_widget::PluginWidget;

    PageStep::Page {
        page: PluginPage {
            id: "region".into(),
            title: "Choose your country".into(),
            subtitle: "Sample plugin page (preview)".into(),
            widgets: vec![
                PluginWidget::Label {
                    id: String::new(),
                    text: "Where will you use this app?".into(),
                },
                PluginWidget::SingleChoice {
                    id: "country".into(),
                    label: "Country".into(),
                    options: vec![
                        ChoiceOption {
                            label: "France".into(),
                            value: "FR".into(),
                        },
                        ChoiceOption {
                            label: "DOM-TOM".into(),
                            value: "DOM".into(),
                        },
                        ChoiceOption {
                            label: "Other".into(),
                            value: "XX".into(),
                        },
                    ],
                    style: ChoiceStyle::Radio,
                    default: "FR".into(),
                    required: true,
                },
                PluginWidget::Text {
                    id: "license".into(),
                    label: "License key".into(),
                    default: String::new(),
                    required: false,
                    placeholder: "optional".into(),
                    password: true,
                    number: false,
                    multiline: false,
                },
                PluginWidget::MultiChoice {
                    id: "addons".into(),
                    label: "Optional add-ons".into(),
                    options: vec![
                        ChoiceOption {
                            label: "Documentation".into(),
                            value: "docs".into(),
                        },
                        ChoiceOption {
                            label: "Samples".into(),
                            value: "samples".into(),
                        },
                    ],
                    default: vec!["docs".into()],
                    required: false,
                },
            ],
            buttons: true,
        },
        notice: String::new(),
        back: true,
    }
}
