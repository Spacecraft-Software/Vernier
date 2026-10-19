// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Output mode, color, the response envelope, diagnostics and rendering.
//!
//! This module is the only place in `vnr` that writes to stdout or stderr
//! (apart from the confirmation prompt). stdout carries the data payload and
//! nothing else; every diagnostic goes to stderr.

use std::env;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{AppError, ErrorCode};
use crate::time;

/// The binary's name, as it appears in `metadata.tool`.
pub const TOOL: &str = "vnr";

/// `--format` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// One JSON document with `metadata` and `data`.
    Json,
    /// One JSON record per line, no envelope.
    Jsonl,
    /// YAML 1.2, envelope preserved.
    Yaml,
    /// RFC 4180 CSV; metadata goes to stderr.
    Csv,
    /// Interactive table viewer.
    Explore,
}

/// `--color` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorChoice {
    /// Color when stdout is a terminal and the environment allows it.
    #[default]
    Auto,
    /// Always color human output.
    Always,
    /// Never color.
    Never,
}

/// How the payload is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Tables and `[TAG]` lines for a person at a terminal.
    Human,
    /// Machine-readable output in the given format.
    Machine(Format),
}

impl Mode {
    /// Whether this is a machine-readable mode.
    #[must_use]
    pub fn is_machine(self) -> bool {
        matches!(self, Self::Machine(_))
    }
}

/// Diagnostic severity, ordered `Info < Ok < Warn < Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Narration, shown under `--verbose`.
    Info,
    /// A side effect completed.
    Ok,
    /// Degraded, deprecated or fell back.
    Warn,
    /// The command failed.
    Error,
}

impl Severity {
    const fn tag(self) -> &'static str {
        match self {
            Self::Info => "[INFO]",
            Self::Ok => "[OK]",
            Self::Warn => "[WARN]",
            Self::Error => "[ERROR]",
        }
    }
}

/// The parts of the environment the cascade reads, so tests can supply them.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent environment signal"
)]
pub struct Env {
    agent: bool,
    ci: bool,
    stdout_tty: bool,
    stdin_tty: bool,
    term_dumb: bool,
    force_color: bool,
    no_color: bool,
    clicolor_off: bool,
    tool_agent: Option<String>,
}

fn set(name: &str) -> bool {
    env::var_os(name).is_some_and(|v| !v.is_empty())
}

impl Env {
    /// Reads the process environment.
    #[must_use]
    pub fn read() -> Self {
        // Presence-based on purpose (CLI Standard §5): harnesses export
        // descriptive strings such as `claude-code_2-1-218_agent`, so an
        // agent is any non-empty value other than "0" or "false".
        let agent = ["AI_AGENT", "AGENT"]
            .iter()
            .any(|name| env::var(name).is_ok_and(|v| !v.is_empty() && v != "0" && v != "false"));
        let ci = env::var("CI").is_ok_and(|v| !v.is_empty() && v != "0" && v != "false");
        let tool_agent = [
            ("CLAUDECODE", "claude-code"),
            ("CURSOR_AGENT", "cursor"),
            ("GEMINI_CLI", "gemini-cli"),
            ("CODEX_SANDBOX", "codex"),
        ]
        .iter()
        .find(|(name, _)| set(name))
        .map(|(_, label)| (*label).to_owned());
        Self {
            agent,
            ci,
            stdout_tty: io::stdout().is_terminal(),
            stdin_tty: io::stdin().is_terminal(),
            term_dumb: env::var("TERM").as_deref() == Ok("dumb"),
            force_color: set("FORCE_COLOR"),
            no_color: set("NO_COLOR"),
            clicolor_off: env::var("CLICOLOR").as_deref() == Ok("0"),
            tool_agent,
        }
    }

    /// Whether an agent or CI is driving `vnr`, so it must never prompt.
    #[must_use]
    pub fn is_automated(&self) -> bool {
        self.agent || self.ci
    }

    /// Whether stdout is a terminal.
    #[must_use]
    pub fn stdout_tty(&self) -> bool {
        self.stdout_tty
    }

    /// Whether a person can answer a prompt: stdin and stderr-side terminal.
    #[must_use]
    pub fn interactive(&self) -> bool {
        self.stdin_tty && self.stdout_tty && !self.term_dumb
    }
}

/// What resolving the cascade decided.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Output mode.
    pub mode: Mode,
    /// Whether human output carries color escapes.
    pub color: bool,
    /// Minimum severity written to stderr.
    pub floor: Severity,
    /// A `TUI_FALLBACK` reason, when `--format explore` could not start.
    pub explore_fallback: Option<&'static str>,
}

/// Applies the CLI Standard §5 mode cascade and §6 color precedence.
#[must_use]
pub fn resolve(
    format: Option<Format>,
    color: ColorChoice,
    quiet: bool,
    verbose: bool,
    env: &Env,
) -> Resolved {
    let mut explore_fallback = None;
    let mode = match format {
        Some(Format::Explore) => {
            explore_fallback = if !env.stdout_tty {
                Some("stdout is not a terminal")
            } else if env.agent {
                Some("an AI agent is driving vnr")
            } else if env.term_dumb {
                Some("TERM is dumb")
            } else if env.ci {
                Some("CI is set")
            } else {
                None
            };
            if explore_fallback.is_some() {
                Mode::Machine(Format::Json)
            } else {
                // The viewer is not built yet (TODO T-017): the table view
                // is the closest human rendering.
                Mode::Human
            }
        }
        Some(format) => Mode::Machine(format),
        None if env.is_automated() || !env.stdout_tty => Mode::Machine(Format::Json),
        None => Mode::Human,
    };
    let color = mode == Mode::Human
        && match color {
            ColorChoice::Never => false,
            ColorChoice::Always => true,
            ColorChoice::Auto => {
                env.force_color
                    || (!env.no_color && !env.clicolor_off && !env.term_dumb && env.stdout_tty)
            }
        };
    let floor = if quiet {
        Severity::Error
    } else if verbose {
        Severity::Info
    } else if env.agent {
        Severity::Warn
    } else {
        Severity::Ok
    };
    Resolved {
        mode,
        color,
        floor,
        explore_fallback,
    }
}

/// The `metadata` half of the envelope.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Metadata {
    /// Binary name.
    pub tool: String,
    /// Tool version.
    pub version: String,
    /// The invocation.
    pub command: String,
    /// Response time, ISO 8601 UTC.
    pub timestamp: String,
    /// True when nothing was changed because of `--dry-run`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[schemars(default)]
    pub dry_run: bool,
    /// The detected coding agent, when one is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_agent: Option<String>,
    /// Maintainer, on `--version` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
    /// Project website, on `--version` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
}

/// The JSON envelope every data command returns.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Response<T> {
    /// Who answered, when, and to what.
    pub metadata: Metadata,
    /// The payload.
    pub data: T,
}

/// Per-invocation output state shared by every command.
#[derive(Debug, Clone)]
pub struct Out {
    /// Resolved mode, color and floor.
    pub resolved: Resolved,
    /// The invocation, for `metadata.command` and error objects.
    pub command: String,
    /// `--fields`, when given.
    pub fields: Option<Vec<String>>,
    /// `--print0`.
    pub print0: bool,
    /// `--dry-run`.
    pub dry_run: bool,
    /// The environment the cascade read.
    pub env: Env,
    /// Terminal colors, when human output is colored.
    pub palette: Option<vnr_theme::Steelbore>,
    /// Adds maintainer and website to the metadata (`--version`).
    pub contact: bool,
}

/// Writes a 24-bit foreground color escape.
fn paint(text: &str, rgb: vnr_theme::Rgb, bold: bool) -> String {
    let weight = if bold { "1;" } else { "" };
    format!(
        "\x1b[{weight}38;2;{};{};{}m{text}\x1b[0m",
        rgb.r, rgb.g, rgb.b
    )
}

impl Out {
    /// The mode in effect.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.resolved.mode
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            tool: TOOL.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            command: self.command.clone(),
            timestamp: time::now(),
            dry_run: self.dry_run,
            tool_agent: self.env.tool_agent.clone(),
            maintainer: self.contact.then(|| crate::spec::MAINTAINER.to_owned()),
            website: self.contact.then(|| crate::spec::WEBSITE.to_owned()),
        }
    }

    /// Colors `text` with a theme token when human output is colored.
    #[must_use]
    pub fn tint(
        &self,
        text: &str,
        pick: fn(&vnr_theme::Steelbore) -> vnr_theme::Rgb,
        bold: bool,
    ) -> String {
        match &self.palette {
            Some(palette) if self.resolved.color => paint(text, pick(palette), bold),
            _ => text.to_owned(),
        }
    }

    /// Emits a non-error diagnostic, honoring the severity floor.
    pub fn diag(&self, severity: Severity, code: &str, message: &str, hint: Option<&str>) {
        self.diag_with(severity, code, message, hint, Map::new());
    }

    /// Emits a non-error diagnostic with extension fields.
    pub fn diag_with(
        &self,
        severity: Severity,
        code: &str,
        message: &str,
        hint: Option<&str>,
        extensions: Map<String, Value>,
    ) {
        debug_assert!(severity != Severity::Error, "errors go through `report`");
        if severity < self.resolved.floor {
            return;
        }
        let mut line = String::new();
        if self.mode().is_machine() {
            let mut body = Map::new();
            body.insert(
                "severity".into(),
                serde_json::to_value(severity).unwrap_or_default(),
            );
            body.insert("code".into(), code.into());
            body.insert("message".into(), message.into());
            if let Some(hint) = hint {
                body.insert("hint".into(), hint.into());
            }
            body.insert("timestamp".into(), time::now().into());
            body.insert("command".into(), self.command.clone().into());
            body.extend(extensions);
            let mut wrapper = Map::new();
            wrapper.insert("diagnostic".into(), Value::Object(body));
            line = Value::Object(wrapper).to_string();
        } else {
            let tag = match severity {
                Severity::Info => self.tint(severity.tag(), |p| p.structure, false),
                Severity::Ok => self.tint(severity.tag(), |p| p.success, false),
                Severity::Warn | Severity::Error => self.tint(severity.tag(), |p| p.warning, true),
            };
            let _ = write!(line, "{tag} {message}");
            if let Some(hint) = hint {
                let _ = write!(
                    line,
                    "\n  {}",
                    self.tint(&format!("hint: {hint}"), |p| p.accent, false)
                );
            }
        }
        let _ = writeln!(io::stderr().lock(), "{line}");
    }

    /// Writes the success payload for a command. `human` renders the
    /// person-facing text when in human mode; it receives the data.
    ///
    /// # Errors
    ///
    /// Returns an error when `--fields` names an unknown field or the
    /// payload cannot be serialized.
    pub fn emit<T: Serialize>(
        &self,
        data: &T,
        human: impl FnOnce(&Self) -> String,
    ) -> Result<(), AppError> {
        let value = serde_json::to_value(data).map_err(internal)?;
        let value = self.select_fields(value)?;
        let mut stdout = io::stdout().lock();
        let text = match self.mode() {
            _ if self.print0 => print0(&value),
            Mode::Human => human(self),
            Mode::Machine(Format::Json | Format::Explore) => {
                let envelope = Response {
                    metadata: self.metadata(),
                    data: value,
                };
                let json = if self.env.stdout_tty {
                    serde_json::to_string_pretty(&envelope)
                } else {
                    serde_json::to_string(&envelope)
                }
                .map_err(internal)?;
                json + "\n"
            }
            Mode::Machine(Format::Jsonl) => match value {
                Value::Array(items) => items.iter().fold(String::new(), |mut text, item| {
                    let _ = writeln!(text, "{item}");
                    text
                }),
                other => format!("{other}\n"),
            },
            Mode::Machine(Format::Yaml) => {
                let envelope = Response {
                    metadata: self.metadata(),
                    data: value,
                };
                serde_norway::to_string(&envelope).map_err(internal)?
            }
            Mode::Machine(Format::Csv) => {
                let mut wrapper = Map::new();
                wrapper.insert(
                    "metadata".into(),
                    serde_json::to_value(self.metadata()).map_err(internal)?,
                );
                let _ = writeln!(io::stderr().lock(), "{}", Value::Object(wrapper));
                csv(&value)?
            }
        };
        stdout
            .write_all(text.as_bytes())
            .and_then(|()| stdout.flush())
            .or_else(|e| {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .map_err(internal)
    }

    /// Applies `--fields` to a record or a list of records.
    fn select_fields(&self, value: Value) -> Result<Value, AppError> {
        let Some(fields) = &self.fields else {
            return Ok(value);
        };
        let keep = |record: Map<String, Value>| -> Map<String, Value> {
            fields
                .iter()
                .filter_map(|f| record.get(f).map(|v| (f.clone(), v.clone())))
                .collect()
        };
        let known = |record: &Map<String, Value>, field: &str| record.contains_key(field);
        match value {
            Value::Array(items) => {
                let records: Vec<Map<String, Value>> = items
                    .into_iter()
                    .filter_map(|item| match item {
                        Value::Object(map) => Some(map),
                        _ => None,
                    })
                    .collect();
                // A field is valid if any record has it; optional fields are
                // omitted when null, so not every record carries every key.
                if let Some(unknown) = fields
                    .iter()
                    .find(|f| !records.is_empty() && !records.iter().any(|r| known(r, f)))
                {
                    return Err(unknown_field(unknown, &self.command));
                }
                Ok(Value::Array(
                    records
                        .into_iter()
                        .map(|r| Value::Object(keep(r)))
                        .collect(),
                ))
            }
            Value::Object(map) => {
                if let Some(unknown) = fields.iter().find(|f| !known(&map, f)) {
                    return Err(unknown_field(unknown, &self.command));
                }
                Ok(Value::Object(keep(map)))
            }
            other => Ok(other),
        }
    }

    /// Reports an error in the current mode and returns its exit code.
    #[must_use]
    pub fn report(&self, mut error: AppError) -> u8 {
        error.command.clone_from(&self.command);
        error.hint = error.hint.replace(crate::validate::RETRY, &self.command);
        report(
            &error,
            self.mode(),
            self.palette.as_ref().filter(|_| self.resolved.color),
        );
        error.exit_code
    }
}

fn unknown_field(field: &str, command: &str) -> AppError {
    let schema = command
        .split_whitespace()
        .filter(|w| !w.starts_with('-'))
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
        .replacen("vnr", "vnr schema", 1);
    AppError::new(
        ErrorCode::InvalidArgument,
        format!("unknown field `{field}` in `--fields`"),
        schema,
    )
}

/// Writes an error to stderr in the given mode.
pub fn report(error: &AppError, mode: Mode, palette: Option<&vnr_theme::Steelbore>) {
    let text = if mode.is_machine() {
        let mut wrapper = Map::new();
        wrapper.insert(
            "error".into(),
            serde_json::to_value(error).unwrap_or_default(),
        );
        Value::Object(wrapper).to_string()
    } else {
        let (tag, hint) = match palette {
            Some(p) => (
                paint("[ERROR]", p.error, true),
                paint(&format!("hint: {}", error.hint), p.accent, false),
            ),
            None => ("[ERROR]".to_owned(), format!("hint: {}", error.hint)),
        };
        format!("{tag} {}\n  {hint}", error.message)
    };
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
    let _ = stderr.flush();
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::InternalError,
        format!("could not write output: {error}"),
        "vnr --verbose describe",
    )
}

/// NUL-terminated values of each record's first field.
fn print0(value: &Value) -> String {
    let first = |item: &Value| match item {
        Value::Object(map) => map.values().next().map(scalar).unwrap_or_default(),
        other => scalar(other),
    };
    match value {
        Value::Array(items) => items.iter().map(|item| first(item) + "\0").collect(),
        other => first(other) + "\0",
    }
}

/// A JSON value as a single CSV cell.
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(";"),
        other => other.to_string(),
    }
}

/// RFC 4180 CSV of a record or list of records; the header is the union
/// of keys in first-seen order.
fn csv(value: &Value) -> Result<String, AppError> {
    let records: Vec<&Map<String, Value>> = match value {
        Value::Array(items) => items.iter().filter_map(Value::as_object).collect(),
        Value::Object(map) => vec![map],
        _ => Vec::new(),
    };
    let mut header: Vec<&str> = Vec::new();
    for record in &records {
        for key in record.keys() {
            if !header.contains(&key.as_str()) {
                header.push(key);
            }
        }
    }
    let mut writer = ::csv::Writer::from_writer(Vec::new());
    writer.write_record(&header).map_err(internal)?;
    for record in records {
        let row: Vec<String> = header
            .iter()
            .map(|key| record.get(*key).map(scalar).unwrap_or_default())
            .collect();
        writer.write_record(&row).map_err(internal)?;
    }
    let bytes = writer.into_inner().map_err(internal)?;
    String::from_utf8(bytes).map_err(internal)
}

/// Lays out rows as aligned columns under a header and a rule.
///
/// One record per line, columns separated by at least two spaces, so
/// `awk '{print $1}'` yields the first column.
#[must_use]
pub fn table(out: &Out, header: &[&str], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |c| c.chars().count()))
                .chain([h.chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<String>| -> String {
        let mut text = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                text.push_str(cell);
            } else {
                let pad = widths[i].saturating_sub(cell.chars().count()) + 2;
                text.push_str(cell);
                text.push_str(&" ".repeat(pad));
            }
        }
        text.trim_end().to_owned()
    };
    let mut text = String::new();
    let head = line(header.iter().map(|h| (*h).to_owned()).collect());
    let _ = writeln!(text, "{}", out.tint(&head, |p| p.structure, true));
    let rule: usize = widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1);
    let _ = writeln!(text, "{}", out.tint(&"─".repeat(rule), |p| p.border, false));
    for row in rows {
        let _ = writeln!(text, "{}", line(row.clone()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tty() -> Env {
        Env {
            stdout_tty: true,
            stdin_tty: true,
            ..Env::default()
        }
    }

    #[test]
    fn cascade_order() {
        let r = resolve(None, ColorChoice::Auto, false, false, &tty());
        assert_eq!(r.mode, Mode::Human);
        assert!(r.color);
        assert_eq!(r.floor, Severity::Ok);

        let piped = Env::default();
        assert_eq!(
            resolve(None, ColorChoice::Auto, false, false, &piped).mode,
            Mode::Machine(Format::Json)
        );

        let agent = Env {
            agent: true,
            ..tty()
        };
        let r = resolve(None, ColorChoice::Always, false, false, &agent);
        assert_eq!(r.mode, Mode::Machine(Format::Json));
        assert!(!r.color, "machine mode never colors");
        assert_eq!(r.floor, Severity::Warn);

        let explicit = resolve(Some(Format::Yaml), ColorChoice::Auto, false, false, &tty());
        assert_eq!(explicit.mode, Mode::Machine(Format::Yaml));
    }

    #[test]
    fn color_precedence() {
        let no_color = Env {
            no_color: true,
            ..tty()
        };
        assert!(!resolve(None, ColorChoice::Auto, false, false, &no_color).color);
        let forced = Env {
            force_color: true,
            ..no_color
        };
        assert!(resolve(None, ColorChoice::Auto, false, false, &forced).color);
        assert!(!resolve(None, ColorChoice::Never, false, false, &forced).color);
        let dumb = Env {
            term_dumb: true,
            ..tty()
        };
        assert!(!resolve(None, ColorChoice::Auto, false, false, &dumb).color);
    }

    #[test]
    fn floor_flags_beat_agent_env() {
        let agent = Env {
            agent: true,
            ..tty()
        };
        assert_eq!(
            resolve(None, ColorChoice::Auto, false, true, &agent).floor,
            Severity::Info
        );
        assert_eq!(
            resolve(None, ColorChoice::Auto, true, false, &agent).floor,
            Severity::Error
        );
    }

    #[test]
    fn explore_falls_back_when_piped_or_automated() {
        let r = resolve(
            Some(Format::Explore),
            ColorChoice::Auto,
            false,
            false,
            &Env::default(),
        );
        assert_eq!(r.mode, Mode::Machine(Format::Json));
        assert_eq!(r.explore_fallback, Some("stdout is not a terminal"));
        let agent = Env {
            agent: true,
            ..tty()
        };
        assert!(
            resolve(
                Some(Format::Explore),
                ColorChoice::Auto,
                false,
                false,
                &agent
            )
            .explore_fallback
            .is_some()
        );
    }

    #[test]
    fn csv_unions_headers_and_joins_arrays() {
        let value = serde_json::json!([
            {"name": "A", "modes": ["1x1", "2x2"]},
            {"name": "B", "primary": true}
        ]);
        assert_eq!(
            csv(&value).unwrap(),
            "name,modes,primary\nA,1x1;2x2,\nB,,true\n"
        );
    }

    #[test]
    fn print0_takes_first_field() {
        let value = serde_json::json!([{"name": "A", "x": 1}, {"name": "B b"}]);
        assert_eq!(print0(&value), "A\0B b\0");
    }
}
