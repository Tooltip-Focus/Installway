// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cleanup;
mod stages;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Uninstaller — invoked by Windows' "Add or remove programs".
///
/// All flags are intentionally hidden: this is a GUI application launched by
/// the system, not a tool meant to be called manually.
#[derive(Parser)]
#[command(
    name = "uninstall",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Silent (non-interactive) uninstall. Also accepted as `/S` (NSIS compat).
    #[arg(long)]
    silent: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Internal second-stage cleanup, spawned from %TEMP% after the main uninstall.
    ///
    /// This sub-command is not meant to be called by users; it is spawned
    /// automatically by the uninstall stage to delete directories that were
    /// locked while the first stage was running.
    #[command(hide = true)]
    Finalize {
        /// Application directory to remove.
        /// Omitted when the metadata was unreadable (best-effort fallback).
        #[arg(long)]
        app_dir: Option<PathBuf>,

        /// Uninstaller data directory to remove
        /// (`%LOCALAPPDATA%\<publisher>\Uninstall\<product>`).
        #[arg(long)]
        data_dir: PathBuf,

        /// Product name, used to find the correct log file in %TEMP%.
        #[arg(long)]
        product: String,

        /// PID of the uninstall process to wait for before deleting anything.
        #[arg(long)]
        parent_pid: Option<u32>,

        /// Product display name for the "uninstall complete" message box.
        #[arg(long)]
        display_name: Option<String>,

        /// Show the "uninstall complete" message box after cleanup finishes.
        #[arg(long)]
        show_complete: bool,
    },
}

fn main() {
    if let Err(e) = run() {
        ui::fatal(&format!("{e:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Collect argv including the binary name (index 0) so that clap can use it
    // in any error messages. Then normalise the NSIS-style `/S` silent flag to
    // `--silent` in-place (only for indices ≥ 1 to leave the binary name alone).
    let mut argv: Vec<String> = std::env::args().collect();

    // Plugin-host child: `--run-plugin <dll> <up|down>`. The context arrives on
    // stdin. Runs before clap so the raw args aren't rejected; exits with the
    // plugin's code.
    if let Some(idx) = argv.iter().position(|a| a == "--run-plugin") {
        let code = match (argv.get(idx + 1), argv.get(idx + 2)) {
            (Some(dll), Some(func)) => {
                common::plugin::host_main(std::path::Path::new(dll), func, None, None)
            }
            _ => 2,
        };
        std::process::exit(code);
    }

    for arg in argv.iter_mut().skip(1) {
        if arg == "/S" {
            *arg = "--silent".to_string();
        }
    }

    // Language detection uses the original user-visible arguments (no argv[0]).
    ui::set_translator(common::i18n::Translator::detect(if argv.len() > 1 {
        &argv[1..]
    } else {
        &[]
    }));

    let cli = Cli::parse_from(&argv);

    match cli.command {
        Some(Cmd::Finalize {
            app_dir,
            data_dir,
            product,
            parent_pid,
            display_name,
            show_complete,
        }) => stages::finalize::run(
            app_dir,
            data_dir,
            product,
            parent_pid,
            display_name,
            show_complete,
        ),
        None => stages::uninstall::run(cli.silent),
    }
}
