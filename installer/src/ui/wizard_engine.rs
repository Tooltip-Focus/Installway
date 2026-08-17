// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! UI-agnostic stepping core for `ui = true` plugin wizards, shared by both
//! backends. Each keeps its own page state and calls [`run_step_query`] or
//! [`run_plugin_then_step`] off the UI thread (they spawn plugin subprocesses),
//! then applies the returned [`StepOutcome`].

use crate::extract::TempDirGuard;
use common::model::page_step::PageStep;
use common::model::plugin_ctx::PluginContext;
use common::model::plugin_entry::PluginEntry;
use common::model::plugin_page::{PluginInputs, PluginPage};
use common::plugin::InputsByPlugin;
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
    use super::{StepOutcome, advance_steps};
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
}
