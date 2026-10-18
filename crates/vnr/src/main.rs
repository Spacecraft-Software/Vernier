// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `vnr`: the Vernier command line and launcher for the graphical editor.
//!
//! Without a subcommand `vnr` opens the editor. Every subcommand that reads
//! state can print JSON for Nushell (`vnr list --json | from json`).

// Rust guideline compliant 2026-05-18

mod cli;
mod commands;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match commands::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vnr: {error:#}");
            ExitCode::FAILURE
        }
    }
}
