// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command implementations.
//!
//! Each command returns its data or an [`AppError`]; `main` decides how to
//! print either. Commands that change displays share [`change`], which
//! validates, tests, applies, and then keeps or reverts.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use vnr_core::geometry::{normalize, resolve_overlap};
use vnr_core::{
    Backend, BackendKind, Capabilities, Layout, Output, Position, Profile, Saved,
    Severity as IssueSeverity, export, validate,
};

use crate::cli::{
    ApplyArgs, BackendVerb, CheckArgs, Command, ConfigVerb, ExportArgs, GetArgs, Global, GuiArgs,
    LayoutVerb, OutputVerb, ProfileVerb, SaveArgs, SchemaArgs, SetArgs,
};
use crate::error::{AppError, ErrorCode};
use crate::output::{Mode, Out, Severity};
use crate::records::{
    ApplyReport, BackendRecord, BindingRecord, ConfigReport, ExportRecord, IssueRecord,
    OutputRecord, VersionRecord, WriteReport, diff,
};
use crate::spec;
use crate::validate::Roots;

/// Runs one parsed command.
///
/// # Errors
///
/// Returns the command's failure, ready to report.
pub fn run(out: &Out, global: &Global, command: Option<Command>) -> Result<(), AppError> {
    let backend = global.backend;
    match command {
        None if out.env.is_automated() => Err(AppError::new(
            ErrorCode::MissingArgument,
            "no command given, and the editor does not open under an agent or CI",
            "vnr describe --json",
        )),
        None => gui(out, global, &GuiArgs::default()),
        Some(Command::Gui(args)) => gui(out, global, &args),
        Some(Command::Output { verb }) => match verb {
            OutputVerb::List(_) => output_list(out, backend),
            OutputVerb::Get(args) => output_get(out, backend, &args),
            OutputVerb::Set(args) => output_set(out, global, &args),
        },
        Some(Command::Profile { verb }) => match verb {
            ProfileVerb::Save(args) => profile_save(out, global, &args),
            ProfileVerb::Apply(args) => profile_apply(out, global, &args),
        },
        Some(Command::Layout {
            verb: LayoutVerb::Export(args),
        }) => layout_export(out, global, &args),
        Some(Command::Backend {
            verb: BackendVerb::Get(_),
        }) => backend_get(out, backend),
        Some(Command::Config {
            verb: ConfigVerb::Check(args),
        }) => config_check(out, &args),
        Some(Command::Schema(args)) => schema(out, &args),
        Some(Command::Describe(_)) => describe(out),
    }
}

/// Prints the version (CLI Standard §3: maintainer and project URL).
///
/// # Errors
///
/// Returns an error only if stdout cannot be written.
pub fn version(out: &Out) -> Result<(), AppError> {
    let record = VersionRecord {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        maintainer: spec::MAINTAINER.to_owned(),
        website: spec::WEBSITE.to_owned(),
        license: "GPL-3.0-or-later".to_owned(),
    };
    out.emit(&record, |_| {
        format!(
            "vnr {}\nMaintained by {}\n{}\nLicense: GPL-3.0-or-later\nBuilt by Spacecraft Software\n",
            record.version, record.maintainer, record.website
        )
    })
}

fn open(out: &Out, kind: Option<BackendKind>) -> Result<Box<dyn Backend>, AppError> {
    let backend = vnr_backends::open(kind).map_err(|e| AppError::backend(&e))?;
    out.diag(
        Severity::Info,
        "RESOLVED",
        &format!("using the `{}` backend", backend.kind()),
        None,
    );
    Ok(backend)
}

fn snapshot(backend: &mut dyn Backend) -> Result<Layout, AppError> {
    backend.snapshot().map_err(|e| AppError::backend(&e))
}

fn gui(out: &Out, global: &Global, args: &GuiArgs) -> Result<(), AppError> {
    if out.env.is_automated() && !global.yes {
        return Err(AppError::new(
            ErrorCode::MissingArgument,
            "the editor blocks until its window closes; under an agent or CI it needs `--yes`",
            "vnr gui --yes",
        ));
    }
    let accessible = match (args.accessible, args.no_accessible) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    let request = vnr_theme::Request {
        theme: args.theme.clone(),
        accessible,
    };
    for line in vnr_theme::resolve(&request).report().lines() {
        out.diag(Severity::Info, "RESOLVED", line, None);
    }
    // A broken configuration file must not keep the editor from opening:
    // warn, and start with the default key bindings.
    let (config, config_problem) = match vnr_config::Config::load_default() {
        Ok((config, _)) => (config, None),
        Err(e) => {
            out.diag(
                Severity::Warn,
                "CONFIG_INVALID",
                &format!("{e}; using the default key bindings"),
                Some(&format!("vnr config check {}", e.path().display())),
            );
            (vnr_config::Config::default(), Some(e.to_string()))
        }
    };
    let missing = vnr_gui::missing_libraries();
    if !missing.is_empty() {
        let nixos = std::path::Path::new("/etc/NIXOS").exists();
        return Err(AppError::missing_libraries(&missing, nixos));
    }
    vnr_gui::run(vnr_gui::Options {
        backend: global.backend,
        theme: args.theme.clone(),
        accessible,
        demo: args.demo,
        verbose: false,
        config,
        config_problem,
    })
    .map_err(|e| {
        AppError::new(
            ErrorCode::BackendUnavailable,
            crate::error::lower_first(&e.to_string()),
            "vnr gui --demo",
        )
    })
}

fn config_check(out: &Out, args: &CheckArgs) -> Result<(), AppError> {
    let requested = match &args.path {
        Some(path) => Some(path.clone()),
        None => vnr_config::default_path().filter(|p| p.is_file()),
    };
    let (config, path) = match requested {
        None => (vnr_config::Config::default(), None),
        Some(requested) => {
            let roots = Roots::standard();
            note_widened(out, &roots);
            let path = roots.check(&requested)?;
            let config = vnr_config::Config::load(&path).map_err(|e| {
                if e.is_missing() {
                    AppError::new(
                        ErrorCode::NotFound,
                        format!("`{}` does not exist", path.display()),
                        "vnr config check",
                    )
                } else {
                    AppError::new(
                        ErrorCode::InvalidArgument,
                        crate::error::lower_first(&e.to_string()),
                        "vnr schema config check",
                    )
                    .with("problems", e.problems())
                }
            })?;
            (config, Some(path))
        }
    };
    let report = ConfigReport {
        source: if path.is_some() { "file" } else { "defaults" }.to_owned(),
        path: path.map(|p| p.display().to_string()),
        overlays: config.overlays,
        previews: config.previews,
        keys: config
            .keys
            .iter()
            .map(|(action, chords)| BindingRecord {
                action: action.to_string(),
                keys: chords.iter().map(ToString::to_string).collect(),
            })
            .collect(),
    };
    out.emit(&report, |out| {
        let rows: Vec<Vec<String>> = report
            .keys
            .iter()
            .map(|b| {
                let keys = if b.keys.is_empty() {
                    "(unbound)".to_owned()
                } else {
                    b.keys.join("  ")
                };
                vec![b.action.clone(), keys]
            })
            .collect();
        format!(
            "Screen labels: {}\nScreen contents: {}\n\n{}",
            if report.overlays { "on" } else { "off" },
            if report.previews { "on" } else { "off" },
            crate::output::table(out, &["ACTION", "KEYS"], &rows)
        )
    })
}

fn output_list(out: &Out, kind: Option<BackendKind>) -> Result<(), AppError> {
    let mut backend = open(out, kind)?;
    let layout = snapshot(backend.as_mut())?;
    let records: Vec<OutputRecord> = layout.outputs.iter().map(OutputRecord::from).collect();
    out.emit(&records, |out| {
        let rows: Vec<Vec<String>> = layout
            .outputs
            .iter()
            .map(|o| {
                let state = match (&o.mirror_of, o.enabled) {
                    (_, false) => "off".to_owned(),
                    (Some(source), true) => format!("mirror:{source}"),
                    (None, true) => "on".to_owned(),
                };
                vec![
                    o.name.clone(),
                    state,
                    o.mode.map_or_else(|| "-".to_owned(), |m| m.to_string()),
                    if o.enabled {
                        o.position.to_string()
                    } else {
                        "-".to_owned()
                    },
                    o.scale.to_string(),
                    o.transform.to_string(),
                    o.adaptive_sync
                        .map_or_else(|| "-".to_owned(), |a| a.to_string()),
                    if o.primary { "yes" } else { "no" }.to_owned(),
                ]
            })
            .collect();
        crate::output::table(
            out,
            &[
                "NAME", "STATE", "MODE", "POSITION", "SCALE", "ROTATION", "VRR", "PRIMARY",
            ],
            &rows,
        )
    })
}

fn find_output<'a>(layout: &'a Layout, name: &str) -> Result<&'a Output, AppError> {
    layout.get(name).ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            format!("no output named `{name}`"),
            "vnr output list --fields name --json",
        )
    })
}

fn output_get(out: &Out, kind: Option<BackendKind>, args: &GetArgs) -> Result<(), AppError> {
    let mut backend = open(out, kind)?;
    let layout = snapshot(backend.as_mut())?;
    let output = find_output(&layout, &args.output)?;
    let record = OutputRecord::from(output);
    out.emit(&record, |_| {
        let mut text = String::new();
        let mut line = |key: &str, value: String| {
            let _ = writeln!(text, "{key:<15}{value}");
        };
        line("name", record.name.clone());
        line("label", output.label());
        for (key, value) in [
            ("make", &record.make),
            ("model", &record.model),
            ("serial", &record.serial),
        ] {
            if let Some(value) = value {
                line(key, value.clone());
            }
        }
        line("enabled", record.enabled.to_string());
        if let Some(mode) = &record.mode {
            line("mode", mode.clone());
        }
        line("position", format!("{},{}", record.x, record.y));
        line("scale", output.scale.to_string());
        line("transform", record.transform.clone());
        if let Some(sync) = &record.adaptive_sync {
            line("adaptive_sync", sync.clone());
        }
        if let Some(source) = &record.mirror_of {
            line("mirror_of", source.clone());
        }
        line("primary", record.primary.to_string());
        if let (Some(w), Some(h)) = (record.physical_width_mm, record.physical_height_mm) {
            line("physical_size", format!("{w} mm x {h} mm"));
        }
        for mode in &output.modes {
            let mut flags = Vec::new();
            if mode.preferred {
                flags.push("preferred");
            }
            if output.mode.is_some_and(|m| m.same_timing(mode)) {
                flags.push("current");
            }
            let suffix = if flags.is_empty() {
                String::new()
            } else {
                format!(" ({})", flags.join(", "))
            };
            // `available`, not `mode`: one key per meaning keeps the text
            // easy to grep and cut.
            line("available", format!("{mode}{suffix}"));
        }
        text
    })
}

fn backend_get(out: &Out, kind: Option<BackendKind>) -> Result<(), AppError> {
    let backend = open(out, kind)?;
    let record = BackendRecord::new(backend.kind(), backend.capabilities());
    out.emit(&record, |_| {
        let value = serde_json::to_value(&record).unwrap_or_default();
        let mut text = String::new();
        if let Value::Object(map) = value {
            for (key, value) in map {
                let shown = match value {
                    Value::Bool(true) => "yes".to_owned(),
                    Value::Bool(false) => "no".to_owned(),
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                let _ = writeln!(text, "{key:<25}{shown}");
            }
        }
        text
    })
}

fn output_set(out: &Out, global: &Global, args: &SetArgs) -> Result<(), AppError> {
    let wants_change = args.on
        || args.off
        || args.mode.is_some()
        || args.preferred
        || args.position.is_some()
        || args.left_of.is_some()
        || args.right_of.is_some()
        || args.above.is_some()
        || args.below.is_some()
        || args.scale.is_some()
        || args.transform.is_some()
        || args.adaptive_sync.is_some()
        || args.mirror.is_some()
        || args.no_mirror
        || args.primary;
    if !wants_change {
        return Err(AppError::new(
            ErrorCode::MissingArgument,
            "`output set` needs at least one setting to change",
            format!(
                "vnr output set {} --mode <WxH@HZ> --pos <X,Y> --scale <N> --dry-run --json",
                args.output
            ),
        ));
    }
    change(
        out,
        global,
        args.timeout,
        args.persist.as_deref(),
        args.save,
        |current| set(current, args),
    )
}

fn profile_apply(out: &Out, global: &Global, args: &ApplyArgs) -> Result<(), AppError> {
    let (path, profile) = read_profile(out, &args.path)?;
    change(
        out,
        global,
        args.timeout,
        args.persist.as_deref(),
        args.save,
        |current| {
            profile
                .resolve(current)
                .map_err(|e| AppError::profile(&e, &path.display().to_string()))
        },
    )
}

fn read_profile(out: &Out, path: &Path) -> Result<(PathBuf, Profile), AppError> {
    let roots = Roots::standard();
    note_widened(out, &roots);
    let path = roots.check(path)?;
    let text = fs::read_to_string(&path).map_err(|e| io_error(&e, &path, "vnr profile save"))?;
    let profile = Profile::from_toml(&text)
        .map_err(|e| AppError::profile(&e, &path.display().to_string()))?;
    Ok((path, profile))
}

fn note_widened(out: &Out, roots: &Roots) {
    if !roots.widened().is_empty() {
        let dirs: Vec<String> = roots
            .widened()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        out.diag(
            Severity::Info,
            "RESOLVED",
            &format!("allowed paths widened to include {}", dirs.join(", ")),
            None,
        );
    }
}

fn io_error(error: &io::Error, path: &Path, retry: &str) -> AppError {
    let shown = path.display();
    match error.kind() {
        io::ErrorKind::NotFound => AppError::new(
            ErrorCode::NotFound,
            format!("`{shown}` does not exist"),
            format!("{retry} {shown}"),
        ),
        io::ErrorKind::PermissionDenied => AppError::new(
            ErrorCode::PermissionDenied,
            format!("no permission to access `{shown}`"),
            format!("ls -l {shown}"),
        ),
        _ => AppError::new(
            ErrorCode::InternalError,
            format!("cannot access `{shown}`: {error}"),
            format!("ls -l {shown}"),
        ),
    }
}

/// The confirm-or-revert pipeline shared by `output set` and `profile apply`.
fn change(
    out: &Out,
    global: &Global,
    timeout: u64,
    persist: Option<&Path>,
    save: bool,
    build: impl FnOnce(&Layout) -> Result<Layout, AppError>,
) -> Result<(), AppError> {
    if save && global.yes {
        return Err(AppError::new(
            ErrorCode::InvalidArgument,
            "`--save` waits for the display server's own keep-or-revert dialog, which `--yes` cannot answer",
            out.command.replace(" --yes", ""),
        ));
    }
    // Refuse a bad `--persist` target before anything on screen changes.
    let persist = persist.map(|p| persist_target(out, p)).transpose()?;
    let mut backend = open(out, global.backend)?;
    let caps = backend.capabilities();
    let save = save && save_supported(out, backend.kind(), caps)?;
    let before = snapshot(backend.as_mut())?;
    let desired = build(&before)?;
    let changes = diff(&before, &desired);
    let found = validate(&desired, &caps);
    let issues: Vec<IssueRecord> = found.iter().map(IssueRecord::from).collect();
    let mut report = ApplyReport {
        backend: backend.kind().to_string(),
        outcome: String::new(),
        changes,
        issues,
        accepted_by_backend: None,
        persistent: caps.persistent,
        persisted: None,
        saved_by_server: None,
    };

    for issue in found
        .iter()
        .filter(|i| i.severity() == IssueSeverity::Warning)
    {
        out.diag(Severity::Warn, "LAYOUT_WARNING", &issue.to_string(), None);
    }
    let has_errors = found.iter().any(|i| i.severity() == IssueSeverity::Error);
    // Saving an unchanged layout still has the server save it.
    let saving = save && !out.dry_run;
    if report.changes.is_empty() && !saving {
        report.outcome = "unchanged".into();
        out.diag(Severity::Info, "UNCHANGED", "nothing needs to change", None);
        report.persisted = persist
            .map(|p| write_persist(out, &p, &before))
            .transpose()?;
        return emit_report(out, &report);
    }
    if has_errors && !out.dry_run {
        return Err(AppError::new(
            ErrorCode::InvalidLayout,
            "the resulting layout has errors and was not sent",
            format!("{} --dry-run --json", out.command.replace(" --yes", "")),
        )
        .with("issues", &report.issues));
    }
    if caps.test_only && !has_errors {
        match backend.test(&desired) {
            Ok(()) => report.accepted_by_backend = Some(true),
            Err(e) if e.is_rejected() => report.accepted_by_backend = Some(false),
            Err(e) => return Err(AppError::backend(&e)),
        }
    }
    if out.dry_run {
        report.outcome = "planned".into();
        report.persisted = persist
            .map(|p| write_persist(out, &p, &desired))
            .transpose()?;
        return emit_report(out, &report);
    }
    if report.accepted_by_backend == Some(false) {
        return Err(AppError::new(
            ErrorCode::Rejected,
            "the display server rejected the configuration in a test; nothing was changed",
            "vnr output list --json",
        ));
    }

    if save {
        report.saved_by_server = Some(save_on_server(out, backend.as_mut(), &desired)?);
        report.outcome = "kept".into();
        return emit_report(out, &report);
    }
    backend.apply(&desired).map_err(|e| AppError::backend(&e))?;
    announce(out, &report);
    let interactive = out.mode() == Mode::Human
        && out.env.interactive()
        && !global.yes
        && !out.env.is_automated();
    report.outcome = if interactive {
        confirm_or_revert(out, backend.as_mut(), &before, timeout)?;
        "kept".into()
    } else {
        "applied".into()
    };
    // Reached only when the change was kept: a revert returned early.
    report.persisted = persist
        .map(|p| write_persist(out, &p, &desired))
        .transpose()?;
    warn_forgotten(out, backend.kind(), caps, &report);
    emit_report(out, &report)
}

/// Warns when the kept change will not outlast a compositor restart or the
/// session, with the command that would make it last.
fn warn_forgotten(out: &Out, kind: BackendKind, caps: Capabilities, report: &ApplyReport) {
    if report.persisted.is_some() || caps.persistent {
        return;
    }
    if matches!(kind, BackendKind::Wlr | BackendKind::Cosmic) {
        out.diag(
            Severity::Warn,
            "NOT_PERSISTENT",
            "the compositor forgets this change when it restarts",
            Some(&format!(
                "{} --persist ~/.config/niri/{}",
                out.command,
                export::NIRI_INCLUDE_FILE
            )),
        );
    } else if caps.save_on_request {
        out.diag(
            Severity::Warn,
            "NOT_PERSISTENT",
            "the display server forgets this change when the session ends",
            Some(&format!("{} --save", without_keep_flags(&out.command))),
        );
    }
}

/// `command` without the flags `--save` rejects (`--yes`, `--timeout N`).
fn without_keep_flags(command: &str) -> String {
    let mut words = command.split(' ');
    let mut kept = Vec::new();
    while let Some(word) = words.next() {
        match word {
            "--yes" | "-y" => {}
            "--timeout" => {
                words.next();
            }
            w if w.starts_with("--timeout=") => {}
            w => kept.push(w),
        }
    }
    kept.join(" ")
}

/// Whether `--save` should go to the server: yes when it saves on request;
/// no, with a note, when it saves every change anyway; an error otherwise.
fn save_supported(out: &Out, kind: BackendKind, caps: Capabilities) -> Result<bool, AppError> {
    if caps.save_on_request {
        return Ok(true);
    }
    if caps.persistent {
        out.diag(
            Severity::Info,
            "RESOLVED",
            &format!("the {kind} backend saves every change itself; `--save` changes nothing"),
            None,
        );
        return Ok(false);
    }
    let hint = if matches!(kind, BackendKind::Wlr | BackendKind::Cosmic) {
        format!(
            "{} --persist ~/.config/niri/{}",
            out.command.replace(" --save", ""),
            export::NIRI_INCLUDE_FILE
        )
    } else {
        "vnr backend get --json".to_owned()
    };
    Err(AppError::new(
        ErrorCode::Unsupported,
        format!("the {kind} backend cannot save a configuration on request"),
        hint,
    ))
}

/// Applies `desired` through the server's own confirmation and saves it.
/// Returns whether it was saved; a refusal is the `REVERTED` error, as when
/// Vernier's own prompt goes unanswered.
fn save_on_server(
    out: &Out,
    backend: &mut dyn Backend,
    desired: &Layout,
) -> Result<bool, AppError> {
    out.diag(
        Severity::Info,
        "CONFIRM_ON_SCREEN",
        "the display server is asking on screen whether to keep this configuration; answer Keep within 20 seconds to save it",
        None,
    );
    match backend
        .apply_and_save(desired)
        .map_err(|e| AppError::backend(&e))?
    {
        Saved::Kept => {
            out.diag(
                Severity::Ok,
                "SAVED",
                "the display server saved the configuration; it returns at every login",
                None,
            );
            Ok(true)
        }
        Saved::Restored => Err(AppError::new(
            ErrorCode::Reverted,
            "the change was not kept in the display server's dialog, so it restored the previous configuration",
            out.command.clone(),
        )),
    }
}

/// Checks a `--persist` file: inside the allowed roots, not niri's main
/// config, not a symlink, and either absent or a file Vernier generated.
fn persist_target(out: &Out, requested: &Path) -> Result<PathBuf, AppError> {
    let roots = Roots::standard().with_compositor_configs();
    note_widened(out, &roots);
    let path = roots.check(requested)?;
    let include = path.with_file_name(export::NIRI_INCLUDE_FILE);
    if path.file_name().is_some_and(|name| name == "config.kdl") {
        return Err(AppError::new(
            ErrorCode::InvalidArgument,
            "`--persist` never writes niri's main config; give it a file the config includes",
            format!("vnr layout export --to niri --write {}", include.display()),
        ));
    }
    // A symlink would carry the write outside the allowed directories.
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(AppError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` is a symbolic link; `--persist` writes only regular files",
                path.display()
            ),
            format!("ls -l {}", path.display()),
        ));
    }
    match fs::read_to_string(&path) {
        Ok(existing) if !export::is_generated(&existing) => Err(AppError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` was not written by Vernier, so `--persist` will not replace it",
                path.display()
            ),
            format!("mv {0} {0}.bak", path.display()),
        )),
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(io_error(&e, &path, "vnr output set <output> --persist"))
        }
        _ => Ok(path),
    }
}

/// Writes `layout` as niri `output` blocks to a checked `--persist` file.
fn write_persist(out: &Out, path: &Path, layout: &Layout) -> Result<WriteReport, AppError> {
    let text = export::niri(layout);
    let mut report = WriteReport {
        path: path.display().to_string(),
        outcome: String::new(),
        bytes: text.len(),
    };
    if fs::read_to_string(path).is_ok_and(|existing| existing == text) {
        report.outcome = "unchanged".into();
    } else if out.dry_run {
        report.outcome = "planned".into();
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(&e, parent, "mkdir -p"))?;
        }
        fs::write(path, &text)
            .map_err(|e| io_error(&e, path, "vnr output set <output> --persist"))?;
        report.outcome = "written".into();
        out.diag(
            Severity::Ok,
            "WRITTEN",
            &format!(
                "wrote `{}`; niri loads it through `include \"{}\"` in config.kdl",
                report.path,
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
            None,
        );
    }
    Ok(report)
}

/// Writes the `APPLIED` diagnostic.
fn announce(out: &Out, report: &ApplyReport) {
    let mut names: Vec<&str> = report.changes.iter().map(|c| c.output.as_str()).collect();
    names.dedup();
    let (changes, outputs) = (report.changes.len(), names.len());
    out.diag(
        Severity::Ok,
        "APPLIED",
        &format!(
            "applied {changes} change{} to {outputs} output{}",
            if changes == 1 { "" } else { "s" },
            if outputs == 1 { "" } else { "s" }
        ),
        None,
    );
}

/// Asks the person to keep the change; restores `before` unless they do.
fn confirm_or_revert(
    out: &Out,
    backend: &mut dyn Backend,
    before: &Layout,
    timeout: u64,
) -> Result<(), AppError> {
    if confirm_keep(out, Duration::from_secs(timeout)) {
        out.diag(Severity::Ok, "KEPT", "kept the new configuration", None);
        return Ok(());
    }
    match backend.apply(before) {
        Ok(()) => Err(AppError::new(
            ErrorCode::Reverted,
            "the change was not confirmed, so the previous configuration was restored",
            format!("{} --yes", out.command),
        )),
        Err(e) => Err(AppError::new(
            ErrorCode::RevertFailed,
            format!(
                "could not restore the previous configuration: {}",
                crate::error::lower_first(&e.to_string())
            ),
            "vnr profile apply <saved profile> --yes",
        )),
    }
}

fn emit_report(out: &Out, report: &ApplyReport) -> Result<(), AppError> {
    out.emit(report, |out| {
        let rows: Vec<Vec<String>> = report
            .changes
            .iter()
            .map(|c| {
                vec![
                    c.output.clone(),
                    c.field.clone(),
                    c.from.clone().unwrap_or_else(|| "-".into()),
                    c.to.clone().unwrap_or_else(|| "-".into()),
                ]
            })
            .collect();
        let mut text = if rows.is_empty() {
            String::new()
        } else {
            crate::output::table(out, &["OUTPUT", "SETTING", "FROM", "TO"], &rows)
        };
        if report.outcome == "planned" {
            let verdict = match report.accepted_by_backend {
                Some(true) => "the display server accepts this configuration",
                Some(false) => "the display server rejects this configuration",
                None => "the display server cannot test configurations",
            };
            let errors = report
                .issues
                .iter()
                .filter(|i| i.severity == "error")
                .count();
            let _ = writeln!(text, "dry run: nothing was changed; {verdict}");
            if errors > 0 {
                let _ = writeln!(text, "dry run: {errors} error(s) would block applying");
            }
        }
        text
    })
}

/// What the keep prompt says before the countdown number.
const KEEP_TAG: &str = "[WARN]";
const KEEP_LEAD: &str = "keep this configuration? [y/N] (reverting in ";

/// Asks on the terminal whether to keep the change; anything but `y`,
/// including silence until `timeout`, means no.
///
/// The seconds count down in place where the terminal can move the cursor,
/// except in accessible mode, where a screen reader would re-read the line
/// every second.
fn confirm_keep(out: &Out, timeout: Duration) -> bool {
    let seconds = timeout.as_secs();
    let width = seconds.to_string().len();
    let animate = io::stderr().is_terminal()
        && !out.env.term_dumb()
        && !vnr_theme::resolve(&vnr_theme::Request::default()).accessible;
    let mut stderr = io::stderr().lock();
    let _ = write!(
        stderr,
        "{} {KEEP_LEAD}{seconds:>width$} s): ",
        out.tint(KEEP_TAG, |p| p.warning, true),
    );
    let _ = stderr.flush();
    drop(stderr);

    let (sender, receiver) = mpsc::channel();
    // Reading stdin has no timeout of its own, so it runs on a detached
    // thread; if the deadline passes the process exits with the thread
    // still waiting, which is harmless.
    thread::spawn(move || {
        let mut line = String::new();
        let answer = io::stdin().lock().read_line(&mut line).map(|_| line);
        let _ = sender.send(answer);
    });

    let column = KEEP_TAG.chars().count() + 1 + KEEP_LEAD.chars().count();
    let deadline = Instant::now() + timeout;
    let mut shown = seconds;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let wait = if animate {
            left.min(Duration::from_secs(1))
        } else {
            left
        };
        match receiver.recv_timeout(wait) {
            Ok(Ok(line)) => {
                // End of input (Ctrl-D) echoes no newline; Enter does.
                if line.is_empty() {
                    let _ = writeln!(io::stderr());
                }
                return matches!(line.trim(), "y" | "Y" | "yes" | "Yes" | "YES");
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = whole_seconds(deadline.saturating_duration_since(Instant::now()));
                if animate && now != shown {
                    shown = now;
                    let mut stderr = io::stderr().lock();
                    let _ = write!(stderr, "{}", countdown(column, width, now));
                    let _ = stderr.flush();
                }
            }
        }
    }
    let _ = writeln!(io::stderr());
    false
}

/// Seconds left, rounded to the nearest whole second.
fn whole_seconds(left: Duration) -> u64 {
    (left.as_millis() + 500)
        .checked_div(1000)
        .map_or(0, |s| u64::try_from(s).unwrap_or(u64::MAX))
}

/// Rewrites the countdown number in place: save the cursor (where the user
/// may be typing), jump to the number's column, write it, restore.
fn countdown(column: usize, width: usize, seconds: u64) -> String {
    format!("\x1b7\r\x1b[{column}C{seconds:>width$}\x1b8")
}

fn profile_save(out: &Out, global: &Global, args: &SaveArgs) -> Result<(), AppError> {
    let roots = Roots::standard();
    note_widened(out, &roots);
    let path = roots.check(&args.path)?;
    let mut backend = open(out, global.backend)?;
    let layout = snapshot(backend.as_mut())?;
    let text = Profile::from_layout(&layout).to_toml().map_err(|e| {
        AppError::new(
            ErrorCode::InternalError,
            crate::error::lower_first(&e.to_string()),
            "vnr --verbose profile save <path>",
        )
    })?;
    let text =
        format!("# Vernier display profile. Apply with: vnr profile apply <this file>\n{text}");
    write_file(out, global, &path, &text, "profile save")
}

/// Writes `text` to `path` idempotently: identical content is `unchanged`,
/// different content needs `--yes`, and `--dry-run` only plans.
fn write_file(
    out: &Out,
    global: &Global,
    path: &Path,
    text: &str,
    verb: &str,
) -> Result<(), AppError> {
    let mut report = WriteReport {
        path: path.display().to_string(),
        outcome: String::new(),
        bytes: text.len(),
    };
    match fs::read_to_string(path) {
        Ok(existing) if existing == text => {
            report.outcome = "unchanged".into();
            out.diag(
                Severity::Info,
                "UNCHANGED",
                &format!("`{}` already has this content", report.path),
                None,
            );
        }
        Ok(_) if !global.yes => {
            return Err(AppError::new(
                ErrorCode::Conflict,
                format!("`{}` exists with different content", report.path),
                format!("vnr {verb} {} --yes", report.path),
            ));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(io_error(&e, path, &format!("vnr {verb}")));
        }
        _ if out.dry_run => report.outcome = "planned".into(),
        _ => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| io_error(&e, parent, "mkdir -p"))?;
            }
            fs::write(path, text).map_err(|e| io_error(&e, path, &format!("vnr {verb}")))?;
            report.outcome = "written".into();
            out.diag(
                Severity::Ok,
                "WRITTEN",
                &format!("wrote `{}`", report.path),
                None,
            );
        }
    }
    out.emit(&report, |_| format!("{}\n", report.path))
}

fn layout_export(out: &Out, global: &Global, args: &ExportArgs) -> Result<(), AppError> {
    let mut backend = open(out, global.backend)?;
    let mut layout = snapshot(backend.as_mut())?;
    if let Some(profile) = &args.profile {
        let (path, profile) = read_profile(out, profile)?;
        layout = profile
            .resolve(&layout)
            .map_err(|e| AppError::profile(&e, &path.display().to_string()))?;
    }
    let text = export::render(&layout, args.to);
    if let Some(target) = &args.write {
        let roots = Roots::standard().with_compositor_configs();
        note_widened(out, &roots);
        let path = roots.check(target)?;
        write_file(
            out,
            global,
            &path,
            &text,
            &format!("layout export --to {} --write", args.to),
        )
    } else {
        let record = ExportRecord {
            format: args.to.to_string(),
            text,
        };
        out.emit(&record, |_| record.text.clone())
    }
}

fn schema(out: &Out, args: &SchemaArgs) -> Result<(), AppError> {
    let document = match (&args.noun, &args.verb) {
        (None, _) => spec::tool_schema(),
        (Some(noun), verb) => {
            let path = match verb {
                Some(verb) => format!("{noun} {verb}"),
                None => noun.clone(),
            };
            if let Some(spec) = spec::find(&path) {
                spec::command_schema(spec)
            } else {
                let matching: Vec<Value> = spec::SPECS
                    .iter()
                    .filter(|s| verb.is_none() && s.path.split_whitespace().next() == Some(noun))
                    .map(spec::command_schema)
                    .collect();
                if matching.is_empty() {
                    return Err(AppError::new(
                        ErrorCode::NotFound,
                        format!("no command `vnr {path}`"),
                        "vnr describe --json",
                    ));
                }
                serde_json::json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "title": format!("vnr {noun}"),
                    "commands": matching,
                })
            }
        }
    };
    print_document(out, &document)
}

fn describe(out: &Out) -> Result<(), AppError> {
    print_document(out, &spec::manifest())
}

/// `schema` and `describe` print their document bare (no envelope): a
/// schema must validate against the JSON Schema meta-schema as it stands.
fn print_document(out: &Out, document: &Value) -> Result<(), AppError> {
    let text = if out.env.stdout_tty() {
        serde_json::to_string_pretty(document)
    } else {
        serde_json::to_string(document)
    }
    .map_err(|e| AppError::new(ErrorCode::InternalError, e.to_string(), "vnr describe"))?;
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{text}");
    let _ = stdout.flush();
    Ok(())
}

/// Applies `output set` arguments to `current`.
fn set(current: &Layout, args: &SetArgs) -> Result<Layout, AppError> {
    let mut layout = current.clone();
    let index = layout
        .outputs
        .iter()
        .position(|o| o.name == args.output)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                format!("no output named `{}`", args.output),
                "vnr output list --fields name --json",
            )
        })?;
    for target in [
        &args.left_of,
        &args.right_of,
        &args.above,
        &args.below,
        &args.mirror,
    ]
    .into_iter()
    .flatten()
    {
        find_output(&layout, target)?;
    }

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
        layout.outputs[index].position = place(&layout, index, target, side)?;
    } else if geometry_changed {
        resolve_overlap(&mut layout, index);
    }
    normalize(&mut layout);
    Ok(layout)
}

/// Where `layout.outputs[index]` goes to sit beside `target`.
fn place(layout: &Layout, index: usize, target: &str, side: Side) -> Result<Position, AppError> {
    let anchor = layout
        .get(target)
        .and_then(Output::logical_rect)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidArgument,
                format!("`{target}` is not an enabled output with a position"),
                format!("vnr output set {target} --on"),
            )
        })?;
    let output = &layout.outputs[index];
    let (wide, tall) = output.logical_size().ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidArgument,
            format!("`{}` has no mode to place", output.name),
            format!("vnr output set {} --on --preferred", output.name),
        )
    })?;
    let pixels = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    Ok(match side {
        Side::Left => Position::new(anchor.x - pixels(wide), anchor.y),
        Side::Right => Position::new(anchor.right(), anchor.y),
        Side::Above => Position::new(anchor.x, anchor.y - pixels(tall)),
        Side::Below => Position::new(anchor.x, anchor.bottom()),
    })
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
    use vnr_core::Mode as DisplayMode;

    use super::*;
    use crate::cli::Cli;

    fn layout() -> Layout {
        let mut laptop = Output::new("eDP-1");
        laptop.modes = vec![DisplayMode::new(1920, 1080, 60_000)];
        laptop.mode = laptop.modes.first().copied();
        let mut monitor = Output::new("DP-1");
        monitor.modes = vec![DisplayMode::new(2560, 1440, 59_951)];
        monitor.mode = monitor.modes.first().copied();
        monitor.position = Position::new(1920, 0);
        Layout::new(vec![laptop, monitor])
    }

    fn set_args(extra: &[&str]) -> SetArgs {
        let mut command_line = vec!["vnr", "output", "set"];
        command_line.extend_from_slice(extra);
        match Cli::parse_from(command_line).command {
            Some(Command::Output {
                verb: OutputVerb::Set(set),
            }) => set,
            other => panic!("expected output set, got {other:?}"),
        }
    }

    #[test]
    fn countdown_rewrites_only_the_number() {
        assert_eq!(countdown(52, 2, 9), "\x1b7\r\x1b[52C 9\x1b8");
        assert_eq!(whole_seconds(Duration::from_millis(13_600)), 14);
        assert_eq!(whole_seconds(Duration::from_millis(13_400)), 13);
        assert_eq!(whole_seconds(Duration::ZERO), 0);
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
    fn mode_matches_advertised_refresh() {
        let result = set(&layout(), &set_args(&["DP-1", "--mode", "2560x1440@60"])).unwrap();
        assert_eq!(
            result.get("DP-1").unwrap().mode,
            Some(DisplayMode::new(2560, 1440, 59_951))
        );
    }

    #[test]
    fn unknown_targets_are_not_found() {
        let err = set(&layout(), &set_args(&["HDMI-A-9", "--off"])).expect_err("unknown output");
        assert_eq!(err.code, ErrorCode::NotFound);
        let err =
            set(&layout(), &set_args(&["DP-1", "--mirror", "nope"])).expect_err("unknown mirror");
        assert_eq!(err.exit_code, 3);
    }
}
