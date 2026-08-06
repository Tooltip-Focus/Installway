// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Binds the process to the Windows App Runtime installed on the machine.

use windows::Win32::Security::PSID;
use windows::Win32::Storage::Packaging::Appx::{
    AddPackageDependency, AddPackageDependencyOptions_None, CreatePackageDependencyOptions_None,
    GetPackagesByPackageFamily, PACKAGE_VERSION, PACKAGE_VERSION_0, PACKAGEDEPENDENCY_CONTEXT,
    PackageDependencyLifetimeKind_Process, PackageDependencyProcessorArchitectures_None,
    TryCreatePackageDependency,
};
use windows::core::PCWSTR;

/// Windows App SDK release this build targets, mirroring reactor's
/// `WINDOWSAPPSDK_RELEASE_MAJORMINOR` (0x00020000).
const RELEASE_MAJOR: u16 = 2;
const RELEASE_MINOR: u16 = 0;

/// Minimum runtime version, mirroring reactor's
/// `WINDOWSAPPSDK_RUNTIME_VERSION_UINT64`: 2.0.1.0 packed as major/minor/build/rev.
const MIN_VERSION: u64 = 0x0002_0000_0001_0000;

const PUBLISHER_ID: &str = "8wekyb3d8bbwe";

pub(super) fn bind() -> bool {
    for family in candidate_families() {
        if !family_is_installed(&family) {
            continue;
        }
        if add_dependency(&family) {
            common::log::info(format!("bound to Windows App Runtime '{family}'"));
            return true;
        }
    }
    false
}

/// Family names to try, most specific first. Releases from 2.0 dropped the
/// minor from the name (`Microsoft.WindowsAppRuntime.2`), earlier ones keep it
/// (`Microsoft.WindowsAppRuntime.1.8`), so try both.
fn candidate_families() -> Vec<String> {
    let mut out = Vec::with_capacity(2);
    if RELEASE_MINOR == 0 {
        out.push(format!(
            "Microsoft.WindowsAppRuntime.{RELEASE_MAJOR}_{PUBLISHER_ID}"
        ));
    }
    out.push(format!(
        "Microsoft.WindowsAppRuntime.{RELEASE_MAJOR}.{RELEASE_MINOR}_{PUBLISHER_ID}"
    ));
    out
}

/// Whether any package of `family` is registered for this user.
fn family_is_installed(family: &str) -> bool {
    let name = common::utils::wide(family);
    let mut count = 0u32;
    let mut buf_len = 0u32;
    // Sizing call: it reports ERROR_INSUFFICIENT_BUFFER when there are matches,
    // so the count is the answer rather than the return code.
    unsafe {
        let _ =
            GetPackagesByPackageFamily(PCWSTR(name.as_ptr()), &mut count, None, &mut buf_len, None);
    }
    count > 0
}

fn add_dependency(family: &str) -> bool {
    let name = common::utils::wide(family);
    let min = PACKAGE_VERSION {
        Anonymous: PACKAGE_VERSION_0 {
            Version: MIN_VERSION,
        },
    };
    unsafe {
        // `None` architecture means "whatever this process is". A Process
        // lifetime needs no artifact and is released when we exit.
        let id = match TryCreatePackageDependency(
            PSID::default(),
            PCWSTR(name.as_ptr()),
            min,
            PackageDependencyProcessorArchitectures_None,
            PackageDependencyLifetimeKind_Process,
            PCWSTR::null(),
            CreatePackageDependencyOptions_None,
        ) {
            Ok(id) => id,
            Err(e) => {
                common::log::info(format!("no runtime for '{family}': {e}"));
                return false;
            }
        };
        let mut ctx = PACKAGEDEPENDENCY_CONTEXT::default();
        let added = AddPackageDependency(
            PCWSTR(id.0),
            0,
            AddPackageDependencyOptions_None,
            &mut ctx,
            None,
        );
        // The id allocation and the context are deliberately leaked: the package
        // must stay in the graph for as long as the UI runs, which is until exit.
        match added {
            Ok(()) => true,
            Err(e) => {
                common::log::info(format!("add package dependency '{family}': {e}"));
                false
            }
        }
    }
}
