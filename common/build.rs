// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Builds and statically links PakLib, the installer payload format.
//!
//! PakLib is the `vendor/PakLib` git submodule. It is configured with CMake
//! and its dependencies (zstd, xxHash) come from vcpkg in manifest mode, as the
//! static `x64-windows-static` triplet with the static CRT, so the installer
//! stays a single self-contained exe. The build lives in Cargo's `OUT_DIR`,
//! leaving the submodule clean. `PAKLIB_BUILD_DIR` points at an existing build
//! instead (a `windows-static` preset build of a PakLib checkout).
//!
//! CMake and vcpkg are taken from `PATH` / `VCPKG_ROOT` when set, else from the
//! Visual Studio installation (both ship with the C++ workload).

use std::path::{Path, PathBuf};
use std::process::Command;

const LIBRARIES: [&str; 3] = ["pak_c_api", "zstd", "xxhash"];
const TRIPLET: &str = "x64-windows-static";

fn main() {
    // `common::pak` only exists on Windows, like the installer itself.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=PAKLIB_BUILD_DIR");

    let build = match std::env::var_os("PAKLIB_BUILD_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => build_submodule(),
    };
    link(&build);
}

/// Configure and build `pak_c_api` from the submodule into `OUT_DIR`.
fn build_submodule() -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("common/ sits inside the workspace")
        .join("vendor")
        .join("PakLib");
    if !source.join("CMakeLists.txt").exists() {
        panic!(
            "PakLib submodule missing at {}. Run `git submodule update --init`.",
            source.display()
        );
    }
    for input in ["CMakeLists.txt", "vcpkg.json", "include", "src"] {
        println!("cargo:rerun-if-changed={}", source.join(input).display());
    }

    let build = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("paklib");
    let cmake = find_cmake();
    let toolchain = find_vcpkg()
        .join("scripts")
        .join("buildsystems")
        .join("vcpkg.cmake");

    run(Command::new(&cmake)
        .arg("-S")
        .arg(&source)
        .arg("-B")
        .arg(&build)
        .args(["-A", "x64"])
        .arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain.display()))
        .arg(format!("-DVCPKG_TARGET_TRIPLET={TRIPLET}"))
        .arg("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded")
        .args([
            "-DPAKLIB_BUILD_TOOLS=OFF",
            "-DPAKLIB_BUILD_TESTS=OFF",
            "-DPAKLIB_BUILD_BENCHMARKS=OFF",
            "-DPAKLIB_ENABLE_INSTALL=OFF",
        ]));
    run(Command::new(&cmake).arg("--build").arg(&build).args([
        "--config",
        "Release",
        "--target",
        "pak_c_api",
        "--parallel",
    ]));
    build
}

/// Link the three static libraries of a PakLib build directory.
fn link(build: &Path) {
    let release = build.join("Release");
    let dependencies = build.join("vcpkg_installed").join(TRIPLET).join("lib");
    for (dir, library) in [
        (&release, LIBRARIES[0]),
        (&dependencies, LIBRARIES[1]),
        (&dependencies, LIBRARIES[2]),
    ] {
        let path = dir.join(format!("{library}.lib"));
        if !path.exists() {
            panic!("PakLib: {} not found", path.display());
        }
        // Static libs are bundled into this crate's rlib, so a rebuilt
        // library must re-run this script or the old one stays linked.
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-link-search=native={}", release.display());
    println!("cargo:rustc-link-search=native={}", dependencies.display());
    for library in LIBRARIES {
        println!("cargo:rustc-link-lib=static={library}");
    }
}

fn find_cmake() -> PathBuf {
    if Command::new("cmake").arg("--version").output().is_ok() {
        return PathBuf::from("cmake");
    }
    visual_studio()
        .map(|vs| vs.join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"))
        .filter(|path| path.exists())
        .unwrap_or_else(|| panic!("CMake not found: install it or the Visual Studio C++ workload"))
}

fn find_vcpkg() -> PathBuf {
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    println!("cargo:rerun-if-env-changed=VCPKG_INSTALLATION_ROOT");
    // VCPKG_INSTALLATION_ROOT is what GitHub's Windows runners set.
    ["VCPKG_ROOT", "VCPKG_INSTALLATION_ROOT"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain(visual_studio().map(|vs| vs.join(r"VC\vcpkg")))
        .find(|root| root.join(r"scripts\buildsystems\vcpkg.cmake").exists())
        .unwrap_or_else(|| {
            panic!("vcpkg not found: set VCPKG_ROOT or install Visual Studio's vcpkg component")
        })
}

/// Latest Visual Studio installation with the C++ tools, via `vswhere`.
fn visual_studio() -> Option<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles(x86)")?;
    let vswhere = Path::new(&program_files).join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let output = Command::new(vswhere)
        .args([
            "-latest",
            "-prerelease",
            "-products",
            "*",
            "-property",
            "installationPath",
        ])
        .args([
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        ])
        .output()
        .ok()?;
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|e| panic!("run {command:?}: {e}"));
    if !status.success() {
        panic!("PakLib build step failed ({status}): {command:?}");
    }
}
