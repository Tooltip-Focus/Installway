// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! UI-agnostic core for `ui = true` plugin wizards, shared by both backends:
//! the [`Wizard`] state, and the steps each backend runs off the UI thread
//! ([`run_step_query`], [`run_plugin_then_step`], which spawn plugin
//! subprocesses) before applying the returned [`StepOutcome`].

use crate::extract::TempDirGuard;
use common::model::page_step::PageStep;
use common::model::plugin_ctx::PluginContext;
use common::model::plugin_entry::PluginEntry;
use common::model::plugin_page::{PluginInputs, PluginPage};
use common::plugin::InputsByPlugin;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

/// Wizard state extracted for a background query thread (all `Clone + Send`).
pub(crate) struct StepArgs {
    pub(crate) self_exe: PathBuf,
    pub(crate) base_ctx: PluginContext,
    pub(crate) plugins: Vec<(PluginEntry, PathBuf)>,
    pub(crate) cur: usize,
    pub(crate) answers: PluginInputs,
    pub(crate) finished: InputsByPlugin,
    /// Keeps the extracted-DLL temp dir alive, so a detached thread can't read a
    /// DLL after the dir is removed (window closed mid-query).
    pub(crate) keepalive: Option<Arc<TempDirGuard>>,
    pub(crate) on_progress: Option<Box<dyn Fn(u32) + Send>>,
}

pub(crate) enum StepOutcome {
    /// Every plugin finished; install with these answers.
    Install(InputsByPlugin),
    /// Show this page, which belongs to plugin `cur`.
    Page {
        cur: usize,
        answers: PluginInputs,
        finished: InputsByPlugin,
        page: PluginPage,
        notice: String,
        back: bool,
    },
}

/// What the UI does after a wizard transition.
pub(crate) enum Step {
    /// Show the now-current page.
    Show,
    /// Every plugin finished: install with [`Wizard::inputs`].
    Install,
    /// Validation failed: stay on the current page (already warned).
    Stay,
    /// Backed out before the first plugin page: return to the built-in flow.
    Exit,
    /// Page had `buttons: false`: run the plugin's `up` in the background.
    AutoRun { marquee: bool },
}

/// One shown page of the current plugin's path, kept for Back.
pub(crate) struct Frame<U> {
    pub(crate) page: PluginPage,
    back: bool,
    notice: String,
    /// The answers as of this page, so Back restores them.
    answers: PluginInputs,
    /// What the UI keeps for the page (the Win32 backend: its control slot).
    pub(crate) ui: U,
}

/// Drives the per-plugin step loop: ask the plugin for its next page given the
/// answers so far, show it, collect, repeat until `Done`, then the next plugin.
/// The plugin stays a stateless step function; all state lives here.
pub(crate) struct Wizard<U> {
    plugins: Vec<(PluginEntry, PathBuf)>,
    base_ctx: PluginContext,
    self_exe: PathBuf,
    /// Keeps the extracted-DLL temp dir alive, shared into every [`StepArgs`].
    /// `None` for the canned preview wizard (no real DLLs).
    tmp: Option<Arc<TempDirGuard>>,
    /// Index of the current plugin.
    cur: usize,
    /// The current plugin's answers.
    answers: PluginInputs,
    /// The current plugin's path; the last frame is the page shown.
    stack: Vec<Frame<U>>,
    finished: InputsByPlugin,
    /// Preview: replay these steps instead of spawning a plugin.
    canned: Option<VecDeque<PageStep>>,
}

impl<U> Wizard<U> {
    pub(crate) fn new(
        plugins: Vec<(PluginEntry, PathBuf)>,
        base_ctx: PluginContext,
        self_exe: PathBuf,
        tmp: Option<Arc<TempDirGuard>>,
    ) -> Self {
        Wizard {
            plugins,
            base_ctx,
            self_exe,
            tmp,
            cur: 0,
            answers: PluginInputs::new(),
            stack: Vec::new(),
            finished: InputsByPlugin::new(),
            canned: None,
        }
    }

    /// A preview wizard that replays `steps` for one synthetic plugin.
    #[cfg(any(test, debug_assertions))]
    pub(crate) fn canned(steps: Vec<PageStep>) -> Self {
        let plugins = vec![(PluginEntry::default(), PathBuf::new())];
        let mut wizard = Self::new(plugins, PluginContext::default(), PathBuf::new(), None);
        wizard.canned = Some(steps.into());
        wizard
    }

    pub(crate) fn is_canned(&self) -> bool {
        self.canned.is_some()
    }

    /// Whether the current page opts in to Back (the plugin decides per page).
    pub(crate) fn wants_back(&self) -> bool {
        self.stack.last().is_some_and(|f| f.back)
    }

    /// Whether there is a previous plugin page to step back to.
    pub(crate) fn can_pop(&self) -> bool {
        self.stack.len() > 1
    }

    pub(crate) fn current(&self) -> Option<&Frame<U>> {
        self.stack.last()
    }

    /// Header title and subtitle; a plugin-sent notice replaces the subtitle.
    pub(crate) fn current_title(&self) -> (String, String) {
        match self.stack.last() {
            Some(f) if f.notice.is_empty() => (f.page.title.clone(), f.page.subtitle.clone()),
            Some(f) => (f.page.title.clone(), f.notice.clone()),
            None => (String::new(), String::new()),
        }
    }

    pub(crate) fn answers(&self) -> &PluginInputs {
        &self.answers
    }

    /// Final answers, routed per plugin, for the install worker.
    pub(crate) fn inputs(&self) -> InputsByPlugin {
        self.finished.clone()
    }

    /// Record the answers collected on the current page.
    pub(crate) fn commit(&mut self, answers: PluginInputs) {
        self.answers = answers;
        if let Some(frame) = self.stack.last_mut() {
            frame.answers = self.answers.clone();
        }
    }

    /// State for a background [`run_step_query`] or [`run_plugin_then_step`]
    /// call. `None` when every plugin is exhausted.
    pub(crate) fn step_args(&self) -> Option<StepArgs> {
        if self.cur >= self.plugins.len() {
            return None;
        }
        Some(StepArgs {
            self_exe: self.self_exe.clone(),
            base_ctx: self.base_ctx.clone(),
            plugins: self.plugins.clone(),
            cur: self.cur,
            answers: self.answers.clone(),
            finished: self.finished.clone(),
            keepalive: self.tmp.clone(),
            on_progress: None,
        })
    }

    /// Step back to the previous page, or signal `Exit` at the first one.
    pub(crate) fn back(&mut self) -> Step {
        if self.stack.len() <= 1 {
            return Step::Exit;
        }
        self.stack.pop();
        if let Some(frame) = self.stack.last() {
            self.answers = frame.answers.clone();
        }
        Step::Show
    }

    /// Replay the next canned step through the same engine as the async path,
    /// so the preview can't drift from production. Synchronous (no subprocess).
    pub(crate) fn step_canned(&mut self, build: impl FnOnce(&PluginPage) -> U) -> Step {
        let mut queued = self.canned.take().unwrap_or_default();
        let answers = std::mem::take(&mut self.answers);
        let finished = std::mem::take(&mut self.finished);
        let outcome = advance_steps(&self.plugins, self.cur, answers, finished, |_, _, _| {
            Ok(queued.pop_front().unwrap_or(PageStep::Done))
        });
        self.canned = Some(queued);
        self.apply_outcome(outcome, build)
    }

    /// Apply a [`StepOutcome`]. A new page gets its UI state from `build`.
    pub(crate) fn apply_outcome(
        &mut self,
        outcome: StepOutcome,
        build: impl FnOnce(&PluginPage) -> U,
    ) -> Step {
        let (cur, answers, finished, page, notice, back) = match outcome {
            StepOutcome::Install(finished) => {
                self.finished = finished;
                return Step::Install;
            }
            StepOutcome::Page {
                cur,
                answers,
                finished,
                page,
                notice,
                back,
            } => (cur, answers, finished, page, notice, back),
        };
        // A new plugin starts a fresh Back path.
        if cur != self.cur {
            self.stack.clear();
        }
        self.cur = cur;
        self.answers = answers;
        self.finished = finished;
        let step = if page.buttons {
            Step::Show
        } else {
            Step::AutoRun {
                marquee: page_marquee(&page),
            }
        };
        let ui = build(&page);
        self.stack.push(Frame {
            page,
            back,
            notice,
            answers: self.answers.clone(),
            ui,
        });
        step
    }
}

/// Call `installway_up` for the current plugin, commit its answers, then keep
/// querying from the next one. Background thread only.
pub(crate) fn run_plugin_then_step(args: StepArgs) -> StepOutcome {
    let StepArgs {
        self_exe,
        base_ctx,
        plugins,
        cur,
        mut answers,
        mut finished,
        keepalive: _keepalive,
        on_progress,
    } = args;
    if let Some((entry, dll)) = plugins.get(cur) {
        let inputs_json = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".into());
        if let Err(e) = common::plugin::run_up_single(
            &self_exe,
            &base_ctx,
            entry,
            dll,
            &inputs_json,
            on_progress,
        ) {
            common::log::warn(format!("plugin '{}' up (wizard): {e:#}", entry.name));
        }
        finished
            .entry(entry.name.clone())
            .or_default()
            .extend(std::mem::take(&mut answers));
    }
    let next = cur + 1;
    advance_steps(
        &plugins,
        next,
        answers,
        finished,
        |entry, dll, answers_json| {
            common::plugin::query_step(&self_exe, &base_ctx, entry, dll, answers_json)
        },
    )
}

/// Advance through plugins until one returns a `Page`, or all are exhausted.
/// Background thread only; `_keepalive` holds the DLL temp dir for the duration.
pub(crate) fn run_step_query(args: StepArgs) -> StepOutcome {
    let StepArgs {
        self_exe,
        base_ctx,
        plugins,
        cur,
        answers,
        finished,
        keepalive: _keepalive,
        on_progress: _,
    } = args;
    advance_steps(
        &plugins,
        cur,
        answers,
        finished,
        |entry, dll, answers_json| {
            common::plugin::query_step(&self_exe, &base_ctx, entry, dll, answers_json)
        },
    )
}

/// Walk plugins from `cur`, asking `query` for each one's next step. `Done`
/// routes that plugin's answers into `finished` and moves on; `Page` stops.
///
/// A `query` error is logged and treated as `Done`, so a broken plugin never
/// blocks the wizard nor drops the ones after it. `query` is injected to keep
/// this testable without spawning subprocesses.
pub(crate) fn advance_steps(
    plugins: &[(PluginEntry, PathBuf)],
    mut cur: usize,
    mut answers: PluginInputs,
    mut finished: InputsByPlugin,
    mut query: impl FnMut(&PluginEntry, &std::path::Path, &str) -> anyhow::Result<PageStep>,
) -> StepOutcome {
    loop {
        if cur >= plugins.len() {
            return StepOutcome::Install(finished);
        }
        let (entry, dll) = &plugins[cur];
        let answers_json = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".into());
        let step = match query(entry, dll, &answers_json) {
            Ok(step) => step,
            Err(e) => {
                common::log::warn(format!("plugin '{}' step: {e:#}", entry.name));
                PageStep::Done
            }
        };
        match step {
            PageStep::Done => {
                finished
                    .entry(entry.name.clone())
                    .or_default()
                    .extend(std::mem::take(&mut answers));
                cur += 1;
            }
            PageStep::Page { page, notice, back } => {
                return StepOutcome::Page {
                    cur,
                    answers,
                    finished,
                    page,
                    notice,
                    back,
                };
            }
        }
    }
}

/// Whether the page's `Progress` widget is marquee. Marquee when absent.
pub(crate) fn page_marquee(page: &PluginPage) -> bool {
    use common::model::plugin_widget::PluginWidget;
    page.widgets
        .iter()
        .find_map(|w| match w {
            PluginWidget::Progress { marquee } => Some(*marquee),
            _ => None,
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::{Step, StepOutcome, Wizard, advance_steps};
    use common::model::page_step::PageStep;
    use common::model::plugin_entry::PluginEntry;
    use common::model::plugin_page::PluginInputs;
    use common::model::plugin_page::PluginPage;
    use common::plugin::InputsByPlugin;
    use std::path::{Path, PathBuf};

    fn plugins(names: &[&str]) -> Vec<(PluginEntry, PathBuf)> {
        names
            .iter()
            .map(|n| {
                (
                    PluginEntry {
                        name: (*n).into(),
                        ..Default::default()
                    },
                    PathBuf::new(),
                )
            })
            .collect()
    }

    fn page(id: &str) -> PageStep {
        PageStep::Page {
            page: PluginPage {
                id: id.into(),
                title: String::new(),
                subtitle: String::new(),
                widgets: vec![],
                buttons: true,
            },
            notice: String::new(),
            back: true,
        }
    }

    fn answers(pairs: &[(&str, &str)]) -> PluginInputs {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn advance_all_done_installs() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(PageStep::Done),
        );
        match out {
            StepOutcome::Install(f) => {
                assert!(f.contains_key("a"));
                assert!(f.contains_key("b"));
            }
            _ => panic!("expected install"),
        }
    }

    #[test]
    fn advance_stops_at_first_page() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "a" {
                    Ok(page("p"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Page { cur, .. } => assert_eq!(cur, 0),
            _ => panic!("expected page"),
        }
    }

    #[test]
    fn advance_routes_answers_to_finishing_plugin() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            answers(&[("region.country", "FR")]),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "b" {
                    Ok(page("p"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Page {
                cur,
                answers,
                finished,
                ..
            } => {
                assert_eq!(cur, 1);
                assert!(answers.is_empty());
                assert_eq!(finished["a"]["region.country"], "FR");
            }
            _ => panic!("expected page"),
        }
    }

    #[test]
    fn advance_skips_plugin_on_error() {
        let pl = plugins(&["a", "b"]);
        let out = advance_steps(
            &pl,
            0,
            answers(&[("a.k", "v")]),
            InputsByPlugin::new(),
            |e, _: &Path, _| {
                if e.name == "a" {
                    Err(anyhow::anyhow!("boom"))
                } else {
                    Ok(PageStep::Done)
                }
            },
        );
        match out {
            StepOutcome::Install(f) => {
                assert_eq!(f["a"]["a.k"], "v");
                assert!(f.contains_key("b"));
            }
            _ => panic!("expected install"),
        }
    }

    #[test]
    fn advance_passes_answers_json_to_query() {
        let pl = plugins(&["a"]);
        let mut seen = String::new();
        advance_steps(
            &pl,
            0,
            answers(&[("region.country", "DOM")]),
            InputsByPlugin::new(),
            |_, _: &Path, json| {
                seen = json.to_string();
                Ok(PageStep::Done)
            },
        );
        assert!(seen.contains("region.country"));
        assert!(seen.contains("DOM"));
    }

    #[test]
    fn advance_threads_state_across_calls() {
        let pl = plugins(&["a"]);
        let out1 = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(page("page1")),
        );
        let (cur, mut answers, finished) = match out1 {
            StepOutcome::Page {
                cur,
                answers,
                finished,
                page,
                ..
            } => {
                assert_eq!(page.id, "page1");
                (cur, answers, finished)
            }
            _ => panic!("expected page1"),
        };
        // The user fills page1; the handler would collect these before re-querying.
        answers.insert("page1.x".into(), "1".into());
        let out2 = advance_steps(&pl, cur, answers, finished, |_, _: &Path, _| {
            Ok(PageStep::Done)
        });
        match out2 {
            StepOutcome::Install(f) => assert_eq!(f["a"]["page1.x"], "1"),
            _ => panic!("expected install"),
        }
    }

    #[test]
    fn advance_empty_plugins_installs() {
        let pl: Vec<(PluginEntry, PathBuf)> = Vec::new();
        let out = advance_steps(
            &pl,
            0,
            PluginInputs::new(),
            InputsByPlugin::new(),
            |_, _: &Path, _| Ok(PageStep::Done),
        );
        assert!(matches!(out, StepOutcome::Install(f) if f.is_empty()));
    }

    /// Back restores the answers of the page it returns to, dropping those of
    /// the page left; stepping past the last page installs with what was kept.
    #[test]
    fn wizard_back_restores_the_answers_of_the_previous_page() {
        let mut w: Wizard<u8> = Wizard::canned(vec![page("p1"), page("p2")]);
        assert!(matches!(w.step_canned(|_| 1), Step::Show));
        assert_eq!(w.current().map(|f| f.ui), Some(1));
        w.commit(answers(&[("p1.a", "x")]));

        assert!(matches!(w.step_canned(|_| 2), Step::Show));
        assert!(w.can_pop() && w.wants_back());
        let mut on_p2 = w.answers().clone();
        on_p2.insert("p2.b".into(), "y".into());
        w.commit(on_p2);

        assert!(matches!(w.back(), Step::Show));
        assert_eq!(w.current().map(|f| f.page.id.as_str()), Some("p1"));
        assert_eq!(w.answers(), &answers(&[("p1.a", "x")]));
        assert!(matches!(w.back(), Step::Exit));

        // No step left: the plugin is done and its kept answers are routed.
        assert!(matches!(w.step_canned(|_| 3), Step::Install));
        assert_eq!(w.inputs()[""], answers(&[("p1.a", "x")]));
    }
}
