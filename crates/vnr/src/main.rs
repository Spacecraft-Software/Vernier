// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `vnr`: the Vernier command line and launcher for the graphical editor.
//!
//! Grammar is `vnr [global flags] <noun> <verb> [args]` (Spacecraft CLI
//! Standard v1.1). On a terminal it prints tables; piped, or under an agent
//! or CI, it prints a `{metadata, data}` JSON envelope. Errors are a single
//! JSON object on stderr in machine mode, with a runnable `hint`.

// Rust guideline compliant 2026-10-19

mod cli;
mod commands;
mod error;
mod output;
mod records;
mod spec;
mod time;
mod validate;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::FromArgMatches;
use clap::error::ErrorKind;

use crate::cli::Cli;
use crate::error::{AppError, ErrorCode};
use crate::output::{ColorChoice, Env, Format, Mode, Out, Severity};

fn main() -> ExitCode {
    install_panic_hook();
    let env = Env::read();
    let args = match validate::arguments(std::env::args_os().collect()) {
        Ok(args) => args,
        Err(error) => return ExitCode::from(early_report(error, &[], &env)),
    };
    ExitCode::from(run(&args, env))
}

fn run(args: &[String], env: Env) -> u8 {
    let matches = match spec::command().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(e) => return clap_error(&e, args, &env),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => return clap_error(&e, args, &env),
    };
    let global = &cli.global;
    let format = match (global.json, global.format) {
        (true, Some(f)) if f != Format::Json => {
            return early_report(
                AppError::new(
                    ErrorCode::InvalidArgument,
                    "`--json` and `--format` disagree",
                    "vnr output list --format json",
                ),
                args,
                &env,
            );
        }
        (true, _) => Some(Format::Json),
        (false, f) => f,
    };
    let color = if global.no_color {
        ColorChoice::Never
    } else {
        global.color.unwrap_or(ColorChoice::Auto)
    };
    let resolved = output::resolve(format, color, global.quiet, global.verbose, &env);
    let palette = resolved
        .color
        .then(|| vnr_theme::resolve(&vnr_theme::Request::default()).tokens);
    let out = Out {
        resolved,
        command: command_line(args),
        fields: global.fields.clone(),
        print0: global.print0,
        dry_run: global.dry_run,
        env,
        palette,
        contact: global.version,
    };
    if let Some(reason) = out.resolved.explore_fallback {
        out.diag(
            Severity::Warn,
            "TUI_FALLBACK",
            &format!("the explorer cannot start ({reason}); printing JSON instead"),
            Some(&format!(
                "{} --json",
                out.command
                    .split_whitespace()
                    .filter(|w| *w != "--format" && *w != "explore" && *w != "--format=explore")
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
        );
    }
    if format == Some(Format::Explore) && out.resolved.explore_fallback.is_none() {
        out.diag(
            Severity::Warn,
            "TUI_UNAVAILABLE",
            "the explore viewer is not built yet; printing the table instead",
            None,
        );
    }
    let result = if global.version {
        commands::version(&out)
    } else {
        commands::run(&out, global, cli.command)
    };
    match result {
        Ok(()) => 0,
        Err(error) => out.report(error),
    }
}

/// The invocation as one line, program name normalized to `vnr`.
fn command_line(args: &[String]) -> String {
    std::iter::once("vnr")
        .chain(args.iter().skip(1).map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Guesses the output mode from raw arguments, for errors raised before
/// (or while) clap parses them.
fn raw_mode(args: &[String], env: &Env) -> Mode {
    let mut format = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => format = Some(Format::Json),
            "--format" => {
                format = iter
                    .next()
                    .and_then(|v| clap::ValueEnum::from_str(v, true).ok());
            }
            other => {
                if let Some(v) = other.strip_prefix("--format=") {
                    format = clap::ValueEnum::from_str(v, true).ok();
                }
            }
        }
    }
    output::resolve(format, ColorChoice::Never, false, false, env).mode
}

fn early_report(mut error: AppError, args: &[String], env: &Env) -> u8 {
    error.command = command_line(args);
    output::report(&error, raw_mode(args, env), None);
    error.exit_code
}

/// Maps clap's own failures onto the standard error object (exit 2), and
/// prints help to stdout with exit 0.
fn clap_error(e: &clap::Error, args: &[String], env: &Env) -> u8 {
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            if e.kind() == ErrorKind::DisplayHelp =>
        {
            let _ = write!(io::stdout().lock(), "{}", e.render());
            0
        }
        ErrorKind::DisplayVersion => {
            let _ = write!(io::stdout().lock(), "{}", e.render());
            0
        }
        kind => {
            let code = match kind {
                ErrorKind::MissingRequiredArgument
                | ErrorKind::MissingSubcommand
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => ErrorCode::MissingArgument,
                _ => ErrorCode::InvalidArgument,
            };
            let path: Vec<&str> = args
                .iter()
                .skip(1)
                .map(String::as_str)
                .filter(|a| !a.starts_with('-'))
                .take(2)
                .collect();
            let hint = if path.is_empty() {
                "vnr --help".to_owned()
            } else {
                format!("vnr {} --help", path.join(" "))
            };
            let message = e
                .render()
                .to_string()
                .lines()
                .find(|l| l.starts_with("error:"))
                .map_or_else(
                    || "invalid arguments".to_owned(),
                    |l| l.trim_start_matches("error:").trim().to_owned(),
                );
            early_report(AppError::new(code, message, hint), args, env)
        }
    }
}

/// Release builds abort on panic, so the hook is the last chance to tell
/// the caller what happened in the agreed shape (exit 1, `INTERNAL_ERROR`).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let env = Env::read();
        let args: Vec<String> = std::env::args().collect();
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let error = AppError::new(
            ErrorCode::InternalError,
            format!("internal error{location}: {payload}"),
            "vnr --verbose backend get",
        );
        let _ = early_report(error, &args, &env);
        std::process::exit(1);
    }));
}
