// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "hintway")]
mod analytics;
mod cleanup;
mod stages;
mod ui;
mod worker;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// Uninstaller - invoked by Windows' "Add or remove programs".
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

    /// Silent (non-interactive) uninstall.
    #[arg(long)]
    silent: bool,

    /// Internal: run as the elevated worker, streaming events back over the
    /// named pipe whose name is given here. Spawned via UAC by the main process.
    #[arg(long, hide = true, value_name = "PIPE")]
    elevated_worker: Option<String>,

    /// Internal: plugin-host child. Values are `<dll> <func> [pages_pipe]
    /// [progress_pipe]`; the context arrives on stdin. Re-launched as an
    /// isolated process so a crashing plugin can't stall the uninstall.
    #[arg(long, hide = true, num_args = 2..=4, value_names = ["DLL", "FUNC", "PAGES", "PROGRESS"])]
    run_plugin: Option<Vec<String>>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Finalize stage cleanup, spawned from %TEMP% after the main uninstall.
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
        #[cfg(feature = "hintway")]
        {
            analytics::error("unknown");
            analytics::shutdown();
        }
        ui::fatal(&format!("{e:#}"));
        std::process::exit(1);
    }
    #[cfg(feature = "hintway")]
    analytics::shutdown();
}

fn run() -> Result<()> {
    // Collect argv including the binary name (index 0) so that clap can use it
    // in any error messages.
    let argv: Vec<String> = std::env::args().collect();

    // Language detection uses the original user-visible arguments (no argv[0]).
    let translator =
        common::i18n::Translator::detect(if argv.len() > 1 { &argv[1..] } else { &[] });
    // Record process-wide so the `down` plugin context carries the same language.
    translator.set_global();
    ui::set_translator(translator);

    let cli = Cli::parse_from(&argv);

    // Elevated worker: performs the uninstall with admin rights, streaming
    // events back via the named pipe. Spawned via UAC by the main process.
    if let Some(pipe_name) = cli.elevated_worker.as_deref() {
        let code = match worker::run(pipe_name) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("elevated-worker error: {e:#}");
                1
            }
        };
        std::process::exit(code);
    }

    // Plugin-host child: load the DLL, call the function, exit with its code.
    // The context arrives on stdin; needs no payload. Values are
    // `<dll> <func> [pages_pipe] [progress_pipe]` (clap guarantees the first 2).
    if let Some(args) = cli.run_plugin.as_deref() {
        let code = common::plugin::host_main(
            Path::new(&args[0]),
            &args[1],
            args.get(2).filter(|s| !s.is_empty()).map(String::as_str),
            args.get(3).filter(|s| !s.is_empty()).map(String::as_str),
        );
        std::process::exit(code);
    }

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
        None => {
            #[cfg(feature = "hintway")]
            {
                let mode = if cli.silent { "silent" } else { "interactive" };
                let data_dir = cleanup::self_dir().unwrap_or_default();
                let privilege = match std::env::var("ProgramData") {
                    Ok(pd) if !pd.is_empty() && data_dir.starts_with(&pd) => "admin",
                    _ => "user",
                };
                analytics::init(mode, privilege);
            }
            stages::uninstall::run(cli.silent)
        }
    }
}
