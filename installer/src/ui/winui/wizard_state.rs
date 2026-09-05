// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Plugin-page wizard state. Unlike the Win32 `Wizard`, which also owns the
//! controls it built, this only tracks the loop: current plugin, answers so far,
//! and the page stack Back walks. Stepping is [`crate::ui::wizard_engine`].

use crate::extract::TempDirGuard;
use crate::ui::wizard_engine::{StepArgs, StepOutcome};
#[cfg(debug_assertions)]
use common::model::page_step::PageStep;
use common::model::plugin_ctx::PluginContext;
use common::model::plugin_entry::PluginEntry;
use common::model::plugin_page::{PluginInputs, PluginPage};
use common::plugin::InputsByPlugin;
use std::path::PathBuf;
use std::sync::Arc;

/// What the caller does after a wizard transition.
pub(super) enum Step {
    Show,
    Install,
    /// Backed out before the first plugin page; return to the built-in flow.
    Exit,
    /// `buttons: false` page; run the plugin's `up` in the background.
    AutoRun {
        marquee: bool,
    },
}

#[derive(Clone)]
struct Frame {
    page: PluginPage,
    back: bool,
    notice: String,
    /// Answers as of when this page was shown, so Back restores the fields.
    answers: PluginInputs,
}

pub(super) struct Wizard {
    plugins: Vec<(PluginEntry, PathBuf)>,
    base_ctx: PluginContext,
    self_exe: PathBuf,
    /// Shared into [`StepArgs`] so an in-flight query outlives the window.
    tmp: Option<Arc<TempDirGuard>>,
    cur: usize,
    answers: PluginInputs,
    stack: Vec<Frame>,
    finished: InputsByPlugin,
    /// Preview: replay these steps instead of spawning a plugin.
    #[cfg(debug_assertions)]
    canned: Option<std::collections::VecDeque<PageStep>>,
}

impl Wizard {
    pub(super) fn new(
        plugins: Vec<(PluginEntry, PathBuf)>,
        base_ctx: PluginContext,
        self_exe: PathBuf,
        tmp: Arc<TempDirGuard>,
    ) -> Self {
        Wizard {
            plugins,
            base_ctx,
            self_exe,
            tmp: Some(tmp),
            cur: 0,
            answers: PluginInputs::new(),
            stack: Vec::new(),
            finished: InputsByPlugin::new(),
            #[cfg(debug_assertions)]
            canned: None,
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn canned(steps: Vec<PageStep>) -> Self {
        Wizard {
            plugins: vec![(PluginEntry::default(), PathBuf::new())],
            base_ctx: PluginContext::default(),
            self_exe: PathBuf::new(),
            tmp: None,
            cur: 0,
            answers: PluginInputs::new(),
            stack: Vec::new(),
            finished: InputsByPlugin::new(),
            canned: Some(steps.into()),
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn is_canned(&self) -> bool {
        self.canned.is_some()
    }

    /// Whether the current page opts in to Back (the plugin decides per page).
    pub(super) fn wants_back(&self) -> bool {
        self.stack.last().map(|f| f.back).unwrap_or(false)
    }

    pub(super) fn can_pop(&self) -> bool {
        self.stack.len() > 1
    }

    pub(super) fn current_page(&self) -> Option<&PluginPage> {
        self.stack.last().map(|f| &f.page)
    }

    /// Header title and subtitle; a plugin-sent notice replaces the subtitle.
    pub(super) fn current_title(&self) -> (String, String) {
        match self.stack.last() {
            Some(f) => {
                let sub = if f.notice.is_empty() {
                    f.page.subtitle.clone()
                } else {
                    f.notice.clone()
                };
                (f.page.title.clone(), sub)
            }
            None => (String::new(), String::new()),
        }
    }

    pub(super) fn inputs(&self) -> InputsByPlugin {
        self.finished.clone()
    }

    /// Current answers, filled in from widget defaults for anything unanswered.
    pub(super) fn page_answers(&self) -> PluginInputs {
        let mut out = self.answers.clone();
        if let Some(page) = self.current_page() {
            for (k, v) in super::plugin_page::widget_defaults(page) {
                out.entry(k).or_insert(v);
            }
        }
        out
    }

    pub(super) fn commit(&mut self, answers: PluginInputs) {
        self.answers = answers;
        if let Some(frame) = self.stack.last_mut() {
            frame.answers = self.answers.clone();
        }
    }

    /// `None` when every plugin is exhausted.
    pub(super) fn step_args(&self) -> Option<StepArgs> {
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

    /// Replays through the same engine as the async path, so preview can't drift.
    #[cfg(debug_assertions)]
    pub(super) fn step_canned(&mut self) -> Step {
        let mut queued = self.canned.take().unwrap_or_default();
        let answers = std::mem::take(&mut self.answers);
        let finished = std::mem::take(&mut self.finished);
        let outcome = crate::ui::wizard_engine::advance_steps(
            &self.plugins,
            self.cur,
            answers,
            finished,
            |_, _, _| Ok(queued.pop_front().unwrap_or(PageStep::Done)),
        );
        self.canned = Some(queued);
        self.apply_outcome(outcome)
    }

    pub(super) fn back(&mut self) -> Step {
        if self.stack.len() > 1 {
            self.stack.pop();
            if let Some(frame) = self.stack.last() {
                self.answers = frame.answers.clone();
            }
            Step::Show
        } else {
            Step::Exit
        }
    }

    pub(super) fn apply_outcome(&mut self, outcome: StepOutcome) -> Step {
        match outcome {
            StepOutcome::Install(finished) => {
                self.finished = finished;
                Step::Install
            }
            StepOutcome::Page {
                cur,
                answers,
                finished,
                page,
                notice,
                back,
            } => {
                // A new plugin starts a fresh Back path.
                if cur != self.cur {
                    self.stack.clear();
                }
                self.cur = cur;
                self.answers = answers;
                self.finished = finished;
                let auto_run = !page.buttons;
                let marquee = crate::ui::wizard_engine::page_marquee(&page);
                self.stack.push(Frame {
                    page,
                    back,
                    notice,
                    answers: self.answers.clone(),
                });
                if auto_run {
                    Step::AutoRun { marquee }
                } else {
                    Step::Show
                }
            }
        }
    }
}
