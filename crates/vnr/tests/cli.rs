// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI Standard v1.1 compliance tests (`testing-compliance.md`).
//!
//! Every test drives the real binary against the fixture backend
//! (`VNR_FIXTURE`), so no display server is needed. Each test gets its own
//! temporary directory as working directory and XDG home, which keeps the
//! path allow-list hermetic.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const FIXTURE: &str = r#"{"outputs":[
 {"name":"eDP-1","make":"BOE","enabled":true,
  "modes":[{"width":1920,"height":1080,"refresh_mhz":60000,"preferred":true}],
  "mode":{"width":1920,"height":1080,"refresh_mhz":60000,"preferred":true},
  "position":{"x":0,"y":0},"scale":1.0},
 {"name":"DP-1","enabled":true,
  "modes":[{"width":2560,"height":1440,"refresh_mhz":59951,"preferred":true},
           {"width":1920,"height":1080,"refresh_mhz":60000,"preferred":false}],
  "mode":{"width":2560,"height":1440,"refresh_mhz":59951,"preferred":true},
  "position":{"x":1920,"y":0},"scale":1.0}
]}"#;

/// An isolated home with a fixture display server.
struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self::with_fixture(FIXTURE)
    }

    fn with_fixture(fixture: &str) -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        fs::write(dir.path().join("fixture.json"), fixture).expect("write fixture");
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn fixture(&self) -> PathBuf {
        self.path().join("fixture.json")
    }

    /// `vnr` with a clean, non-agent environment and stdout piped.
    fn vnr(&self) -> Command {
        let mut cmd = Command::cargo_bin("vnr").expect("vnr binary");
        for var in [
            "AI_AGENT",
            "AGENT",
            "CI",
            "CLAUDECODE",
            "CURSOR_AGENT",
            "GEMINI_CLI",
            "CODEX_SANDBOX",
            "NO_COLOR",
            "FORCE_COLOR",
            "CLICOLOR",
            "VNR_BACKEND",
            "VNR_ALLOWED_PATHS",
            "SPACECRAFT_THEME",
            "SPACECRAFT_A11Y",
            "WAYLAND_DISPLAY",
            "DISPLAY",
            "XDG_RUNTIME_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd.current_dir(self.path())
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            .env("XDG_DATA_HOME", self.path().join("data"))
            .env("XDG_CACHE_HOME", self.path().join("cache"))
            .env("VNR_FIXTURE", self.fixture());
        cmd
    }

    fn run(&self, args: &[&str]) -> Outcome {
        Outcome::from(self.vnr().args(args).output().expect("run vnr"))
    }
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

impl From<std::process::Output> for Outcome {
    fn from(output: std::process::Output) -> Self {
        Self {
            code: output.status.code().expect("exit code"),
            stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
            stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        }
    }
}

impl Outcome {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }

    /// The single-line error object on stderr.
    fn error(&self) -> Value {
        let line = self
            .stderr
            .lines()
            .find(|l| l.starts_with("{\"error\""))
            .unwrap_or_else(|| panic!("no error object on stderr: {}", self.stderr));
        serde_json::from_str::<Value>(line).expect("error is JSON")["error"].clone()
    }

    fn diagnostics(&self) -> Vec<Value> {
        self.stderr
            .lines()
            .filter(|l| l.starts_with("{\"diagnostic\""))
            .map(|l| {
                serde_json::from_str::<Value>(l).expect("diagnostic is JSON")["diagnostic"].clone()
            })
            .collect()
    }
}

fn is_iso_utc(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z'
}

// --- Output modes and envelope -------------------------------------------

#[test]
fn piped_output_is_a_compact_envelope() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1, "compact JSON is one line");
    let json = out.json();
    assert_eq!(json["metadata"]["tool"], "vnr");
    assert_eq!(json["metadata"]["command"], "vnr output list");
    assert!(is_iso_utc(json["metadata"]["timestamp"].as_str().unwrap()));
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
    assert!(!out.stdout.contains('\u{1b}'), "no ANSI escapes in JSON");
    assert!(
        json["data"][1].get("make").is_none(),
        "null fields are omitted"
    );
}

#[test]
fn descriptive_ai_agent_value_selects_agent_mode() {
    let sandbox = Sandbox::new();
    let out = Outcome::from(
        sandbox
            .vnr()
            .env("AI_AGENT", "claude-code_2-1-218_agent")
            .args(["output", "set", "DP-1", "--scale", "1.25", "--yes"])
            .output()
            .unwrap(),
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    out.json();
    // Agent mode raises the floor to warn: no `ok` chatter.
    assert!(
        out.diagnostics().iter().all(|d| d["severity"] != "ok"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains('\u{1b}'));
}

#[test]
fn ci_falsy_values_do_not_force_machine_mode_on_their_own() {
    let sandbox = Sandbox::new();
    for value in ["0", "false"] {
        let out = Outcome::from(
            sandbox
                .vnr()
                .env("CI", value)
                .arg("--version")
                .output()
                .unwrap(),
        );
        // stdout is piped, so JSON regardless; the run must still succeed.
        assert_eq!(out.code, 0);
    }
}

#[test]
fn formats_jsonl_csv_yaml_and_print0() {
    let sandbox = Sandbox::new();

    let jsonl = sandbox.run(&["output", "list", "--format", "jsonl", "--fields", "name"]);
    assert_eq!(jsonl.stdout, "{\"name\":\"eDP-1\"}\n{\"name\":\"DP-1\"}\n");

    let csv = sandbox.run(&["output", "list", "--format", "csv", "--fields", "name,x,y"]);
    assert_eq!(csv.stdout, "name,x,y\neDP-1,0,0\nDP-1,1920,0\n");
    assert!(
        csv.stderr.contains("\"metadata\""),
        "csv metadata goes to stderr"
    );

    let yaml = sandbox.run(&[
        "output",
        "get",
        "DP-1",
        "--format",
        "yaml",
        "--fields",
        "name,width",
    ]);
    assert!(
        yaml.stdout.contains("metadata:") && yaml.stdout.contains("width: 2560"),
        "{}",
        yaml.stdout
    );

    let print0 = sandbox.run(&["output", "list", "-0"]);
    assert_eq!(print0.stdout, "eDP-1\0DP-1\0");
}

#[test]
fn fields_restricts_and_rejects_unknown_fields() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "get", "DP-1", "--fields", "name,scale", "--json"]);
    assert_eq!(
        out.json()["data"],
        serde_json::json!({"name": "DP-1", "scale": 1.0})
    );

    // An optional field absent from this answer is still a known field.
    let unchanged = sandbox.run(&[
        "output",
        "set",
        "DP-1",
        "--scale",
        "1",
        "--fields",
        "outcome,accepted_by_backend",
    ]);
    assert_eq!(unchanged.code, 0, "{}", unchanged.stderr);
    assert_eq!(
        unchanged.json()["data"],
        serde_json::json!({"outcome": "unchanged"})
    );

    let bad = sandbox.run(&["output", "list", "--fields", "nonsense"]);
    assert_eq!(bad.code, 2);
    assert_eq!(bad.error()["code"], "INVALID_ARGUMENT");
    assert!(bad.stdout.is_empty());
}

#[test]
fn explore_falls_back_to_json_when_piped() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "list", "--format", "explore"]);
    assert_eq!(out.code, 0);
    out.json();
    let diags = out.diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| d["code"] == "TUI_FALLBACK" && d["severity"] == "warn")
    );
}

#[test]
fn json_and_format_must_agree() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "list", "--json", "--format", "csv"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.error()["code"], "INVALID_ARGUMENT");
}

#[test]
fn quiet_and_verbose_conflict() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "list", "--quiet", "--verbose"]);
    assert_eq!(out.code, 2);
}

#[test]
fn verbose_reports_the_backend_and_quiet_silences_it() {
    let sandbox = Sandbox::new();
    let verbose = sandbox.run(&["output", "list", "--verbose"]);
    assert!(
        verbose
            .diagnostics()
            .iter()
            .any(|d| d["code"] == "RESOLVED")
    );
    let quiet = sandbox.run(&["output", "set", "DP-1", "--scale", "2", "--yes", "--quiet"]);
    assert_eq!(quiet.code, 0);
    assert!(quiet.stderr.is_empty(), "{}", quiet.stderr);
}

// --- Errors and exit codes ------------------------------------------------

#[test]
fn errors_are_single_line_json_with_runnable_hints() {
    let sandbox = Sandbox::new();
    let cases: &[(&[&str], i32, &str)] = &[
        (&["output", "list", "--bogus"], 2, "INVALID_ARGUMENT"),
        (&["output"], 2, "MISSING_ARGUMENT"),
        (&["output", "set", "DP-1"], 2, "MISSING_ARGUMENT"),
        (&["output", "get", "HDMI-A-9"], 3, "NOT_FOUND"),
        (
            &["output", "set", "DP-1", "--right-of", "nope"],
            3,
            "NOT_FOUND",
        ),
        (&["schema", "nothing"], 3, "NOT_FOUND"),
        (
            &["profile", "save", "/etc/vernier.toml"],
            4,
            "PERMISSION_DENIED",
        ),
        (
            &["--backend", "wlr", "output", "list"],
            10,
            "BACKEND_UNAVAILABLE",
        ),
        (
            &["--backend", "mutter", "output", "list"],
            11,
            "UNSUPPORTED",
        ),
    ];
    for (args, exit, code) in cases {
        let out = sandbox.run(args);
        assert_eq!(out.code, *exit, "{args:?}: {}", out.stderr);
        assert!(
            out.stdout.is_empty(),
            "{args:?}: stdout must be empty on error"
        );
        let error = out.error();
        assert_eq!(error["code"], *code, "{args:?}");
        assert_eq!(error["exit_code"], *exit);
        assert!(
            error["hint"].as_str().unwrap().starts_with("vnr ")
                || error["hint"].as_str().unwrap().starts_with("VNR_"),
            "{args:?}: hint `{}` is not a command",
            error["hint"]
        );
        assert!(is_iso_utc(error["timestamp"].as_str().unwrap()));
        assert!(error["command"].as_str().unwrap().starts_with("vnr"));
    }
}

#[test]
fn rejected_and_invalid_layouts_have_their_own_codes() {
    let rejecting = FIXTURE.replacen('{', "{\"reject\":true,", 1);
    let sandbox = Sandbox::with_fixture(&rejecting);
    let out = sandbox.run(&["output", "set", "DP-1", "--scale", "2", "--yes"]);
    assert_eq!(out.code, 12, "{}", out.stderr);
    assert_eq!(out.error()["code"], "REJECTED");

    let sandbox = Sandbox::new();
    let both_off = sandbox.run(&["output", "set", "eDP-1", "--off", "--yes"]);
    assert_eq!(both_off.code, 0, "{}", both_off.stderr);
    let none_left = sandbox.run(&["output", "set", "DP-1", "--off", "--yes"]);
    assert_eq!(none_left.code, 15, "{}", none_left.stderr);
    let error = none_left.error();
    assert_eq!(error["code"], "INVALID_LAYOUT");
    assert!(error["issues"].as_array().is_some_and(|i| !i.is_empty()));
}

#[test]
fn existing_files_need_yes() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.path().join("desk.toml"), "# mine\n").unwrap();
    let out = sandbox.run(&["profile", "save", "desk.toml"]);
    assert_eq!(out.code, 5);
    assert_eq!(out.error()["code"], "CONFLICT");
    assert_eq!(
        fs::read_to_string(sandbox.path().join("desk.toml")).unwrap(),
        "# mine\n"
    );
    assert_eq!(
        sandbox.run(&["profile", "save", "desk.toml", "--yes"]).code,
        0
    );
}

#[test]
fn no_command_under_an_agent_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let out = Outcome::from(sandbox.vnr().env("AGENT", "1").output().unwrap());
    assert_eq!(out.code, 2);
    assert_eq!(out.error()["hint"], "vnr describe --json");
    let gui = Outcome::from(sandbox.vnr().env("CI", "true").arg("gui").output().unwrap());
    assert_eq!(gui.code, 2);
}

// --- Input validation -----------------------------------------------------

#[test]
fn control_characters_and_oversized_arguments_are_rejected() {
    let sandbox = Sandbox::new();
    for bad in ["DP\u{1b}[31m", "DP\u{7f}", &"x".repeat(5000)] {
        let out = sandbox.run(&["output", "get", bad]);
        assert_eq!(out.code, 2, "{bad:?}");
        assert_eq!(out.error()["code"], "INVALID_ARGUMENT");
    }
    let long_name = sandbox.run(&["output", "get", &"x".repeat(65)]);
    assert_eq!(long_name.code, 2);
}

#[test]
fn path_traversal_cannot_escape_the_allow_list() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["profile", "save", "a/../../escape.toml"]);
    assert_eq!(out.code, 4);
    let error = out.error();
    assert!(error["allowed_paths"].is_array());
    assert!(
        !sandbox
            .path()
            .parent()
            .unwrap()
            .join("escape.toml")
            .exists()
    );

    let widened = Outcome::from(
        sandbox
            .vnr()
            .env("VNR_ALLOWED_PATHS", sandbox.path().parent().unwrap())
            .args(["profile", "save", "--dry-run", "a/../../escape.toml"])
            .output()
            .unwrap(),
    );
    assert_eq!(widened.code, 0, "{}", widened.stderr);
    assert_eq!(widened.json()["data"]["outcome"], "planned");
}

// --- Idempotency and dry runs ---------------------------------------------

#[test]
fn dry_run_changes_nothing_and_tests_with_the_backend() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["output", "set", "DP-1", "--left-of", "eDP-1", "--dry-run"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let json = out.json();
    assert_eq!(json["metadata"]["dry_run"], true);
    assert_eq!(json["data"]["outcome"], "planned");
    assert_eq!(json["data"]["accepted_by_backend"], true);
    assert_eq!(json["data"]["changes"].as_array().unwrap().len(), 2);
    assert_eq!(fs::read_to_string(sandbox.fixture()).unwrap(), FIXTURE);

    let save = sandbox.run(&["profile", "save", "plan.toml", "--dry-run"]);
    assert_eq!(save.json()["data"]["outcome"], "planned");
    assert!(!sandbox.path().join("plan.toml").exists());
}

#[test]
fn applying_twice_is_idempotent() {
    let sandbox = Sandbox::new();
    let args = ["output", "set", "DP-1", "--left-of", "eDP-1", "--yes"];
    let first = sandbox.run(&args);
    assert_eq!(first.json()["data"]["outcome"], "applied");
    assert!(first.diagnostics().iter().any(|d| d["code"] == "APPLIED"));
    let second = sandbox.run(&args);
    assert_eq!(second.code, 0);
    assert_eq!(second.json()["data"]["outcome"], "unchanged");

    assert_eq!(
        sandbox.run(&["profile", "save", "desk.toml"]).json()["data"]["outcome"],
        "written"
    );
    assert_eq!(
        sandbox.run(&["profile", "save", "desk.toml"]).json()["data"]["outcome"],
        "unchanged"
    );
    assert_eq!(
        sandbox.run(&["profile", "apply", "desk.toml"]).json()["data"]["outcome"],
        "unchanged"
    );
}

#[test]
fn profiles_round_trip() {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.run(&["profile", "save", "desk.toml"]).code, 0);
    assert_eq!(
        sandbox
            .run(&["output", "set", "DP-1", "--scale", "2", "--yes"])
            .code,
        0
    );
    let restore = sandbox.run(&["profile", "apply", "desk.toml"]);
    assert_eq!(restore.code, 0, "{}", restore.stderr);
    let data = &restore.json()["data"];
    assert_eq!(data["outcome"], "applied");
    let scale = data["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "scale")
        .unwrap();
    assert_eq!(scale["to"], "1");
}

#[test]
fn layout_export_prints_and_writes() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["layout", "export", "--to", "sway", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.json()["data"]["text"]
            .as_str()
            .unwrap()
            .contains("output \"DP-1\"")
    );

    let target = sandbox.path().join("config/niri/vernier.kdl");
    let write = sandbox.run(&[
        "layout",
        "export",
        "--to",
        "niri",
        "--write",
        target.to_str().unwrap(),
    ]);
    assert_eq!(write.code, 0, "{}", write.stderr);
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("output \"DP-1\"")
    );
}

// --- Introspection --------------------------------------------------------

#[test]
fn schema_is_valid_json_schema_and_describes_real_output() {
    let sandbox = Sandbox::new();
    let tool = sandbox.run(&["schema"]).json();
    let commands = tool["commands"].as_array().unwrap();
    assert!(commands.len() >= 10);
    for command in commands {
        let name = command["command"].as_str().unwrap();
        for part in ["parameters", "output_schema"] {
            jsonschema::meta::validate(&command[part])
                .unwrap_or_else(|e| panic!("`{name}` {part} is not valid JSON Schema: {e}"));
        }
        assert_eq!(
            command["parameters"]["type"], "object",
            "`{name}` parameters must be an object"
        );
        assert!(command["examples"].as_array().unwrap().len() >= 2);
    }

    // Real output validates against the published output schema.
    for args in [
        vec!["output", "list"],
        vec!["output", "get", "DP-1"],
        vec!["backend", "get"],
        vec!["output", "set", "DP-1", "--scale", "1.5", "--dry-run"],
        vec!["layout", "export", "--to", "niri"],
    ] {
        let path = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        let schema = sandbox.run(&["schema", args[0], args[1]]).json();
        let validator =
            jsonschema::validator_for(&schema["output_schema"]).expect("compile schema");
        let output = sandbox.run(&args).json();
        let errors: Vec<String> = validator
            .iter_errors(&output)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "`vnr {path}` output does not match its schema: {errors:?}"
        );
    }
}

#[test]
fn describe_lists_commands_and_flags() {
    let sandbox = Sandbox::new();
    let manifest = sandbox.run(&["describe"]).json();
    assert_eq!(manifest["tool"], "vnr");
    let names: Vec<&str> = manifest["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    for expected in [
        "vnr output list",
        "vnr output set",
        "vnr profile apply",
        "vnr schema",
        "vnr describe",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    let flags = manifest["global_flags"].as_array().unwrap();
    for flag in ["json", "format", "fields", "dry-run", "yes", "print0"] {
        assert!(
            flags.iter().any(|f| *f == format!("--{flag}")),
            "missing --{flag}"
        );
    }
}

#[test]
fn help_and_version_carry_examples_maintainer_and_website() {
    let sandbox = Sandbox::new();
    let help = sandbox.run(&["output", "set", "--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("Examples:") && help.stdout.contains("--json"));
    assert!(
        help.stdout
            .contains("https://Vernier.SpacecraftSoftware.org/")
    );
    assert!(help.stdout.contains("Maintained by Mohamed Hammad"));

    let version = sandbox.run(&["--version", "--json"]).json();
    assert_eq!(
        version["metadata"]["maintainer"],
        "Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    );
    assert_eq!(
        version["metadata"]["website"],
        "https://Vernier.SpacecraftSoftware.org/"
    );
}

// --- Encoding -------------------------------------------------------------

#[test]
fn output_is_utf8_without_bom() {
    let sandbox = Sandbox::new();
    for args in [
        &["output", "list"][..],
        &["describe"],
        &["output", "get", "nope"],
    ] {
        let raw = sandbox.vnr().args(args).output().unwrap();
        for stream in [&raw.stdout, &raw.stderr] {
            assert!(
                !stream.starts_with(b"\xEF\xBB\xBF"),
                "{args:?} starts with a BOM"
            );
            std::str::from_utf8(stream).expect("UTF-8");
        }
    }
}

#[test]
fn no_color_keeps_escapes_out_even_when_forced_elsewhere() {
    let sandbox = Sandbox::new();
    let out = Outcome::from(
        sandbox
            .vnr()
            .env("NO_COLOR", "1")
            .args(["output", "list", "--format", "explore"])
            .output()
            .unwrap(),
    );
    assert!(!out.stdout.contains('\u{1b}') && !out.stderr.contains('\u{1b}'));
}
