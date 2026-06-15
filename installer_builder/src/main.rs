// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

mod args;
mod embed;
mod icon;
mod keygen;
mod pack;
mod version;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = args::Cli::parse();
    match cli.command {
        args::Command::Keygen(a) => keygen::run(&a),
        args::Command::Pack(cli) => {
            let cfg = args::PackArgs::resolve(*cli)?;
            pack::run(&cfg)
        }
    }
}
