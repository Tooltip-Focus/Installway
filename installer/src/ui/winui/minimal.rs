// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Compact auto-update UI built on Reactor's component API.

use super::model::{AsyncValue, Progress, Signal, tr, with_payload};
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
const LINGER: Duration = Duration::from_millis(900);

thread_local! {
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

pub(super) enum Message {
    Poll,
    Close,
}

pub(super) struct Minimal {
    progress: Progress,
    progress_sink: AsyncValue<Progress>,
    signal: Signal,
    signal_sink: AsyncValue<Signal>,
    error: String,
}

impl Minimal {
    fn poll(context: &ComponentContext<Self>, delay: Duration) {
        _ = context.spawn_background(move |_| {
            std::thread::sleep(delay);
            Message::Poll
        });
    }
}

impl Component for Minimal {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let progress_sink = AsyncValue::default();
        let signal_sink = AsyncValue::default();
        if let Some(job) = PENDING.with(|p| p.borrow_mut().take()) {
            spawn(job, progress_sink.clone(), signal_sink.clone());
        }
        Self::poll(context, Duration::from_millis(50));
        Self {
            progress: Progress::default(),
            progress_sink,
            signal: Signal::None,
            signal_sink,
            error: String::new(),
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Close => {
                _ = context.window().request_close();
            }
            Message::Poll => {
                self.progress = self.progress_sink.get();
                let signal = self.signal_sink.take();
                if signal != Signal::None {
                    self.signal = signal;
                    match &self.signal {
                        Signal::Done => {
                            _ = context.spawn_background(|_| {
                                std::thread::sleep(LINGER);
                                Message::Close
                            });
                            return;
                        }
                        Signal::Error(text) => {
                            self.error = text.clone();
                            return;
                        }
                        _ => {}
                    }
                }
                Self::poll(context, Duration::from_millis(50));
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let t = tr();
        let (product, version) = with_payload(|p| (p.product.clone(), p.to_version.clone()));
        let title = t.get("install.minimal_title");
        let sub = t.fmt(
            "install.minimal_sub",
            &[("product", &product), ("version", &version)],
        );
        context.window_title(title.clone());
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(WIN_W, WIN_H),
        );

        let status = if !self.error.is_empty() {
            self.error.clone()
        } else if self.signal == Signal::Done {
            t.get("install.minimal_done")
        } else if self.progress.total > 0 {
            format!(
                "{}%   {}",
                self.progress
                    .done
                    .saturating_mul(100)
                    .checked_div(self.progress.total)
                    .unwrap_or_default(),
                self.progress.name
            )
        } else {
            self.progress.name.clone()
        };
        let fraction = if self.signal == Signal::Done {
            100.0
        } else if self.progress.total > 0 {
            (self.progress.done as f64 / self.progress.total as f64).clamp(0.0, 1.0) * 100.0
        } else {
            0.0
        };

        Grid::new()
            .columns([GridLength::Auto, GridLength::STAR])
            .column_spacing(20.0)
            .margin(PAD)
            .children((
                icon_cell().grid_column(0),
                StackPanel::new()
                    .spacing(6.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .grid_column(1)
                    .children((
                        TextBlock::new()
                            .text(title)
                            .font_size(16.0)
                            .font_weight(FontWeight::SEMI_BOLD),
                        TextBlock::new().text(sub).font_size(12.0),
                        ProgressBar::new()
                            .minimum(0.0)
                            .maximum(100.0)
                            .value(fraction)
                            .horizontal_alignment(HorizontalAlignment::Stretch),
                        TextBlock::new()
                            .text(status)
                            .font_size(12.0)
                            .text_wrapping(TextWrapping::Wrap),
                    )),
            ))
    }
}

fn icon_cell() -> Image {
    let image = Image::new()
        .stretch(Stretch::Uniform)
        .width(ICON_SZ)
        .height(ICON_SZ)
        .vertical_alignment(VerticalAlignment::Center);
    super::staged_icon_uri()
        .and_then(|uri| image.clone().source(uri).ok())
        .unwrap_or(image)
}

fn spawn(job: Job, progress: AsyncValue<Progress>, signal: AsyncValue<Signal>) {
    std::thread::spawn(move || {
        let Job {
            mut loaded,
            install_dir,
            launch,
        } = job;
        let requires_admin = common::paths::is_machine_location(&install_dir);
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
                })
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
