// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subcommand implementations.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use vnr_core::geometry::{normalize, resolve_overlap};
use vnr_core::{Backend, BackendKind, Layout, Position, Profile, Severity, export, validate};

use crate::cli::{ChangeArgs, Cli, Command, GuiArgs, SetArgs};

/// Runs the parsed command line.
pub fn run(cli: Cli) -> Result<()> {
    let backend = cli.backend;
    match cli.command {
        None => gui(backend, cli.verbose, &GuiArgs::default()),
        Some(Command::Gui(args)) => gui(backend, cli.verbose, &args),
        Some(Command::List { json }) => list(backend, json),
        Some(Command::Backend { json }) => describe_backend(backend, json),
        Some(Command::Save { path, force }) => save(backend, &path, force),
        Some(Command::Apply { path, change }) => {
            let profile = read_profile(&path)?;
            change_outputs(backend, &change, |current| {
                profile
                    .resolve(current)
                    .with_context(|| format!("cannot apply {}", path.display()))
            })
        }
        Some(Command::Set(args)) => {
            change_outputs(backend, &args.change, |current| set(current, &args))
        }
        Some(Command::Export { format, profile }) => {
            let mut layout = open(backend)?.snapshot()?;
            if let Some(path) = profile {
                layout = read_profile(&path)?.resolve(&layout)?;
            }
            print!("{}", export::render(&layout, format));
            Ok(())
        }
    }
}

fn gui(backend: Option<BackendKind>, verbose: bool, args: &GuiArgs) -> Result<()> {
    let accessible = match (args.accessible, args.no_accessible) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    vnr_gui::run(vnr_gui::Options {
        backend,
        theme: args.theme.clone(),
        accessible,
        demo: args.demo,
        verbose,
    })
}

fn open(backend: Option<BackendKind>) -> Result<Box<dyn Backend>> {
    vnr_backends::open(backend).context("cannot reach the display server")
}

fn list(backend: Option<BackendKind>, json: bool) -> Result<()> {
    let layout = open(backend)?.snapshot()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&layout.outputs)?);
        return Ok(());
    }
    let mut out = String::new();
    for output in &layout.outputs {
        let _ = writeln!(
            out,
            "{} ({}) {}",
            output.name,
            output.label(),
            if output.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        if output.enabled {
            if let Some(mode) = output.mode {
                let _ = writeln!(out, "  mode           {mode}");
            }
            if let Some(source) = &output.mirror_of {
                let _ = writeln!(out, "  mirrors        {source}");
            } else {
                let _ = writeln!(out, "  position       {}", output.position);
            }
            let _ = writeln!(out, "  scale          {}", output.scale);
            let _ = writeln!(out, "  transform      {}", output.transform);
            if let Some(sync) = output.adaptive_sync {
                let _ = writeln!(out, "  adaptive sync  {sync}");
            }
            if output.primary {
                let _ = writeln!(out, "  primary        yes");
            }
        }
        if let Some((w, h)) = output.physical_size_mm {
            let _ = writeln!(out, "  physical size  {w} mm x {h} mm");
        }
        let _ = writeln!(out, "  modes");
        for mode in &output.modes {
            let current = output.mode.is_some_and(|m| m.same_timing(mode));
            let _ = writeln!(
                out,
                "    {mode}{}{}",
                if mode.preferred { " (preferred)" } else { "" },
                if current { " (current)" } else { "" }
            );
        }
    }
    print!("{out}");
    Ok(())
}

fn describe_backend(backend: Option<BackendKind>, json: bool) -> Result<()> {
    let backend = open(backend)?;
    let caps = backend.capabilities();
    if json {
        let value = serde_json::json!({
            "backend": backend.kind(),
            "capabilities": caps,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("backend             {}", backend.kind());
        println!("mirroring           {}", yes_no(caps.mirroring));
        println!("adaptive sync       {}", yes_no(caps.adaptive_sync));
        println!(
            "automatic VRR       {}",
            yes_no(caps.adaptive_sync_automatic)
        );
        println!("primary output      {}", yes_no(caps.primary));
        println!("custom modes        {}", yes_no(caps.custom_modes));
        println!("fractional scale    {}", yes_no(caps.fractional_scale));
        println!("test before apply   {}", yes_no(caps.test_only));
        println!("saves changes       {}", yes_no(caps.persistent));
    }
    Ok(())
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn save(backend: Option<BackendKind>, path: &Path, force: bool) -> Result<()> {
    let backend = open(backend)?.snapshot()?;
    let text = Profile::from_layout(&backend).to_toml()?;
    let text = format!("# Vernier display profile. Apply with: vnr apply <this file>\n{text}");
    if path == Path::new("-") {
        print!("{text}");
        return Ok(());
    }
    if path.exists() && !force {
        bail!("{} exists; pass --force to overwrite it", path.display());
    }
    fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
    eprintln!("saved {}", path.display());
    Ok(())
}

fn read_profile(path: &Path) -> Result<Profile> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Profile::from_toml(&text).with_context(|| format!("in {}", path.display()))
}

/// Computes a new layout from the current one, validates, tests and
/// applies it, then asks the user to keep it or reverts after a timeout.
fn change_outputs(
    backend: Option<BackendKind>,
    change: &ChangeArgs,
    build: impl FnOnce(&Layout) -> Result<Layout>,
) -> Result<()> {
    let mut backend = open(backend)?;
    let caps = backend.capabilities();
    let before = backend.snapshot()?;
    let desired = build(&before)?;

    let issues = validate(&desired, &caps);
    for issue in &issues {
        let label = match issue.severity() {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        eprintln!("{label}: {issue}");
    }
    if issues.iter().any(|i| i.severity() == Severity::Error) {
        bail!("not applying a configuration with errors");
    }
    if desired == before {
        eprintln!("nothing to change");
        return Ok(());
    }

    if caps.test_only {
        backend.test(&desired)?;
    }
    if change.test {
        eprintln!("the display server accepts this configuration");
        return Ok(());
    }
    backend.apply(&desired)?;

    if change.yes || !io::stdin().is_terminal() {
        return Ok(());
    }
    if confirm_keep(Duration::from_secs(change.timeout))? {
        eprintln!("kept");
        Ok(())
    } else {
        backend
            .apply(&before)
            .context("could not restore the previous configuration")?;
        eprintln!("reverted to the previous configuration");
        Ok(())
    }
}

/// Asks on the terminal whether to keep the change; anything but `y`,
/// including silence until `timeout`, means no.
fn confirm_keep(timeout: Duration) -> Result<bool> {
    eprint!(
        "Keep this configuration? [y/N] (reverting in {} s): ",
        timeout.as_secs()
    );
    io::stderr().flush()?;
    let (sender, receiver) = mpsc::channel();
    // Reading stdin blocks with no timeout of its own, so it runs on a
    // detached thread; if the deadline passes the process exits with the
    // thread still waiting, which is harmless.
    thread::spawn(move || {
        let mut line = String::new();
        let answer = io::stdin().lock().read_line(&mut line).map(|_| line);
        let _ = sender.send(answer);
    });
    match receiver.recv_timeout(timeout) {
        Ok(Ok(line)) => Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes" | "YES")),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => {
            eprintln!();
            Ok(false)
        }
    }
}

fn set(current: &Layout, args: &SetArgs) -> Result<Layout> {
    let mut layout = current.clone();
    let index = layout
        .outputs
        .iter()
        .position(|o| o.name == args.output)
        .with_context(|| format!("no output named {:?}", args.output))?;

    if args.on && !layout.outputs[index].enabled {
        layout.enable(&args.output);
    }
    if args.off {
        layout.outputs[index].enabled = false;
    }

    let output = &mut layout.outputs[index];
    let mut geometry_changed = false;
    if let Some(mode) = args.mode {
        output.mode = Some(output.find_mode(&mode).unwrap_or(mode));
        geometry_changed = true;
    }
    if args.preferred {
        output.mode = output.preferred_mode();
        geometry_changed = true;
    }
    if let Some(scale) = args.scale {
        output.scale = scale;
        geometry_changed = true;
    }
    if let Some(transform) = args.transform {
        output.transform = transform;
        geometry_changed = true;
    }
    if let Some(sync) = args.adaptive_sync {
        output.adaptive_sync = Some(sync);
    }
    if let Some(source) = &args.mirror {
        output.mirror_of = Some(source.clone());
    }
    if args.no_mirror {
        output.mirror_of = None;
    }
    if args.primary {
        for (i, other) in layout.outputs.iter_mut().enumerate() {
            other.primary = i == index;
        }
    }

    let relative = [
        (&args.left_of, Side::Left),
        (&args.right_of, Side::Right),
        (&args.above, Side::Above),
        (&args.below, Side::Below),
    ]
    .into_iter()
    .find_map(|(target, side)| target.as_ref().map(|t| (t, side)));

    if let Some(position) = args.position {
        layout.outputs[index].position = position;
    } else if let Some((target, side)) = relative {
        let anchor = layout
            .get(target)
            .and_then(vnr_core::Output::logical_rect)
            .with_context(|| format!("{target:?} is not an enabled, placed output"))?;
        let (width, height) = layout.outputs[index]
            .logical_size()
            .context("the output has no mode to place")?;
        let size = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
        layout.outputs[index].position = match side {
            Side::Left => Position::new(anchor.x - size(width), anchor.y),
            Side::Right => Position::new(anchor.right(), anchor.y),
            Side::Above => Position::new(anchor.x, anchor.y - size(height)),
            Side::Below => Position::new(anchor.x, anchor.bottom()),
        };
    } else if geometry_changed {
        resolve_overlap(&mut layout, index);
    }
    normalize(&mut layout);
    Ok(layout)
}

#[derive(Debug, Clone, Copy)]
enum Side {
    Left,
    Right,
    Above,
    Below,
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use vnr_core::{Mode, Output};

    use super::*;

    fn layout() -> Layout {
        let mut laptop = Output::new("eDP-1");
        laptop.modes = vec![Mode::new(1920, 1080, 60_000)];
        laptop.mode = laptop.modes.first().copied();
        let mut monitor = Output::new("DP-1");
        monitor.modes = vec![Mode::new(2560, 1440, 59_951)];
        monitor.mode = monitor.modes.first().copied();
        monitor.position = Position::new(1920, 0);
        Layout::new(vec![laptop, monitor])
    }

    fn set_args(extra: &[&str]) -> SetArgs {
        let mut command_line = vec!["vnr", "set"];
        command_line.extend_from_slice(extra);
        match Cli::parse_from(command_line).command {
            Some(Command::Set(set)) => set,
            other => panic!("expected set, got {other:?}"),
        }
    }

    #[test]
    fn left_of_moves_and_normalizes() {
        let result = set(&layout(), &set_args(&["DP-1", "--left-of", "eDP-1"])).unwrap();
        assert_eq!(result.get("DP-1").unwrap().position, Position::new(0, 0));
        assert_eq!(
            result.get("eDP-1").unwrap().position,
            Position::new(2560, 0)
        );
    }

    #[test]
    fn scaling_up_pushes_neighbor_free() {
        let result = set(&layout(), &set_args(&["eDP-1", "--scale", "0.5"])).unwrap();
        assert!(
            validate(&result, &vnr_core::Capabilities::default())
                .iter()
                .all(|i| !matches!(i, vnr_core::Issue::Overlap { .. }))
        );
    }

    #[test]
    fn mode_matches_advertised_refresh() {
        let result = set(&layout(), &set_args(&["DP-1", "--mode", "2560x1440@60"])).unwrap();
        assert_eq!(
            result.get("DP-1").unwrap().mode,
            Some(Mode::new(2560, 1440, 59_951))
        );
    }

    #[test]
    fn unknown_output_is_an_error() {
        let _err = set(&layout(), &set_args(&["HDMI-A-9", "--off"]))
            .expect_err("unknown output must fail");
    }
}
