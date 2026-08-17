// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud
//! The full installer wizard: License, Choose, Progress, Done/Error, with
//! plugin-contributed pages spliced in before the install.
//!
//! One render function of one [`Model`]. Sizes are DIPs, so the Win32 wizard's
//! 96-dpi numbers carry over unchanged and XAML handles per-monitor scaling.

use super::model::{
    Dialog, Model, PERM_ERROR, Phase, Progress, QUERIED_STEP, Signal, WIZARD, default_path,
    has_plugin_pages, launch_option, restriction, skip_license, skip_path, tr, with_payload,
};
use super::plugin_page;
use super::wizard_state::Step;
use super::worker::{self, Feed};
use common::model::launch_option::LaunchOption;
use common::model::payload_kind::PayloadKind;
use std::path::{Path, PathBuf};
use windows_reactor::*;

pub(super) const WIN_W: f64 = 700.0;
pub(super) const WIN_H: f64 = 540.0;
const BANNER_H: f64 = 72.0;
const PAD: f64 = 24.0;
const CARD_RADIUS: f64 = 8.0;

/// Placeholder EULA, matching the Win32 wizard's so both previews agree.
const LOREM: &str = "END USER LICENSE AGREEMENT - SAMPLE\r\n\r\n\
Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
consequat.\r\n\r\n\
Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore \
eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, \
sunt in culpa qui officia deserunt mollit anim id est laborum.\r\n\r\n\
Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium \
doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore \
veritatis et quasi architecto beatae vitae dicta sunt explicabo.\r\n\r\n\
Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit, \
sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt.\r\n\r\n\
At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis \
praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias \
excepturi sint occaecati cupiditate non provident, similique sunt in culpa \
qui officia deserunt mollitia animi, id est laborum et dolorum fuga.\r\n\r\n\
By clicking 'I accept' you agree to be bound by the terms above.";

/// A unit handler that clones the model, mutates it, and commits.
fn on(
    model: &Model,
    set: &SetState<Model>,
    f: impl Fn(&mut Model) + 'static,
) -> impl Fn() + 'static {
    let model = model.clone();
    let set = set.clone();
    move || {
        let mut next = model.clone();
        f(&mut next);
        set.call(next);
    }
}

pub(super) fn app(cx: &mut RenderCx) -> Element {
    let (model, set) = cx.use_state(Model::new(Path::new(&default_path())));
    let (signal, set_signal) = cx.use_async_state(Signal::None);
    let (progress, set_progress) = cx.use_async_state(Progress::default());
    let feed = Feed {
        signal: set_signal.clone(),
        progress: set_progress.clone(),
    };
    // Two successive completions must never compare equal, or the second write
    // is a no-op.
    let seq = cx.use_ref(0_u64);

    // The window exists by the time effects run, so the close guard goes in here.
    {
        let (model, set, feed) = (model.clone(), set.clone(), feed.clone());
        let seq = seq.clone();
        let sink = set_signal.clone();
        cx.use_effect((), move || {
            super::attach_window(sink);
            let mut next = model.clone();
            start_flow(&mut next, &feed, &seq);
            set.call(next);
        });
    }

    // Apply whatever a background thread last reported.
    {
        let (model, set, feed) = (model.clone(), set.clone(), feed.clone());
        let seq = seq.clone();
        let signal = signal.clone();
        cx.use_effect(signal.clone(), move || {
            if signal == Signal::None {
                return;
            }
            let mut next = model.clone();
            if apply_signal(&mut next, &signal, &feed, &seq) {
                set.call(next);
            }
        });
    }

    // Progress has its own hook so a 60-per-second stream never round-trips
    // through the whole model.
    let mut model = model;
    model.progress = progress;

    // Nothing here paints an opaque background; Mica is the background.
    let title_bar = TitleBar::new(with_payload(|p| p.product.clone()));
    let children: Vec<Element> = vec![
        Element::from(title_bar).grid_row(0),
        banner(&model).grid_row(1),
        content(&model, &set).grid_row(2),
        buttons(&model, &set, &feed, &seq).grid_row(3),
        dialog(&model, &set),
    ];

    grid(children)
        .rows([
            GridLength::Auto,
            GridLength::Auto,
            GridLength::STAR,
            GridLength::Auto,
        ])
        .into()
}

// ---- Header --------------------------------------------------------------

/// A packaged banner image with the title overlaid, or a large title on Mica.
fn banner(model: &Model) -> Element {
    let (header, sub) = banner_text(model);

    match super::model::BANNER_URI.with(|b| b.borrow().clone()) {
        // Two children in one grid cell overlap in declaration order. Banner art
        // is authored light, so the ink stays dark whatever the theme.
        Some(uri) => {
            let ink = Color::rgb(0x33, 0x33, 0x33);
            let overlay = vstack((
                text_block(header)
                    .font_size(20.0)
                    .semibold()
                    .foreground(ink),
                text_block(sub).font_size(12.0).foreground(ink),
            ))
            .spacing(2.0)
            .vertical_alignment(VerticalAlignment::Center)
            .padding(Thickness {
                left: PAD,
                top: 0.0,
                right: PAD,
                bottom: 0.0,
            });
            grid(vec![
                Element::from(Image::new_with_uri(uri).stretch(Stretch::UniformToFill)),
                Element::from(overlay),
            ])
            .height(BANNER_H)
            .into()
        }
        None => vstack((
            text_block(header).font_size(28.0).semibold(),
            text_block(sub)
                .font_size(14.0)
                .foreground(ThemeRef::SecondaryText),
        ))
        .spacing(4.0)
        .padding(Thickness {
            left: PAD,
            top: 12.0,
            right: PAD,
            bottom: 4.0,
        })
        .into(),
    }
}

/// Plugin pages borrow the header for their own title; every other phase shows
/// the product one, the Error page announcing the failure in its InfoBar.
fn banner_text(model: &Model) -> (String, String) {
    let t = tr();
    if model.phase == Phase::Plugin {
        let title = WIZARD.with(|w| w.borrow().as_ref().map(|z| z.current_title()));
        if let Some((title, sub)) = title
            && !title.is_empty()
        {
            return (title, sub);
        }
    }
    let (product, version, kind, from) = with_payload(|p| {
        (
            p.product.clone(),
            p.to_version.clone(),
            p.kind,
            p.from_version.clone().unwrap_or_default(),
        )
    });
    let header = t.fmt(
        "install.header",
        &[("product", &product), ("version", &version)],
    );
    let sub = match kind {
        PayloadKind::Full => t.get("install.sub_full"),
        PayloadKind::Patch => t.fmt("install.sub_patch", &[("from", &from), ("to", &version)]),
    };
    (header, sub)
}

// ---- Content -------------------------------------------------------------

fn content(model: &Model, set: &SetState<Model>) -> Element {
    let body: Element = match model.phase {
        Phase::License => license_view(model, set),
        Phase::Choose => choose_view(model, set).into(),
        Phase::Plugin => plugin_view(model, set),
        Phase::Progress | Phase::Done => progress_view(model, set).into(),
        Phase::Error => error_view(model),
    };
    // Margin, not padding: on a `Border` padding sits inside the stroke, which
    // would stretch the card edge to edge.
    body.margin(Thickness {
        left: PAD,
        top: 12.0,
        right: PAD,
        bottom: 12.0,
    })
}

fn card(child: impl Into<Element>) -> Border {
    border(child)
        .corner_radius(CARD_RADIUS)
        .background(ThemeRef::CardBackground)
        .border_thickness(Thickness::uniform(1.0))
        .border_brush(ThemeRef::CardStroke)
}

/// A grid, not a stack: the EULA must absorb the leftover height, or the scroll
/// viewer sizes to its content and pushes the checkbox off-window.
fn license_view(model: &Model, set: &SetState<Model>) -> Element {
    let text = with_payload(|p| p.license_text.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| LOREM.to_string());
    let accepted = model.license_accepted;
    grid(vec![
        Element::from(card(
            scroll_viewer(
                text_block(text)
                    .wrap()
                    .selectable()
                    .padding(Thickness::uniform(16.0)),
            )
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto),
        ))
        .grid_row(0),
        Element::from(
            check_box(accepted)
                .content(tr().get("install.license_accept"))
                .on_checked(on_bool(model, set, |m, v| m.license_accepted = v))
                .margin(Thickness {
                    left: 4.0,
                    top: 12.0,
                    right: 0.0,
                    bottom: 0.0,
                }),
        )
        .grid_row(1),
    ])
    .rows([GridLength::STAR, GridLength::Auto])
    .into()
}

fn choose_view(model: &Model, set: &SetState<Model>) -> Border {
    let picker = {
        let model = model.clone();
        let set = set.clone();
        move || {
            if let Some(picked) = super::pick_folder() {
                let mut next = model.clone();
                let product = with_payload(|p| p.product.clone());
                next.path = crate::ui::dest::with_product_subdir(&picked, &product);
                set.call(next);
            }
        }
    };
    let mut rows: Vec<Element> = vec![
        text_block(tr().get("install.choose_label")).into(),
        grid(vec![
            Element::from(text_box(model.path.clone()).on_text_changed(on_string(
                model,
                set,
                |m, v| m.path = v,
            )))
            .grid_column(0),
            Element::from(
                button(tr().get("install.browse"))
                    .on_click(picker)
                    .width(110.0)
                    .margin(Thickness {
                        left: 10.0,
                        top: 0.0,
                        right: 0.0,
                        bottom: 0.0,
                    }),
            )
            .grid_column(1),
        ])
        .columns([GridLength::STAR, GridLength::Auto])
        .into(),
    ];
    if blocks_install(&model.path) {
        rows.push(
            InfoBar::new(tr().get("install.path_not_empty"))
                .warning()
                .is_open(true)
                .is_closable(false)
                .into(),
        );
    }
    card(vstack(rows).spacing(12.0).padding(Thickness::uniform(16.0)))
        .vertical_alignment(VerticalAlignment::Top)
}

/// Scrollable, because a plugin can declare more widgets than the window is
/// tall; the Win32 backend can only clip them.
fn plugin_view(model: &Model, set: &SetState<Model>) -> Element {
    let rows = WIZARD.with(|w| {
        w.borrow()
            .as_ref()
            .and_then(|z| z.current_page().map(|p| plugin_page::render(p, model, set)))
            .unwrap_or_default()
    });
    card(
        scroll_viewer(
            vstack(rows)
                .spacing(plugin_page::row_gap())
                .padding(Thickness::uniform(16.0)),
        )
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto),
    )
    .into()
}

/// Progress and Done share the bar and status line; Done adds "run now".
fn progress_view(model: &Model, set: &SetState<Model>) -> Border {
    let p = &model.progress;
    let done = model.phase == Phase::Done;
    let fraction = if p.total > 0 {
        (p.done as f64 / p.total as f64).clamp(0.0, 1.0) * 100.0
    } else if done {
        100.0
    } else {
        0.0
    };
    let status = if done {
        tr().get("install.done")
    } else if model.cancelling {
        tr().get("install.cancelling")
    } else if p.total > 0 {
        format!(
            "{}%   ({} / {} bytes)\n{}",
            fraction as u32, p.done, p.total, p.name
        )
    } else {
        p.name.clone()
    };

    let mut rows: Vec<Element> = vec![
        ProgressBar::new(fraction)
            .range(0.0, 100.0)
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .into(),
        text_block(status)
            .wrap()
            .foreground(ThemeRef::SecondaryText)
            .height(48.0)
            .into(),
    ];
    if done && launch_option() != LaunchOption::Hidden {
        rows.push(
            check_box(model.launch)
                .content(tr().get("install.run_now"))
                .on_checked(on_bool(model, set, |m, v| m.launch = v))
                .into(),
        );
    }
    card(vstack(rows).spacing(16.0).padding(Thickness::uniform(16.0)))
        .vertical_alignment(VerticalAlignment::Top)
}

fn error_view(model: &Model) -> Element {
    grid(vec![
        Element::from(
            InfoBar::new(tr().get("install.err_title"))
                .message(tr().get("install.err_sub"))
                .error()
                .is_open(true)
                .is_closable(false)
                .margin(Thickness {
                    left: 0.0,
                    top: 0.0,
                    right: 0.0,
                    bottom: 12.0,
                }),
        )
        .grid_row(0),
        Element::from(card(
            scroll_viewer(
                text_block(model.error_text.clone())
                    .wrap()
                    .selectable()
                    .padding(Thickness::uniform(16.0)),
            )
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto),
        ))
        .grid_row(1),
    ])
    .rows([GridLength::Auto, GridLength::STAR])
    .into()
}

// ---- Button row ----------------------------------------------------------

fn buttons(model: &Model, set: &SetState<Model>, feed: &Feed, seq: &HookRef<u64>) -> Element {
    // An auto-run page drives itself, so it gets no nav at all.
    if model.phase == Phase::Plugin && model.auto_run {
        return vstack(()).height(56.0).into();
    }

    let t = tr();
    let mut right: Vec<Element> = Vec::new();

    match model.phase {
        Phase::License => {
            // With nothing after it, Next is really the install trigger.
            let label = if skip_path() && !has_plugin_pages() {
                "install.install"
            } else {
                "install.next"
            };
            right.push(primary(&t.get(label), model, set, feed, seq, !model.busy));
        }
        Phase::Choose => {
            // With plugin pages pending, this advances instead of installing.
            let label = if has_plugin_pages() {
                "install.next"
            } else {
                "install.install"
            };
            let enabled = !model.busy && !blocks_install(&model.path);
            right.push(primary(&t.get(label), model, set, feed, seq, enabled));
        }
        Phase::Plugin => {
            right.push(primary(
                &t.get("install.next"),
                model,
                set,
                feed,
                seq,
                !model.busy,
            ));
        }
        Phase::Done | Phase::Error => {
            let finish = {
                let model = model.clone();
                move || {
                    if model.launch && model.phase == Phase::Done {
                        worker::launch_product(Path::new(&model.path));
                    }
                    super::close_window();
                }
            };
            right.push(
                button(t.get("install.finish"))
                    .accent()
                    .on_click(finish)
                    .width(120.0)
                    .with_key("btn-primary")
                    .into(),
            );
        }
        Phase::Progress => {}
    }

    if matches!(
        model.phase,
        Phase::License | Phase::Choose | Phase::Plugin | Phase::Progress
    ) {
        let cancel = on(model, set, |m| {
            if m.phase == Phase::Progress {
                // Confirm first, so the worker can roll back cleanly.
                m.dialog = Some(Dialog::ConfirmCancel);
            } else {
                super::close_window();
            }
        });
        // Keyed, or entering Progress morphs the accent primary into Cancel and
        // leaves it wearing the accent style.
        right.push(
            button(t.get("install.cancel"))
                .on_click(cancel)
                .width(120.0)
                .with_key("btn-cancel")
                .into(),
        );
    }

    let mut cells: Vec<Element> = Vec::new();
    if show_back(model) {
        let back = back_handler(model, set);
        cells.push(
            button(t.get("install.back"))
                .on_click(back)
                .enabled(!model.busy)
                .width(100.0)
                .horizontal_alignment(HorizontalAlignment::Left)
                .grid_column(0)
                .into(),
        );
    }
    cells.push(
        hstack(right)
            .spacing(10.0)
            .horizontal_alignment(HorizontalAlignment::Right)
            .grid_column(1)
            .into(),
    );

    grid(cells)
        .columns([GridLength::Auto, GridLength::STAR])
        .padding(Thickness {
            left: PAD,
            top: 8.0,
            right: PAD,
            bottom: PAD,
        })
        .into()
}

fn primary(
    label: &str,
    model: &Model,
    set: &SetState<Model>,
    feed: &Feed,
    seq: &HookRef<u64>,
    enabled: bool,
) -> Element {
    let handler = next_handler(model, set, feed, seq);
    button(label.to_string())
        .accent()
        .enabled(enabled)
        .on_click(handler)
        .width(110.0)
        .with_key("btn-primary")
        .into()
}

fn show_back(model: &Model) -> bool {
    match model.phase {
        Phase::Choose => !skip_license(),
        Phase::Plugin => {
            let has_builtin = !skip_path() || !skip_license();
            WIZARD.with(|w| {
                w.borrow()
                    .as_ref()
                    .map(|z| z.wants_back() && (z.can_pop() || has_builtin))
                    .unwrap_or(false)
            })
        }
        _ => false,
    }
}

fn back_handler(model: &Model, set: &SetState<Model>) -> impl Fn() + 'static {
    on(model, set, |m| match m.phase {
        Phase::Choose => m.phase = Phase::License,
        Phase::Plugin => {
            let step = WIZARD.with(|w| w.borrow_mut().as_mut().map(|z| z.back()));
            match step {
                Some(Step::Show) => refresh_page(m),
                // Out of the first plugin page, back to the built-in flow.
                Some(Step::Exit) => {
                    m.phase = if !skip_path() {
                        Phase::Choose
                    } else {
                        Phase::License
                    };
                }
                _ => {}
            }
        }
        _ => {}
    })
}

/// Validates the current phase, then advances or hands off to a worker.
fn next_handler(
    model: &Model,
    set: &SetState<Model>,
    feed: &Feed,
    seq: &HookRef<u64>,
) -> impl Fn() + 'static {
    let model = model.clone();
    let set = set.clone();
    let feed = feed.clone();
    let seq = seq.clone();
    move || {
        let mut m = model.clone();
        match m.phase {
            Phase::License => {
                if !m.license_accepted {
                    m.dialog = Some(Dialog::Warn(tr().get("install.must_accept")));
                } else if !skip_path() {
                    m.phase = Phase::Choose;
                } else if has_plugin_pages() {
                    begin_plugin_wizard(&mut m, &feed, &seq);
                } else {
                    commit_install(&mut m, &feed);
                }
            }
            Phase::Choose => {
                if !validate_path(&mut m) {
                    set.call(m);
                    return;
                }
                if has_plugin_pages() {
                    begin_plugin_wizard(&mut m, &feed, &seq);
                } else {
                    commit_install(&mut m, &feed);
                }
            }
            Phase::Plugin => {
                let page_missing = WIZARD.with(|w| {
                    w.borrow().as_ref().and_then(|z| {
                        z.current_page()
                            .and_then(|p| plugin_page::first_missing_required(p, &m.answers))
                    })
                });
                if page_missing.is_some() {
                    m.dialog = Some(Dialog::Warn(plugin_page::required_warning()));
                    set.call(m);
                    return;
                }
                WIZARD.with(|w| {
                    if let Some(z) = w.borrow_mut().as_mut() {
                        z.commit(m.answers.clone());
                    }
                });
                step_forward(&mut m, &feed, &seq);
            }
            _ => {}
        }
        set.call(m);
    }
}

// ---- Flow control --------------------------------------------------------

/// Pick the first interactive page.
fn start_flow(m: &mut Model, feed: &Feed, seq: &HookRef<u64>) {
    // `--preview` never runs a worker; everything below would try to install.
    #[cfg(debug_assertions)]
    if let Some(phase) = preview_phase() {
        apply_preview(m, phase, feed, seq);
        return;
    }
    if skip_license() && skip_path() {
        if has_plugin_pages() {
            begin_plugin_wizard(m, feed, seq);
        } else {
            commit_install(m, feed);
        }
    } else if skip_license() {
        m.phase = Phase::Choose;
    } else {
        m.phase = Phase::License;
    }
}

fn begin_plugin_wizard(m: &mut Model, feed: &Feed, seq: &HookRef<u64>) {
    m.phase = Phase::Plugin;
    step_forward(m, feed, seq);
}

/// The canned preview wizard steps synchronously; a real one dispatches.
fn step_forward(m: &mut Model, feed: &Feed, seq: &HookRef<u64>) {
    #[cfg(debug_assertions)]
    {
        let canned = WIZARD.with(|w| w.borrow().as_ref().map(|z| z.is_canned()).unwrap_or(false));
        if canned {
            let step = WIZARD.with(|w| w.borrow_mut().as_mut().map(|z| z.step_canned()));
            if let Some(step) = step {
                act_step(m, step, feed, seq);
            }
            return;
        }
    }
    let args = WIZARD.with(|w| w.borrow().as_ref().and_then(|z| z.step_args()));
    let Some(args) = args else {
        // Every plugin is exhausted, so install now.
        commit_install(m, feed);
        return;
    };
    let tag = bump(seq);
    if worker::dispatch_step(feed.clone(), args, tag) {
        m.busy = true;
    }
}

fn act_step(m: &mut Model, step: Step, feed: &Feed, seq: &HookRef<u64>) {
    match step {
        Step::Show => {
            m.phase = Phase::Plugin;
            m.auto_run = false;
            refresh_page(m);
        }
        Step::Install => commit_install(m, feed),
        Step::Exit => {
            m.phase = if !skip_path() {
                Phase::Choose
            } else if !skip_license() {
                Phase::License
            } else {
                m.phase
            };
        }
        Step::AutoRun { marquee } => {
            m.phase = Phase::Plugin;
            m.auto_run = true;
            m.auto_marquee = marquee;
            m.plugin_progress = 0;
            refresh_page(m);
            let args = WIZARD.with(|w| w.borrow().as_ref().and_then(|z| z.step_args()));
            match args {
                Some(args) => {
                    let tag = bump(seq);
                    if worker::dispatch_run(feed.clone(), args, tag, marquee) {
                        m.busy = true;
                    }
                }
                None => commit_install(m, feed),
            }
        }
    }
}

/// Bump the epoch so the render function reloads the page descriptor.
fn refresh_page(m: &mut Model) {
    m.page_epoch = m.page_epoch.wrapping_add(1);
    m.answers = WIZARD
        .with(|w| w.borrow().as_ref().map(|z| z.page_answers()))
        .unwrap_or_default();
}

fn commit_install(m: &mut Model, feed: &Feed) {
    if !validate_path(m) {
        return;
    }
    m.phase = Phase::Progress;
    m.busy = false;
    // From here the X must not silently kill a half-applied install.
    super::set_install_running(true);
    let inputs = WIZARD.with(|w| w.borrow().as_ref().map(|z| z.inputs()).unwrap_or_default());
    worker::start_install(feed.clone(), PathBuf::from(m.path.trim()), inputs);
}

fn validate_path(m: &mut Model) -> bool {
    if m.path.trim().is_empty() {
        m.dialog = Some(Dialog::Warn(tr().get("install.err_no_path")));
        return false;
    }
    if blocks_install(&m.path) {
        m.dialog = Some(Dialog::Warn(tr().get("install.path_not_empty")));
        return false;
    }
    true
}

fn bump(seq: &HookRef<u64>) -> u64 {
    let mut n = seq.borrow_mut();
    *n = n.wrapping_add(1);
    *n
}

// ---- Background signals --------------------------------------------------

/// `false` when nothing changed, so the caller can skip the state write.
fn apply_signal(m: &mut Model, signal: &Signal, feed: &Feed, seq: &HookRef<u64>) -> bool {
    match signal {
        Signal::None => return false,
        Signal::CloseRequested(_) => {
            m.dialog = Some(Dialog::ConfirmCancel);
        }
        Signal::Done => {
            m.phase = Phase::Done;
            m.busy = false;
            super::set_install_running(false);
            // Checked means on when the launch flag is set or an exe is known.
            m.launch = launch_option() == LaunchOption::Checked
                && (super::model::LAUNCH_FLAG.with(|l| *l.borrow())
                    || with_payload(|p| p.manifest.exe.as_ref().is_some_and(|s| !s.is_empty())));
        }
        Signal::Error(text) => {
            m.error_text = text.clone();
            m.phase = Phase::Error;
            m.busy = false;
            super::set_install_running(false);
        }
        Signal::PermError => {
            // The UAC prompt is the user's yes/no, so retry elevated at once.
            let payload = PERM_ERROR.lock().ok().and_then(|mut p| p.take());
            if let Some(payload) = payload {
                worker::start_elevated_install(feed.clone(), payload);
            }
            return false;
        }
        Signal::PermDenied(path) => {
            m.busy = false;
            super::set_install_running(false);
            if !skip_path() {
                m.phase = Phase::Choose;
                m.dialog = Some(Dialog::Warn(tr().get("install.perm_denied_path")));
            } else {
                m.error_text = format!(
                    "No permission to write to:\n{path}\n\nThis location requires administrator \
                     rights."
                );
                m.phase = Phase::Error;
            }
        }
        Signal::Cancelled => {
            super::close_window();
            return false;
        }
        Signal::PluginStep(_) => {
            m.busy = false;
            let outcome = QUERIED_STEP.lock().ok().and_then(|mut s| s.take());
            let Some(outcome) = outcome else {
                return false;
            };
            let step = WIZARD.with(|w| w.borrow_mut().as_mut().map(|z| z.apply_outcome(outcome)));
            match step {
                Some(step) => act_step(m, step, feed, seq),
                None => return false,
            }
        }
        Signal::PluginProgress(v, _) => {
            m.plugin_progress = (*v).min(100);
        }
    }
    true
}

// ---- Dialogs -------------------------------------------------------------

/// Stays mounted for the window's whole life with `is_open` bound to the model.
/// WinUI only animates a dialog open on the transition, so inserting a fresh
/// already-open one never shows it.
fn dialog(model: &Model, set: &SetState<Model>) -> Element {
    let t = tr();
    let (open, text, confirm) = match &model.dialog {
        Some(Dialog::Warn(text)) => (true, text.clone(), false),
        Some(Dialog::ConfirmCancel) => (true, t.get("install.cancel_confirm"), true),
        None => (false, String::new(), false),
    };
    let dismiss = {
        let model = model.clone();
        let set = set.clone();
        move |r: ContentDialogResult| {
            let mut next = model.clone();
            let was_confirm = matches!(next.dialog, Some(Dialog::ConfirmCancel));
            next.dialog = None;
            if was_confirm && r == ContentDialogResult::Primary {
                // Stay up; the worker rolls back, then `Cancelled` closes us.
                super::model::request_cancel();
                next.cancelling = true;
            }
            set.call(next);
        }
    };
    ContentDialog::new(t.get("install.msg_caption"))
        .content(soft_wrap(&text, DIALOG_WRAP_COLS))
        // An empty label hides the button, so Yes/No is confirm-only.
        .primary_button_text(if confirm {
            t.get("install.yes")
        } else {
            String::new()
        })
        .close_button_text(if confirm {
            t.get("install.no")
        } else {
            t.get("install.ok")
        })
        .is_open(open)
        .on_closed(dismiss)
        .into()
}

// ---- Shared predicates ---------------------------------------------------

fn on_bool(
    model: &Model,
    set: &SetState<Model>,
    f: impl Fn(&mut Model, bool) + 'static,
) -> impl Fn(bool) + 'static {
    let model = model.clone();
    let set = set.clone();
    move |v| {
        let mut next = model.clone();
        f(&mut next, v);
        set.call(next);
    }
}

fn on_string(
    model: &Model,
    set: &SetState<Model>,
    f: impl Fn(&mut Model, String) + 'static,
) -> impl Fn(String) + 'static {
    let model = model.clone();
    let set = set.clone();
    move |v| {
        let mut next = model.clone();
        f(&mut next, v);
        set.call(next);
    }
}

fn blocks_install(path: &str) -> bool {
    crate::ui::dest::should_block_nonempty(restriction(), skip_path(), &default_path(), path)
}

/// Roughly what fits a default `ContentDialog` at the body type ramp.
const DIALOG_WRAP_COLS: usize = 58;

/// Fold `text` onto lines of at most `cols`, breaking on whitespace.
///
/// `ContentDialog.Content` is a plain string, which WinUI puts in a non-wrapping
/// `TextBlock`, so a long sentence is clipped at the dialog edge. Existing
/// newlines survive as paragraph breaks and over-long words are left intact.
fn soft_wrap(text: &str, cols: usize) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / cols.max(1));
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut width = 0;
        for word in para.split_whitespace() {
            let w = word.chars().count();
            if width == 0 {
                out.push_str(word);
                width = w;
            } else if width + 1 + w <= cols {
                out.push(' ');
                out.push_str(word);
                width += 1 + w;
            } else {
                out.push('\n');
                out.push_str(word);
                width = w;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{DIALOG_WRAP_COLS, soft_wrap};

    #[test]
    fn wraps_on_whitespace_within_the_column_limit() {
        let wrapped = soft_wrap("aaa bbb ccc ddd", 7);
        assert_eq!(wrapped, "aaa bbb\nccc ddd");
        assert!(wrapped.lines().all(|l| l.chars().count() <= 7));
    }

    #[test]
    fn keeps_existing_newlines_as_paragraph_breaks() {
        assert_eq!(soft_wrap("one\n\ntwo", 20), "one\n\ntwo");
    }

    #[test]
    fn never_splits_a_word_longer_than_the_limit() {
        assert_eq!(soft_wrap("short verylongtoken", 5), "short\nverylongtoken");
    }

    #[test]
    fn folds_every_localized_dialog_string() {
        const KEYS: &[&str] = &[
            "install.cancel_confirm",
            "install.must_accept",
            "install.err_no_path",
            "install.path_not_empty",
            "install.field_required",
            "install.perm_denied_path",
        ];
        for lang in ["en", "fr", "it"] {
            let t = common::i18n::Translator::for_lang(lang);
            for key in KEYS {
                let wrapped = soft_wrap(&t.get(key), DIALOG_WRAP_COLS);
                for line in wrapped.lines() {
                    // Only a single unbreakable token may exceed the limit.
                    assert!(
                        line.chars().count() <= DIALOG_WRAP_COLS
                            || line.split_whitespace().count() == 1,
                        "{lang}/{key}: line too long: {line:?}"
                    );
                }
            }
        }
    }
}

// ---- Dev-only preview ----------------------------------------------------

#[cfg(debug_assertions)]
thread_local! {
    /// The `--preview <view>` argument, empty for a real run.
    static PREVIEW_VIEW: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

#[cfg(debug_assertions)]
pub(super) fn set_preview_phase(view: &str) {
    PREVIEW_VIEW.with(|v| *v.borrow_mut() = view.to_string());
}

/// A `-patch` suffix (`choose-patch`) previews the patch subheader.
#[cfg(debug_assertions)]
fn preview_phase() -> Option<Phase> {
    PREVIEW_VIEW.with(|v| {
        let v = v.borrow();
        if v.is_empty() {
            return None;
        }
        Some(match v.split('-').next().unwrap_or(&v) {
            "choose" => Phase::Choose,
            "plugin" => Phase::Plugin,
            "progress" => Phase::Progress,
            "done" => Phase::Done,
            "error" => Phase::Error,
            _ => Phase::License,
        })
    })
}

#[cfg(debug_assertions)]
fn apply_preview(m: &mut Model, phase: Phase, feed: &Feed, seq: &HookRef<u64>) {
    m.phase = phase;
    match phase {
        // The canned wizard steps synchronously, so this lands on the page.
        Phase::Plugin => begin_plugin_wizard(m, feed, seq),
        // Progress has its own hook, so seed it through the worker's setter.
        Phase::Progress => feed.progress.call(Progress {
            done: 7_700_000,
            total: 12_345_678,
            name: "bin/app.exe".to_string(),
        }),
        Phase::Done => {
            feed.progress.call(Progress {
                done: 12_345_678,
                total: 12_345_678,
                name: String::new(),
            });
            m.launch = true;
        }
        Phase::Error => {
            m.error_text =
                "Sample error: the disk became full while writing bin/app.exe.".to_string();
        }
        _ => {}
    }
}
