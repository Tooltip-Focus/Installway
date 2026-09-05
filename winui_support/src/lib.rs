// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shared runtime detection and Reactor helpers for Installway WinUI frontends.
//!
//! Installway exclusively uses the framework-dependent, no-PRI deployment
//! shape. Version 2.4 is the minimum; newer compatible 2.x runtimes are
//! accepted automatically.

mod async_value;
pub use async_value::{AsyncValue, Progress};

#[cfg(windows)]
pub mod compat;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
pub mod widgets;
#[cfg(windows)]
pub mod window;

/// Whether this process runs with a package identity inherited from a packaged
/// launcher. Nothing here is ever shipped packaged, so an identity always comes
/// from the parent - see [`runtime::is_packaged`] for why XAML cannot survive it.
#[cfg(windows)]
pub fn inherited_package_identity() -> bool {
    runtime::is_packaged()
}

/// Whether a compatible Windows App Runtime (2.4 or newer) is available and
/// bound to this process for the no-PRI deployment.
#[cfg(windows)]
pub fn available() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| {
        // Last line of defence: a caller that could not shed an inherited
        // package identity still gets a working window instead of a crash.
        if inherited_package_identity() {
            common::log::info("running inside a launcher's package; using the Win32 UI");
            return false;
        }
        if !runtime::bind() {
            common::log::info("no suitable Windows App Runtime; using the Win32 UI");
            return false;
        }
        true
    })
}

/// Bind WinUI and run one Reactor component. `Ok(false)` selects Win32.
#[cfg(windows)]
pub fn launch_app<C>(input: C::Input) -> anyhow::Result<bool>
where
    C: windows_reactor::Component,
{
    if !available() {
        return Ok(false);
    }
    windows_reactor::App::run_component::<C>(input)
        .map_err(|error| anyhow::anyhow!("winui: {error}"))?;
    Ok(true)
}
