// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command catalog: examples, exit codes and behavior flags.
//!
//! This table feeds `--help`, `vnr schema` and `vnr describe`. Descriptions
//! and parameters are not repeated here; they come from the clap command
//! tree and the argument structs.

use clap::CommandFactory;
use std::fmt::Write as _;

use schemars::{Schema, schema_for};
use serde_json::{Map, Value, json};

use crate::cli::{
    ApplyArgs, CheckArgs, Cli, ExportArgs, GetArgs, Global, GuiArgs, NoArgs, SaveArgs, SchemaArgs,
    SetArgs,
};
use crate::error::ErrorCode;
use crate::output::Response;
use crate::records::{
    ApplyReport, BackendRecord, ConfigReport, ExportRecord, OutputRecord, VersionRecord,
    WriteReport,
};

/// Project page (Steelbore Standard §15).
pub const WEBSITE: &str = "https://Vernier.SpacecraftSoftware.org/";

/// Maintainer line required in `--version` (CLI Standard §3).
pub const MAINTAINER: &str = "Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>";

/// Footer under every `--help`.
pub const FOOTER: &str = "Project: https://Vernier.SpacecraftSoftware.org/\nMaintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>";

/// Global flags, in the order `describe` lists them.
pub const GLOBAL_FLAGS: [&str; 15] = [
    "--json",
    "--format",
    "--fields",
    "--dry-run",
    "--verbose",
    "--quiet",
    "--no-color",
    "--color",
    "--help",
    "--version",
    "--absolute-time",
    "--print0",
    "--yes",
    "--force",
    "--backend",
];

/// Diagnostic codes `vnr` emits besides errors.
pub const DIAGNOSTICS: [(&str, &str, &str); 10] = [
    ("APPLIED", "ok", "a configuration was applied"),
    (
        "KEPT",
        "ok",
        "a configuration was confirmed on the terminal",
    ),
    ("WRITTEN", "ok", "a file was written"),
    ("UNCHANGED", "info", "nothing needed to change"),
    (
        "LAYOUT_WARNING",
        "warn",
        "the layout has a warning, such as an overlap",
    ),
    (
        "CONFIG_INVALID",
        "warn",
        "the configuration file could not be used; defaults apply",
    ),
    (
        "NOT_PERSISTENT",
        "warn",
        "the compositor forgets this change when it restarts",
    ),
    (
        "TUI_FALLBACK",
        "warn",
        "`--format explore` could not start; JSON was printed instead",
    ),
    (
        "TUI_UNAVAILABLE",
        "warn",
        "the explore viewer is not built yet; the table was printed",
    ),
    (
        "RESOLVED",
        "info",
        "which backend, theme or allowed paths were chosen",
    ),
];

/// One leaf command.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one published command property"
)]
pub struct Spec {
    /// Words after `vnr`, e.g. `output set`.
    pub path: &'static str,
    /// Example invocations with what they do; at least one uses `--json`.
    pub examples: &'static [(&'static str, &'static str)],
    /// Error codes the command can return.
    pub errors: &'static [ErrorCode],
    /// Prints a data payload and honors `--json`.
    pub supports_json: bool,
    /// Accepts `--dry-run` to preview changes.
    pub supports_dry_run: bool,
    /// Running it twice has the same effect as once.
    pub idempotent: bool,
    /// Can lose data or leave the user without a usable screen.
    pub destructive: bool,
    /// Lists records, so `--fields`, `jsonl`, `csv` and `--print0` apply per record.
    pub lists: bool,
    parameters: fn() -> Schema,
    data: fn() -> Schema,
}

const READ_ERRORS: &[ErrorCode] = &[
    ErrorCode::InvalidArgument,
    ErrorCode::BackendUnavailable,
    ErrorCode::Unsupported,
    ErrorCode::ProtocolError,
];

const CHANGE_ERRORS: &[ErrorCode] = &[
    ErrorCode::InvalidArgument,
    ErrorCode::MissingArgument,
    ErrorCode::NotFound,
    ErrorCode::PermissionDenied,
    ErrorCode::Conflict,
    ErrorCode::BackendUnavailable,
    ErrorCode::Unsupported,
    ErrorCode::Rejected,
    ErrorCode::ProtocolError,
    ErrorCode::Reverted,
    ErrorCode::InvalidLayout,
    ErrorCode::RevertFailed,
];

/// Every leaf command.
pub const SPECS: [Spec; 11] = [
    Spec {
        path: "gui",
        examples: &[
            ("vnr gui", "Open the editor on the current displays"),
            (
                "vnr gui --demo --theme steelbore-navywhite",
                "Try the editor on a sample layout in the light theme",
            ),
            (
                "vnr gui --demo --yes --json",
                "Open the editor from an agent session; it prints nothing",
            ),
        ],
        errors: &[
            ErrorCode::MissingArgument,
            ErrorCode::BackendUnavailable,
            ErrorCode::LibraryMissing,
        ],
        supports_json: false,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(GuiArgs),
        data: || schema_for!(()),
    },
    Spec {
        path: "output list",
        examples: &[
            ("vnr output list", "Show every output as a table"),
            ("vnr output list --json", "The same as JSON"),
            (
                "vnr output list --fields name,mode,x,y --format csv",
                "Names, modes and positions as CSV",
            ),
        ],
        errors: READ_ERRORS,
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: true,
        parameters: || schema_for!(NoArgs),
        data: || schema_for!(Vec<OutputRecord>),
    },
    Spec {
        path: "output get",
        examples: &[
            ("vnr output get eDP-1", "Show one output and its modes"),
            (
                "vnr output get eDP-1 --json --fields modes",
                "Only its modes, as JSON",
            ),
        ],
        errors: &[
            ErrorCode::InvalidArgument,
            ErrorCode::NotFound,
            ErrorCode::BackendUnavailable,
            ErrorCode::ProtocolError,
        ],
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(GetArgs),
        data: || schema_for!(OutputRecord),
    },
    Spec {
        path: "output set",
        examples: &[
            (
                "vnr output set DP-1 --mode 2560x1440@144 --right-of eDP-1 --scale 1.25",
                "Place a monitor right of the laptop panel",
            ),
            (
                "vnr output set HDMI-A-1 --mirror eDP-1 --dry-run --json",
                "Preview mirroring the laptop panel and test it with the compositor",
            ),
            (
                "vnr output set eDP-1 --off --yes",
                "Turn the panel off without the keep-or-revert prompt",
            ),
            (
                "vnr output set eDP-1 --scale 1.5 --persist ~/.config/niri/vernier.kdl",
                "Once kept, save the layout where niri reloads it after a restart",
            ),
        ],
        errors: CHANGE_ERRORS,
        supports_json: true,
        supports_dry_run: true,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(SetArgs),
        data: || schema_for!(ApplyReport),
    },
    Spec {
        path: "profile save",
        examples: &[
            (
                "vnr profile save desk.toml",
                "Save the current layout in the working directory",
            ),
            (
                "vnr profile save ~/.config/vernier/desk.toml --yes --json",
                "Overwrite a saved profile and report as JSON",
            ),
        ],
        errors: &[
            ErrorCode::InvalidArgument,
            ErrorCode::PermissionDenied,
            ErrorCode::Conflict,
            ErrorCode::BackendUnavailable,
            ErrorCode::ProtocolError,
        ],
        supports_json: true,
        supports_dry_run: true,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(SaveArgs),
        data: || schema_for!(WriteReport),
    },
    Spec {
        path: "profile apply",
        examples: &[
            (
                "vnr profile apply desk.toml",
                "Apply a profile, then keep or revert it",
            ),
            (
                "vnr profile apply desk.toml --dry-run --json",
                "List what would change and whether the compositor accepts it",
            ),
            (
                "vnr profile apply desk.toml --persist ~/.config/niri/vernier.kdl",
                "Apply a profile and, once kept, persist it for niri",
            ),
        ],
        errors: CHANGE_ERRORS,
        supports_json: true,
        supports_dry_run: true,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(ApplyArgs),
        data: || schema_for!(ApplyReport),
    },
    Spec {
        path: "layout export",
        examples: &[
            (
                "vnr layout export --to niri",
                "Print niri `output` blocks for the current layout",
            ),
            (
                "vnr layout export --to niri --write ~/.config/niri/vernier.kdl --json",
                "Write them to a file your niri config includes",
            ),
            (
                "vnr layout export --to sway --profile desk.toml",
                "Turn a saved profile into sway commands",
            ),
            (
                "vnr layout export --to kanshi --write ~/.config/kanshi/config",
                "Make a kanshi profile that reapplies this layout on hotplug",
            ),
            (
                "vnr layout export --to home-manager",
                "Print the same profile as home-manager `services.kanshi` settings",
            ),
        ],
        errors: &[
            ErrorCode::InvalidArgument,
            ErrorCode::NotFound,
            ErrorCode::PermissionDenied,
            ErrorCode::Conflict,
            ErrorCode::BackendUnavailable,
            ErrorCode::ProtocolError,
        ],
        supports_json: true,
        supports_dry_run: true,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(ExportArgs),
        data: || schema_for!(ExportRecord),
    },
    Spec {
        path: "backend get",
        examples: &[
            ("vnr backend get", "Show the backend and its capabilities"),
            ("vnr backend get --json", "The same as JSON"),
        ],
        errors: READ_ERRORS,
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(NoArgs),
        data: || schema_for!(BackendRecord),
    },
    Spec {
        path: "config check",
        examples: &[
            (
                "vnr config check",
                "Validate ~/.config/vernier/config.ncl and list the key bindings",
            ),
            (
                "vnr config check ./config.ncl --json",
                "Check a draft before installing it, as JSON",
            ),
        ],
        errors: &[
            ErrorCode::InvalidArgument,
            ErrorCode::NotFound,
            ErrorCode::PermissionDenied,
        ],
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(CheckArgs),
        data: || schema_for!(ConfigReport),
    },
    Spec {
        path: "schema",
        examples: &[
            ("vnr schema", "Every command's JSON Schema"),
            (
                "vnr schema output set --json",
                "The schema of one command, ready for LLM function calling",
            ),
        ],
        errors: &[ErrorCode::InvalidArgument, ErrorCode::NotFound],
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(SchemaArgs),
        data: || schema_for!(Value),
    },
    Spec {
        path: "describe",
        examples: &[
            ("vnr describe", "The capability manifest"),
            ("vnr describe --json", "The same, compact, for an agent"),
        ],
        errors: &[],
        supports_json: true,
        supports_dry_run: false,
        idempotent: true,
        destructive: false,
        lists: false,
        parameters: || schema_for!(NoArgs),
        data: || schema_for!(Value),
    },
];

/// Finds the spec for a command path.
#[must_use]
pub fn find(path: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.path == path)
}

/// The one-line description of a command, from its doc comment.
#[must_use]
pub fn description(path: &str) -> String {
    let mut command = Cli::command();
    for word in path.split_whitespace() {
        match command.find_subcommand(word) {
            Some(sub) => command = sub.clone(),
            None => return String::new(),
        }
    }
    command
        .get_about()
        .map(|about| about.to_string().trim_end_matches('.').to_owned())
        .unwrap_or_default()
}

/// The `--help` epilogue for a command: examples, then the footer.
#[must_use]
pub fn help_footer(spec: &Spec) -> String {
    let mut text = String::from("Examples:\n");
    for (command, what) in spec.examples {
        let _ = write!(text, "  {command}\n      {what}\n");
    }
    text.push('\n');
    text.push_str(FOOTER);
    text
}

/// The clap command tree with examples and footers attached.
#[must_use]
pub fn command() -> clap::Command {
    let mut root = Cli::command().after_help(format!(
        "Examples:\n  vnr output list\n      Show every output as a table\n  vnr describe --json\n      Discover every command, for an agent\n\n{FOOTER}"
    ));
    for spec in &SPECS {
        let words: Vec<&str> = spec.path.split_whitespace().collect();
        let footer = help_footer(spec);
        root = match words.as_slice() {
            [noun] => root.mut_subcommand(*noun, |c| c.after_help(footer)),
            [noun, verb] => {
                root.mut_subcommand(*noun, |c| c.mut_subcommand(*verb, |v| v.after_help(footer)))
            }
            _ => root,
        };
    }
    root
}

/// Removes the per-schema `$schema` and `title` keys so a sub-schema can be
/// embedded in a larger document.
fn embed(schema: Schema) -> Value {
    let mut value = schema.to_value();
    if let Value::Object(map) = &mut value {
        map.remove("$schema");
        map.remove("title");
    }
    value
}

fn global_property(name: &str) -> Option<Value> {
    embed(schema_for!(Global))
        .get("properties")
        .and_then(|p| p.get(name))
        .cloned()
}

/// The full schema document of one command.
#[must_use]
pub fn command_schema(spec: &Spec) -> Value {
    let mut parameters = embed((spec.parameters)());
    if let Some(Value::Object(properties)) = parameters.get_mut("properties") {
        if spec.supports_dry_run {
            if let Some(p) = global_property("dry_run") {
                properties.insert("dry_run".into(), p);
            }
            if let Some(p) = global_property("yes") {
                properties.insert("yes".into(), p);
            }
        }
        if let Some(p) = global_property("fields").filter(|_| spec.supports_json) {
            properties.insert("fields".into(), p);
        }
    }
    let mut data = embed((spec.data)());
    let mut output = embed(schema_for!(Response<()>));
    // `$ref`s resolve against the document root, so the payload's
    // definitions move up beside the envelope's.
    if let (Some(Value::Object(defs)), Value::Object(root)) = (
        data.as_object_mut().and_then(|d| d.remove("$defs")),
        &mut output,
    ) {
        let target = root
            .entry("$defs")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(target) = target {
            target.extend(defs);
        }
    }
    if let Some(Value::Object(properties)) = output.get_mut("properties") {
        properties.insert("data".into(), data);
    }
    let mut exit_codes = Map::new();
    exit_codes.insert("0".into(), "SUCCESS".into());
    for code in spec.errors {
        exit_codes.insert(
            code.exit_code().to_string(),
            format!("{}: {}", code.as_str(), code.meaning()).into(),
        );
    }
    let examples: Vec<Value> = spec
        .examples
        .iter()
        .map(|(command, what)| json!({"command": command, "description": what}))
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "command": format!("vnr {}", spec.path),
        "description": description(spec.path),
        "parameters": parameters,
        "output_schema": output,
        "exit_codes": exit_codes,
        "examples": examples,
        "supports_json": spec.supports_json,
        "supports_dry_run": spec.supports_dry_run,
        "idempotent": spec.idempotent,
        "destructive": spec.destructive,
        "lists": spec.lists,
    })
}

/// The schema document for every command.
#[must_use]
pub fn tool_schema() -> Value {
    let mut exit_codes = Map::new();
    exit_codes.insert("0".into(), "SUCCESS".into());
    for code in ErrorCode::ALL {
        exit_codes
            .entry(code.exit_code().to_string())
            .and_modify(|v| {
                if let Value::String(s) = v {
                    let _ = write!(s, "; {}: {}", code.as_str(), code.meaning());
                }
            })
            .or_insert_with(|| format!("{}: {}", code.as_str(), code.meaning()).into());
    }
    let diagnostics: Map<String, Value> = DIAGNOSTICS
        .iter()
        .map(|(code, severity, meaning)| {
            (
                (*code).to_owned(),
                json!({"severity": severity, "meaning": meaning}),
            )
        })
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "vnr",
        "description": "Arrange, rotate, scale and mirror displays on Wayland and X11",
        "commands": SPECS.iter().map(command_schema).collect::<Vec<_>>(),
        "exit_codes": exit_codes,
        "diagnostic_codes": diagnostics,
        "version_output": version_schema(),
        "jsonl": "one record per line, no metadata line",
        "csv": "header row of field names; metadata on stderr as one JSON line",
    })
}

/// The capability manifest.
#[must_use]
pub fn manifest() -> Value {
    let commands: Vec<Value> = SPECS
        .iter()
        .map(|spec| {
            json!({
                "name": format!("vnr {}", spec.path),
                "description": description(spec.path),
                "supports_json": spec.supports_json,
                "supports_dry_run": spec.supports_dry_run,
                "idempotent": spec.idempotent,
                "destructive": spec.destructive,
                "lists": spec.lists,
            })
        })
        .collect();
    json!({
        "tool": "vnr",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Arrange, rotate, scale and mirror displays on Wayland and X11",
        "license": "GPL-3.0-or-later",
        "homepage": WEBSITE,
        "repository": "https://github.com/Spacecraft-Software/Vernier",
        "maintainer": MAINTAINER,
        "commands": commands,
        "global_flags": GLOBAL_FLAGS,
        "output_formats": ["json", "jsonl", "yaml", "csv", "explore"],
        "backends": ["wlr", "cosmic", "kde", "mutter", "x11", "fixture"],
        "environment": {
            "VNR_BACKEND": "force a backend",
            "VNR_FIXTURE": "use a JSON file as the display server (tests)",
            "VNR_ALLOWED_PATHS": "colon-separated directories added to the path allow-list",
            "SPACECRAFT_THEME": "terminal and editor theme",
            "SPACECRAFT_A11Y": "accessible mode on (1) or off (0)",
        },
        "mcp_available": false,
        "schema_command": "vnr schema",
        "context_files": ["AGENTS.md", "CLAUDE.md", "SKILL.md", "CONTRIBUTING.md"],
    })
}

/// The schema of `--version --json` data, for completeness of `schema`.
#[must_use]
pub fn version_schema() -> Value {
    embed(schema_for!(VersionRecord))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_leaf_command_has_a_spec_and_examples() {
        fn leaves(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
            let subs: Vec<_> = cmd
                .get_subcommands()
                .filter(|c| c.get_name() != "help")
                .collect();
            if subs.is_empty() {
                out.push(prefix.trim().to_owned());
            }
            for sub in subs {
                leaves(sub, &format!("{prefix} {}", sub.get_name()), out);
            }
        }
        let mut paths = Vec::new();
        for sub in Cli::command().get_subcommands() {
            leaves(sub, sub.get_name(), &mut paths);
        }
        for path in &paths {
            let spec = find(path).unwrap_or_else(|| panic!("no spec for `{path}`"));
            assert!(spec.examples.len() >= 2, "`{path}` needs two examples");
            if spec.supports_json {
                assert!(
                    spec.examples.iter().any(|(c, _)| c.contains("--json")),
                    "`{path}` needs a --json example"
                );
            }
            assert!(!description(path).is_empty(), "`{path}` has no description");
        }
        assert_eq!(paths.len(), SPECS.len());
    }

    #[test]
    fn write_commands_accept_dry_run() {
        for spec in &SPECS {
            if [
                "output set",
                "profile save",
                "profile apply",
                "layout export",
            ]
            .contains(&spec.path)
            {
                assert!(spec.supports_dry_run, "{}", spec.path);
                let schema = command_schema(spec);
                assert!(schema["parameters"]["properties"]["dry_run"].is_object());
            }
        }
    }

    #[test]
    fn parameters_are_function_calling_ready() {
        for spec in &SPECS {
            let schema = command_schema(spec);
            assert_eq!(schema["parameters"]["type"], "object", "{}", spec.path);
            let text = schema["parameters"].to_string();
            assert!(!text.contains("http"), "no external refs in {}", spec.path);
        }
    }
}
