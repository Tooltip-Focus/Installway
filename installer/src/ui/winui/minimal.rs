// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Compact auto-update UI, the WinUI counterpart of [`crate::ui::minimal`].
//! Icon left, title and progress right, no buttons, closes itself after 100%.

use super::model::{Progress, Signal, tr, with_payload};
use crate::extract::{InstallCtx, install};
use crate::payload::LoadedPayload;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use windows_reactor::*;

pub(super) const WIN_W: f64 = 480.0;
pub(super) const WIN_H: f64 = 140.0;
const PAD: f64 = 20.0;
const ICON_SZ: f64 = 48.0;

/// How long 100% stays up before the window closes.
const LINGER: Duration = Duration::from_millis(900);

thread_local! {
    /// Taken by the first render.
    static PENDING: std::cell::RefCell<Option<Job>> = const { std::cell::RefCell::new(None) };
}

struct Job {
    loaded: LoadedPayload,
    install_dir: PathBuf,
    launch: bool,
}

pub(super) fn set_job(loaded: LoadedPayload, install_dir: PathBuf, launch: bool) {
    PENDING.with(|p| {
        *p.borrow_mut() = Some(Job {
            loaded,
            install_dir,
            launch,
        })
    });
}

pub(super) fn app(cx: &mut RenderCx) -> Element {
    let (progress, set_progress) = cx.use_async_state(Progress::default());
    let (signal, set_signal) = cx.use_async_state(Signal::None);
    let (error, set_error) = cx.use_state(String::new());

    // Start the update on first render.
    {
        let set_progress = set_progress.clone();
        let set_signal = set_signal.clone();
        cx.use_effect((), move || {
            if let Some(job) = PENDING.with(|p| p.borrow_mut().take()) {
                spawn(job, set_progress, set_signal);
            }
        });
    }

    // Show 100% briefly, then close; or surface the error and stay up.
    {
        let signal = signal.clone();
        let set_error = set_error.clone();
        cx.use_effect_with_cleanup(signal.clone(), move || match signal {
            Signal::Done => {
                let timer = DispatcherTimer::new_one_shot(LINGER, super::close_window).ok();
                Some(move || drop(timer))
            }
            Signal::Error(text) => {
                set_error.call(text);
                None
            }
            _ => None,
        });
    }

    let (title, sub) = {
        let t = tr();
        let (product, version) = with_payload(|p| (p.product.clone(), p.to_version.clone()));
        let sub = t.fmt(
            "install.minimal_sub",
            &[("product", &product), ("version", &version)],
        );
        (t.get("install.minimal_title"), sub)
    };

    let status = if !error.is_empty() {
        error
    } else if signal == Signal::Done {
        tr().get("install.minimal_done")
    } else if progress.total > 0 {
        let pct = (progress.done as f64 / progress.total as f64 * 100.0) as u32;
        format!("{pct}%   {}", progress.name)
    } else {
        progress.name.clone()
    };
    let fraction = if signal == Signal::Done {
        100.0
    } else if progress.total > 0 {
        (progress.done as f64 / progress.total as f64).clamp(0.0, 1.0) * 100.0
    } else {
        0.0
    };

    grid(vec![
        icon_cell().grid_column(0),
        Element::from(
            vstack((
                text_block(title).font_size(16.0).semibold(),
                text_block(sub)
                    .font_size(12.0)
                    .foreground(ThemeRef::SecondaryText),
                ProgressBar::new(fraction)
                    .range(0.0, 100.0)
                    .horizontal_alignment(HorizontalAlignment::Stretch),
                text_block(status).font_size(12.0).wrap(),
            ))
            .spacing(6.0)
            .vertical_alignment(VerticalAlignment::Center),
        )
        .grid_column(1),
    ])
    .columns([GridLength::Auto, GridLength::STAR])
    .column_spacing(20.0)
    .padding(Thickness::uniform(PAD))
    .into()
}

fn icon_cell() -> Element {
    match super::staged_icon_uri() {
        Some(uri) => Image::new_with_uri(uri)
            .stretch(Stretch::Uniform)
            .width(ICON_SZ)
            .height(ICON_SZ)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
        None => vstack(()).width(ICON_SZ).into(),
    }
}

fn spawn(job: Job, progress: AsyncSetState<Progress>, signal: AsyncSetState<Signal>) {
    std::thread::spawn(move || {
        let Job {
            mut loaded,
            install_dir,
            launch,
        } = job;
        let requires_admin = common::paths::is_machine_location(&install_dir);
        // No interactive UI, so plugin pages use their declared defaults.
        let plugin_inputs = match crate::ui::headless_plugin_inputs(&loaded, &install_dir) {
            Ok(i) => i,
            Err(e) => {
                signal.call(Signal::Error(format!("{e:#}")));
                return;
            }
        };
        crate::extract::resolve_and_filter(
            &mut loaded,
            &install_dir,
            requires_admin,
            &plugin_inputs,
        );

        let on_progress: common::ProgressFn = {
            let progress = progress.clone();
            Arc::new(move |done, total, name: &str| {
                progress.call(Progress {
                    done,
                    total,
                    name: name.to_string(),
                });
            })
        };
        let ctx = InstallCtx {
            install_dir: install_dir.clone(),
            payload: &loaded.payload,
            zip_bytes: loaded.zip(),
            cancel: Arc::new(AtomicBool::new(false)),
            on_progress,
            plugin_inputs: plugin_inputs.clone(),
            requires_admin,
            hwnd_parent: super::active_hwnd(),
            translator: tr(),
        };
        // Lock held across finalize so a concurrent run can't interleave.
        let _install_lock = match install(ctx) {
            Ok(lock) => lock,
            Err(e) => {
                signal.call(Signal::Error(format!("{e:#}")));
                return;
            }
        };
        if let Err(e) = crate::install::finalize(
            &install_dir,
            &loaded.payload,
            &loaded.uninstaller_bytes,
            loaded.zip(),
            &plugin_inputs,
            requires_admin,
        ) {
            signal.call(Signal::Error(format!("finalize: {e:#}")));
            return;
        }
        if launch && let Some(exe) = loaded.payload.manifest.exe.as_deref() {
            let _ = crate::install::launch_product(&install_dir, Some(exe));
        }
        signal.call(Signal::Done);
    });
}
