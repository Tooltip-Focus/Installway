// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Copy the icon resources from the packaged exe into the installer and
//! uninstaller .exe so Explorer shows the right thumbnail.
//!
//! Copies every `RT_GROUP_ICON` and `RT_ICON` verbatim, preserving original
//! identifiers, so the target's icon tree is identical to the source and
//! Explorer picks the same icon (no guessing, no rebuilt groups that can
//! drift). RT_RCDATA (our payload) is a different type, so no collision.

use anyhow::{Context, Result, bail};
use std::cell::RefCell;
use std::path::Path;
use windows::Win32::Foundation::FreeLibrary;
use windows::Win32::Foundation::{BOOL, HMODULE, TRUE};
use windows::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, EnumResourceNamesW, FindResourceW,
    LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW, LoadResource,
    LockResource, SizeofResource, UpdateResourceW,
};
use windows::core::PCWSTR;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_NEUTRAL: u16 = 0;

pub struct ExeIcons {
    /// Every RT_GROUP_ICON, keyed by its original name/id.
    pub groups: Vec<(ResName, Vec<u8>)>,
    /// Every RT_ICON, keyed by its original name/id.
    pub icons: Vec<(ResName, Vec<u8>)>,
}

/// Read every icon resource from `exe`, preserving identifiers.
/// Returns `Ok(None)` if the source exe has no icons (still a success).
pub fn extract_from_exe(exe: &Path) -> Result<Option<ExeIcons>> {
    let wide: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let hmod = LoadLibraryExW(
            PCWSTR(wide.as_ptr()),
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
        .with_context(|| format!("LoadLibraryEx {}", exe.display()))?;
        if hmod.is_invalid() {
            bail!("LoadLibraryEx returned null for {}", exe.display());
        }

        let group_names = enum_resource_names(hmod, RT_GROUP_ICON);
        if group_names.is_empty() {
            let _ = FreeLibrary(hmod);
            return Ok(None);
        }

        let mut groups = Vec::with_capacity(group_names.len());
        for g in &group_names {
            if let Ok(b) = load_res_named(hmod, RT_GROUP_ICON, g) {
                groups.push((g.clone(), b));
            }
        }

        let icon_names = enum_resource_names(hmod, RT_ICON);
        let mut icons = Vec::with_capacity(icon_names.len());
        for ic in &icon_names {
            if let Ok(b) = load_res_named(hmod, RT_ICON, ic) {
                icons.push((ic.clone(), b));
            }
        }

        let _ = FreeLibrary(hmod);
        if groups.is_empty() || icons.is_empty() {
            return Ok(None);
        }
        Ok(Some(ExeIcons { groups, icons }))
    }
}

/// A resource name is either an integer id or a string (MAKEINTRESOURCE vs name).
#[derive(Clone)]
pub enum ResName {
    Int(u16),
    /// Null-terminated wide string.
    Name(Vec<u16>),
}

impl ResName {
    fn as_pcwstr(&self) -> PCWSTR {
        match self {
            ResName::Int(i) => PCWSTR(*i as usize as *const u16),
            ResName::Name(w) => PCWSTR(w.as_ptr()),
        }
    }
}

unsafe fn enum_resource_names(hmod: HMODULE, rt: u16) -> Vec<ResName> {
    thread_local! {
        static FOUND: RefCell<Vec<ResName>> = const { RefCell::new(Vec::new()) };
    }
    FOUND.with(|f| f.borrow_mut().clear());

    unsafe extern "system" fn cb(_hmod: HMODULE, _ty: PCWSTR, name: PCWSTR, _l: isize) -> BOOL {
        let v = name.0 as usize;
        // IS_INTRESOURCE: high word zero → integer id; else pointer to a wide string.
        if v >> 16 == 0 {
            FOUND.with(|f| f.borrow_mut().push(ResName::Int(v as u16)));
        } else {
            unsafe {
                let mut len = 0usize;
                while *name.0.add(len) != 0 {
                    len += 1;
                }
                let mut w: Vec<u16> = std::slice::from_raw_parts(name.0, len).to_vec();
                w.push(0);
                FOUND.with(|f| f.borrow_mut().push(ResName::Name(w)));
            }
        }
        TRUE
    }

    let _ =
        unsafe { EnumResourceNamesW(Some(hmod), PCWSTR(rt as usize as *const u16), Some(cb), 0) };
    FOUND.with(|f| f.borrow().clone())
}

unsafe fn load_res_named(hmod: HMODULE, rt: u16, name: &ResName) -> Result<Vec<u8>> {
    unsafe {
        let hres = FindResourceW(
            Some(hmod.into()),
            name.as_pcwstr(),
            PCWSTR(rt as usize as *const u16),
        );
        if hres.is_invalid() {
            bail!("FindResource type={} missing", rt);
        }
        let size = SizeofResource(Some(hmod.into()), hres);
        if size == 0 {
            bail!("SizeofResource returned 0");
        }
        let hglobal = LoadResource(Some(hmod.into()), hres).context("LoadResource")?;
        let ptr = LockResource(hglobal);
        if ptr.is_null() {
            bail!("LockResource returned null");
        }
        let slice = std::slice::from_raw_parts(ptr as *const u8, size as usize);
        Ok(slice.to_vec())
    }
}

pub fn embed_icons(target: &Path, icons: &ExeIcons) -> Result<()> {
    let wide: Vec<u16> = target
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let h = BeginUpdateResourceW(PCWSTR(wide.as_ptr()), false)
            .with_context(|| format!("BeginUpdateResource {}", target.display()))?;
        if h.is_invalid() {
            bail!(
                "BeginUpdateResource invalid handle for {}",
                target.display()
            );
        }

        // RT_ICON first, then RT_GROUP_ICON (group references the icons).
        for (name, bytes) in &icons.icons {
            UpdateResourceW(
                h,
                PCWSTR(RT_ICON as usize as *const u16),
                name.as_pcwstr(),
                LANG_NEUTRAL,
                Some(bytes.as_ptr() as *const _),
                bytes.len() as u32,
            )
            .context("UpdateResource RT_ICON")?;
        }
        for (name, bytes) in &icons.groups {
            UpdateResourceW(
                h,
                PCWSTR(RT_GROUP_ICON as usize as *const u16),
                name.as_pcwstr(),
                LANG_NEUTRAL,
                Some(bytes.as_ptr() as *const _),
                bytes.len() as u32,
            )
            .context("UpdateResource RT_GROUP_ICON")?;
        }

        EndUpdateResourceW(h, false).context("EndUpdateResource")?;
    }
    Ok(())
}
