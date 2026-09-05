// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use windows_reactor::*;

/// The themed content card used by Installway WinUI frontends.
pub fn card(child: impl Into<View>) -> crate::compat::Element {
    crate::compat::Element::from(
        Border::new()
            .corner_radius(8.0)
            .background(ThemeBrush::CardBackground)
            .border_thickness(Thickness::uniform(1.0))
            .border_brush(ThemeBrush::CardStroke)
            .content(child),
    )
}
