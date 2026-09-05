// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Binds the process to an installed Windows App Runtime 2.x, version 2.4 or
//! newer.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Security::PSID;
use windows::Win32::Storage::Packaging::Appx::{
    GetPackagesByPackageFamily, PACKAGEDEPENDENCY_CONTEXT,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{HRESULT, PCSTR, PCWSTR, PWSTR, s, w};

/// `TryCreatePackageDependency` and `AddPackageDependency` arrived in Windows 10
/// 2004. Linking them normally would make the whole installer refuse to start on
/// anything older, before any code could pick the Win32 UI, so they are resolved
/// by name instead and a miss just means no WinUI.
type TryCreateFn =
    unsafe extern "system" fn(PSID, PCWSTR, u64, i32, i32, PCWSTR, i32, *mut PWSTR) -> HRESULT;
type AddFn = unsafe extern "system" fn(
    PCWSTR,
    i32,
    i32,
    *mut PACKAGEDEPENDENCY_CONTEXT,
    *mut PWSTR,
) -> HRESULT;

/// `PackageDependencyProcessorArchitectures_None`: match this process.
const ARCH_NONE: i32 = 0;
/// `PackageDependencyLifetimeKind_Process`: released when we exit.
const LIFETIME_PROCESS: i32 = 0;
/// `CreatePackageDependencyOptions_None` / `AddPackageDependencyOptions_None`.
const OPTIONS_NONE: i32 = 0;

fn proc_address(module: HMODULE, name: PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe { GetProcAddress(module, name) }
}

/// Windows App SDK release this build targets, mirroring reactor's
/// `WINDOWSAPPSDK_RELEASE_MAJORMINOR` (0x00020000).
const RELEASE_MAJOR: u16 = 2;
const RELEASE_MINOR: u16 = 0;

const PUBLISHER_ID: &str = "8wekyb3d8bbwe";

/// A `PACKAGE_VERSION` packed into the `UINT64` the dependency APIs take.
const fn version(major: u16, minor: u16, build: u16, revision: u16) -> u64 {
    ((major as u64) << 48) | ((minor as u64) << 32) | ((build as u64) << 16) | revision as u64
}

/// Lower bound for a runtime that can start XAML without a `resources.pri`
/// beside the executable. The dependency resolver may select any newer
/// compatible runtime from the same 2.x family.
const MIN_VERSION: u64 = version(2, 4, 0, 0);

pub(crate) fn bind() -> bool {
    for family in candidate_families() {
        if !family_is_installed(&family) {
            continue;
        }
        if add_dependency(&family, MIN_VERSION) {
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

fn add_dependency(family: &str, min_version: u64) -> bool {
    let name = common::utils::wide(family);
    unsafe {
        // kernelbase is already loaded; this only looks the exports up.
        let Ok(kernelbase) = GetModuleHandleW(w!("kernelbase.dll")) else {
            return false;
        };
        let (Some(try_create), Some(add)) = (
            proc_address(kernelbase, s!("TryCreatePackageDependency")),
            proc_address(kernelbase, s!("AddPackageDependency")),
        ) else {
            common::log::info("dynamic package dependencies need Windows 10 2004 or later");
            return false;
        };
        let try_create: TryCreateFn = std::mem::transmute(try_create);
        let add: AddFn = std::mem::transmute(add);

        let mut id = PWSTR::null();
        let hr = try_create(
            PSID::default(),
            PCWSTR(name.as_ptr()),
            min_version,
            ARCH_NONE,
            LIFETIME_PROCESS,
            PCWSTR::null(),
            OPTIONS_NONE,
            &mut id,
        );
        if hr.is_err() || id.is_null() {
            common::log::info(format!("no runtime for '{family}': {hr:?}"));
            return false;
        }

        let mut ctx = PACKAGEDEPENDENCY_CONTEXT::default();
        let hr = add(
            PCWSTR(id.0),
            0,
            OPTIONS_NONE,
            &mut ctx,
            std::ptr::null_mut(),
        );
        // The id allocation and the context are deliberately leaked: the package
        // must stay in the graph for as long as the UI runs, which is until exit.
        if hr.is_err() {
            common::log::info(format!("add package dependency '{family}': {hr:?}"));
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_VERSION, candidate_families, version};

    /// The packing must match reactor's `WINDOWSAPPSDK_RUNTIME_VERSION_UINT64`,
    /// which is the literal this mirrors.
    #[test]
    fn version_packing_matches_reactor() {
        assert_eq!(MIN_VERSION, 0x0002_0004_0000_0000);
        assert_eq!(version(0, 0, 0, 1), 1);
        assert_eq!(version(1, 8, 0, 0), 0x0001_0008_0000_0000);
    }

    /// 2.x dropped the minor from the family name; the installed package here is
    /// `Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe`.
    #[test]
    fn family_candidates_cover_both_naming_schemes() {
        let fams = candidate_families();
        assert!(fams.contains(&"Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe".to_string()));
        assert!(fams.contains(&"Microsoft.WindowsAppRuntime.2.0_8wekyb3d8bbwe".to_string()));
    }
}
