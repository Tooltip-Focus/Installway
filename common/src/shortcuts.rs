// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Per-user shortcut paths (Desktop + Start Menu).
//!
//! Same layout the launcher uses so behaviour matches across both tools.
//! Returned paths are the canonical `.lnk` locations - exists-check left to caller.

use std::path::PathBuf;

/// Per-user Start Menu Programs directory (no admin needed).
pub fn start_menu_dir() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push(r"Microsoft\Windows\Start Menu\Programs");
    Some(p)
}

/// Per-user Desktop directory.
pub fn desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir()
}

/// Locations of `<product>.lnk` files the installer creates.
pub fn paths_for(product: &str) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(2);
    if let Some(d) = start_menu_dir() {
        out.push(d.join(format!("{}.lnk", product)));
    }
    if let Some(d) = desktop_dir() {
        out.push(d.join(format!("{}.lnk", product)));
    }
    out
}
