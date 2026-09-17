// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Plugin-page wizard state for WinUI: the shared engine, with no per-page UI
//! state (a page is re-rendered from its descriptor and the answers).

use common::model::plugin_page::PluginInputs;

pub(super) type Wizard = crate::ui::wizard_engine::Wizard<()>;

impl Wizard {
    /// Current answers, filled in from widget defaults for anything unanswered.
    pub(super) fn page_answers(&self) -> PluginInputs {
        let mut out = self.answers().clone();
        if let Some(frame) = self.current() {
            for (k, v) in super::plugin_page::widget_defaults(&frame.page) {
                out.entry(k).or_insert(v);
            }
        }
        out
    }
}
