// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! WinUI confirmation and progress frontend for both uninstall stages.

use super::progress::{self, ProgressStore};
use super::{UninstallParams, Worker, tr};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use windows_reactor::*;
use winui_support::Progress;
use winui_support::widgets::card;
use winui_support::window::WindowHandle;

const WIN_W: f64 = 600.0;
const WIN_H: f64 = 360.0;
const PAD: f64 = 24.0;
const SUBCLASS_ID: usize = 0x1_5A12;

#[derive(Clone)]
struct Config {
    title: String,
    subtitle: String,
    confirm_text: String,
    auto_start: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Confirm,
    Progress,
    Error,
}

enum Message {
    Confirm,
    Cancel,
    Poll,
    Finished(anyhow::Result<()>),
    Close,
}

thread_local! {
    static CONFIG: RefCell<Option<Config>> = const { RefCell::new(None) };
    static PENDING_WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

static MAIN_HWND: WindowHandle = WindowHandle::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
static CONFIRMED: AtomicBool = AtomicBool::new(false);

pub(super) fn run(params: UninstallParams) -> anyhow::Result<bool> {
    CONFIRMED.store(false, Ordering::Relaxed);
    RUNNING.store(false, Ordering::Relaxed);
    MAIN_HWND.reset();
    CONFIG.with(|slot| {
        *slot.borrow_mut() = Some(Config {
            title: params.title,
            subtitle: params.subtitle,
            confirm_text: params.confirm_text,
            auto_start: params.auto_start,
        });
    });
    PENDING_WORKER.with(|slot| *slot.borrow_mut() = Some(params.worker));

    let result = winui_support::launch_app::<Uninstaller>(());

    CONFIG.with(|slot| *slot.borrow_mut() = None);
    PENDING_WORKER.with(|slot| *slot.borrow_mut() = None);
    anyhow::ensure!(
        result?,
        "WinUI runtime unavailable after frontend selection"
    );
    Ok(CONFIRMED.load(Ordering::Relaxed))
}

fn config() -> Config {
    CONFIG.with(|slot| slot.borrow().as_ref().cloned().unwrap())
}

struct Uninstaller {
    config: Config,
    phase: Phase,
    progress: Progress,
    progress_store: ProgressStore,
    error: String,
    attached: bool,
}

impl Uninstaller {
    fn poll(context: &ComponentContext<Self>) {
        _ = context.spawn_background(|_| {
            std::thread::sleep(Duration::from_millis(50));
            Message::Poll
        });
    }

    fn start(&mut self, context: &ComponentContext<Self>) {
        let Some(worker) = PENDING_WORKER.with(|slot| slot.borrow_mut().take()) else {
            return;
        };
        CONFIRMED.store(true, Ordering::Relaxed);
        RUNNING.store(true, Ordering::Relaxed);
        self.phase = Phase::Progress;
        let callback = progress::callback(&self.progress_store);
        _ = context.spawn_background(move |_| Message::Finished(worker.run(callback)));
    }
}

impl Component for Uninstaller {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let config = config();
        let auto_start = config.auto_start;
        let mut this = Self {
            phase: if auto_start {
                Phase::Progress
            } else {
                Phase::Confirm
            },
            config,
            progress: Progress::default(),
            progress_store: ProgressStore::default(),
            error: String::new(),
            attached: false,
        };
        if auto_start {
            this.start(context);
        }
        Self::poll(context);
        this
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        if !self.attached && MAIN_HWND.active() != 0 {
            attach_window();
            self.attached = true;
        }
        match message {
            Message::Confirm => self.start(context),
            Message::Cancel => {
                RUNNING.store(false, Ordering::Relaxed);
                _ = context.window().request_close();
            }
            Message::Poll => {
                self.progress = self.progress_store.get();
                Self::poll(context);
            }
            Message::Finished(Ok(())) => {
                self.progress = self.progress_store.get();
                RUNNING.store(false, Ordering::Relaxed);
                _ = context.spawn_background(|_| {
                    std::thread::sleep(Duration::from_millis(150));
                    Message::Close
                });
            }
            Message::Finished(Err(error)) => {
                RUNNING.store(false, Ordering::Relaxed);
                self.error = format!("{error:#}");
                self.phase = Phase::Error;
            }
            Message::Close => {
                _ = context.window().request_close();
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title(self.config.title.clone());
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(WIN_W, WIN_H),
        );
        Grid::new()
            .rows([
                GridLength::Auto,
                GridLength::Auto,
                GridLength::STAR,
                GridLength::Auto,
            ])
            .children((
                TitleBar::new().title(self.config.title.clone()).grid_row(0),
                Border::new().grid_row(1).content(header(&self.config)),
                Border::new().grid_row(2).content(content(
                    self.phase,
                    &self.config,
                    &self.progress,
                    &self.error,
                )),
                Border::new()
                    .grid_row(3)
                    .content(buttons(self.phase, context)),
            ))
    }
}

fn header(config: &Config) -> View {
    Border::new()
        .padding(Thickness::new(PAD, 12.0, PAD, 4.0))
        .content(
            StackPanel::new().spacing(4.0).children((
                TextBlock::new()
                    .text(config.title.clone())
                    .font_size(28.0)
                    .font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new()
                    .text(config.subtitle.clone())
                    .font_size(14.0),
            )),
        )
}

fn content(phase: Phase, config: &Config, progress: &Progress, error: &str) -> View {
    let body = match phase {
        Phase::Confirm => View::from(card(
            ScrollViewer::new()
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .content(
                    Border::new().padding(16.0).content(
                        TextBlock::new()
                            .text(config.confirm_text.clone())
                            .text_wrapping(TextWrapping::Wrap)
                            .is_text_selection_enabled(true),
                    ),
                ),
        )),
        Phase::Progress => progress_view(progress),
        Phase::Error => error_view(error),
    };
    Border::new()
        .margin(Thickness::new(PAD, 12.0, PAD, 12.0))
        .content(body)
}

fn progress_view(progress: &Progress) -> View {
    let fraction = if progress.total > 0 {
        (progress.done as f64 / progress.total as f64).clamp(0.0, 1.0) * 100.0
    } else {
        0.0
    };
    let status = if progress.total > 0 {
        format!("{}%   {}", fraction as u32, progress.name)
    } else {
        progress.name.clone()
    };
    Border::new()
        .vertical_alignment(VerticalAlignment::Top)
        .content(card(
            Border::new().padding(16.0).content(
                StackPanel::new().spacing(16.0).children((
                    ProgressBar::new()
                        .minimum(0.0)
                        .maximum(100.0)
                        .value(fraction)
                        .horizontal_alignment(HorizontalAlignment::Stretch),
                    TextBlock::new()
                        .text(status)
                        .text_wrapping(TextWrapping::Wrap)
                        .height(48.0),
                )),
            ),
        ))
}

fn error_view(error: &str) -> View {
    View::from(card(
        Border::new().padding(16.0).content(
            StackPanel::new().spacing(10.0).children((
                TextBlock::new()
                    .text(tr().get("uninstall.fatal_caption"))
                    .font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new()
                    .text(error)
                    .text_wrapping(TextWrapping::Wrap)
                    .is_text_selection_enabled(true),
            )),
        ),
    ))
}

fn buttons(phase: Phase, context: &ViewContext<Uninstaller>) -> View {
    let children = match phase {
        Phase::Confirm => View::fragment((
            Button::new()
                .style(ButtonStyle::Accent)
                .width(140.0)
                .on_click(context.callback(|_| Message::Confirm))
                .content(tr().get("uninstall.yes")),
            Button::new()
                .width(110.0)
                .on_click(context.callback(|_| Message::Cancel))
                .content(tr().get("uninstall.no")),
        )),
        Phase::Error => Button::new()
            .style(ButtonStyle::Accent)
            .width(110.0)
            .on_click(context.callback(|_| Message::Cancel))
            .content("OK"),
        Phase::Progress => View::empty(),
    };
    Border::new()
        .height(56.0)
        .padding(Thickness::new(PAD, 0.0, PAD, PAD))
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(10.0)
                .horizontal_alignment(HorizontalAlignment::Right)
                .children((children,)),
        )
}

fn attach_window() {
    let raw = MAIN_HWND.active();
    if raw != 0 {
        unsafe {
            _ = SetWindowSubclass(HWND(raw as *mut _), Some(close_guard), SUBCLASS_ID, 0);
        }
    }
}

unsafe extern "system" fn close_guard(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_CLOSE && RUNNING.load(Ordering::Relaxed) {
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
