// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command-line grammar: `vnr [global flags] <noun> <verb> [args]`.
//!
//! Every argument struct also derives `JsonSchema`, and their doc comments
//! are both the `--help` text and the schema descriptions, so `vnr schema`
//! cannot drift from what the parser accepts (CLI Standard
//! `schema-introspection.md` §5).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use schemars::JsonSchema;
use vnr_core::export::Format as ExportFormat;
use vnr_core::{AdaptiveSync, BackendKind, Mode, Position, Scale, Transform};

use crate::output::{ColorChoice, Format};
use crate::validate;

/// Arrange, rotate, scale and mirror displays on Wayland and X11.
#[derive(Debug, Parser)]
#[command(name = "vnr", disable_version_flag = true, max_term_width = 100)]
pub struct Cli {
    /// Flags accepted by every command.
    #[command(flatten)]
    pub global: Global,

    /// What to do. Without one, `vnr` opens the editor.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Flags shared by every Spacecraft Software CLI (CLI Standard §3).
#[derive(Debug, Default, Args, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one standard command-line switch"
)]
pub struct Global {
    /// Same as `--format json`.
    #[arg(long, global = true)]
    #[schemars(default)]
    pub json: bool,

    /// Output format; defaults to a table on a terminal and JSON otherwise.
    #[arg(long, global = true, value_enum)]
    #[schemars(skip)]
    pub format: Option<Format>,

    /// Only these fields of each record, comma-separated.
    #[arg(long, global = true, value_delimiter = ',', value_name = "FIELD,...")]
    pub fields: Option<Vec<String>>,

    /// Show what would change, including whether the display server accepts it, without changing anything.
    #[arg(long, global = true)]
    #[schemars(default)]
    pub dry_run: bool,

    /// Also print informational diagnostics.
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    #[schemars(skip)]
    pub verbose: bool,

    /// Print errors only.
    #[arg(short, long, global = true)]
    #[schemars(skip)]
    pub quiet: bool,

    /// Same as `--color never`.
    #[arg(long, global = true)]
    #[schemars(skip)]
    pub no_color: bool,

    /// When to color human output.
    #[arg(long, global = true, value_enum, value_name = "WHEN")]
    #[schemars(skip)]
    pub color: Option<ColorChoice>,

    /// Accepted for compatibility; Vernier never shows relative times.
    #[arg(long, global = true)]
    #[schemars(skip)]
    pub absolute_time: bool,

    /// Print the first field of each record, each followed by a NUL byte.
    #[arg(short = '0', long, global = true)]
    #[schemars(skip)]
    pub print0: bool,

    /// Keep changes without asking, overwrite existing files, and allow the editor under an agent.
    #[arg(short, long, global = true, visible_alias = "force")]
    #[schemars(default)]
    pub yes: bool,

    /// Use this backend instead of detecting one.
    #[arg(long, global = true, env = vnr_backends::BACKEND_ENV, value_parser = parse::<BackendKind>)]
    #[schemars(skip)]
    pub backend: Option<BackendKind>,

    /// Print the version, maintainer and project page.
    #[arg(short = 'V', long, global = true)]
    #[schemars(skip)]
    pub version: bool,
}

/// Nouns and top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the graphical editor.
    Gui(GuiArgs),
    /// Inspect and change displays.
    Output {
        /// Verb.
        #[command(subcommand)]
        verb: OutputVerb,
    },
    /// Save and apply display profiles.
    Profile {
        /// Verb.
        #[command(subcommand)]
        verb: ProfileVerb,
    },
    /// Export the layout as compositor configuration.
    Layout {
        /// Verb.
        #[command(subcommand)]
        verb: LayoutVerb,
    },
    /// Inspect the display backend.
    Backend {
        /// Verb.
        #[command(subcommand)]
        verb: BackendVerb,
    },
    /// Check the configuration file.
    Config {
        /// Verb.
        #[command(subcommand)]
        verb: ConfigVerb,
    },
    /// Print the JSON Schema of every command, a noun, or one command.
    Schema(SchemaArgs),
    /// Print the capability manifest: commands, flags and formats.
    Describe(NoArgs),
}

/// Verbs of `vnr output`.
#[derive(Debug, Subcommand)]
pub enum OutputVerb {
    /// List connected outputs and their settings.
    List(NoArgs),
    /// Show one output, including every mode it supports.
    Get(GetArgs),
    /// Change one output: enable, mode, position, scale, rotation, mirroring.
    Set(SetArgs),
}

/// Verbs of `vnr profile`.
#[derive(Debug, Subcommand)]
pub enum ProfileVerb {
    /// Save the current configuration as a TOML profile.
    Save(SaveArgs),
    /// Apply a saved TOML profile.
    Apply(ApplyArgs),
}

/// Verbs of `vnr layout`.
#[derive(Debug, Subcommand)]
pub enum LayoutVerb {
    /// Print or write the layout as niri, sway, kanshi, shikane or home-manager configuration.
    Export(ExportArgs),
}

/// Verbs of `vnr backend`.
#[derive(Debug, Subcommand)]
pub enum BackendVerb {
    /// Show which backend is in use and what it can do.
    Get(NoArgs),
}

/// Verbs of `vnr config`.
#[derive(Debug, Subcommand)]
pub enum ConfigVerb {
    /// Validate the configuration file and show the key bindings in effect.
    Check(CheckArgs),
}

/// Arguments of `vnr config check`.
#[derive(Debug, Default, Args, JsonSchema)]
pub struct CheckArgs {
    /// The file to check; default `~/.config/vernier/config.ncl`.
    #[schemars(with = "Option<String>", length(min = 1, max = 4096))]
    pub path: Option<PathBuf>,
}

/// A command without arguments of its own.
#[derive(Debug, Default, Args, JsonSchema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "braces make the schema an empty object, which tool-calling APIs require"
)]
pub struct NoArgs {}

/// Arguments of `vnr gui`.
#[derive(Debug, Default, Args, JsonSchema)]
pub struct GuiArgs {
    /// Theme slug, e.g. `steelbore` or `steelbore-navywhite`; overrides `SPACECRAFT_THEME`.
    #[arg(long, value_parser = bounded::<64>)]
    pub theme: Option<String>,

    /// Turn accessible mode on: high-contrast theme, no animation.
    #[arg(long, overrides_with = "no_accessible")]
    #[schemars(default)]
    pub accessible: bool,

    /// Turn accessible mode off, even if the environment asks for it.
    #[arg(long, overrides_with = "accessible")]
    #[schemars(default)]
    pub no_accessible: bool,

    /// Edit a built-in sample layout without touching any display.
    #[arg(long)]
    #[schemars(default)]
    pub demo: bool,
}

/// Arguments of `vnr output get`.
#[derive(Debug, Args, JsonSchema)]
pub struct GetArgs {
    /// Output connector name, e.g. `eDP-1`.
    #[arg(value_parser = validate::output_name)]
    #[schemars(length(min = 1, max = 64))]
    pub output: String,
}

/// Rotation names, for the schema.
#[derive(Debug, JsonSchema)]
#[expect(dead_code, reason = "schema-only mirror of vnr_core::Transform")]
enum TransformName {
    #[schemars(rename = "normal")]
    Normal,
    #[schemars(rename = "90")]
    R90,
    #[schemars(rename = "180")]
    R180,
    #[schemars(rename = "270")]
    R270,
    #[schemars(rename = "flipped")]
    Flipped,
    #[schemars(rename = "flipped-90")]
    F90,
    #[schemars(rename = "flipped-180")]
    F180,
    #[schemars(rename = "flipped-270")]
    F270,
}

/// Adaptive sync names, for the schema.
#[derive(Debug, JsonSchema)]
#[schemars(rename_all = "lowercase")]
#[expect(dead_code, reason = "schema-only mirror of vnr_core::AdaptiveSync")]
enum AdaptiveSyncName {
    Disabled,
    Automatic,
    Always,
}

/// Arguments of `vnr output set`.
#[derive(Debug, Args, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent command-line switch"
)]
pub struct SetArgs {
    /// Output connector name, e.g. `eDP-1`.
    #[arg(value_parser = validate::output_name)]
    #[schemars(length(min = 1, max = 64))]
    pub output: String,

    /// Turn the output on.
    #[arg(long, conflicts_with = "off")]
    #[schemars(default)]
    pub on: bool,

    /// Turn the output off.
    #[arg(long)]
    #[schemars(default)]
    pub off: bool,

    /// Mode as `WIDTHxHEIGHT` or `WIDTHxHEIGHT@RATE`, rate in Hz, e.g. `2560x1440@143.973`.
    #[arg(long, value_parser = parse::<Mode>)]
    #[schemars(with = "Option<String>")]
    pub mode: Option<Mode>,

    /// Use the display's preferred mode.
    #[arg(long, conflicts_with = "mode")]
    #[schemars(default)]
    pub preferred: bool,

    /// Absolute position as `X,Y` in logical pixels, e.g. `-1920,0`.
    #[arg(long = "pos", value_parser = parse_position, allow_hyphen_values = true)]
    #[schemars(with = "Option<String>", rename = "pos")]
    pub position: Option<Position>,

    /// Place left of this output, top edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative", value_parser = validate::output_name)]
    pub left_of: Option<String>,

    /// Place right of this output, top edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative", value_parser = validate::output_name)]
    pub right_of: Option<String>,

    /// Place above this output, left edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative", value_parser = validate::output_name)]
    pub above: Option<String>,

    /// Place below this output, left edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative", value_parser = validate::output_name)]
    pub below: Option<String>,

    /// Scale factor, e.g. `1.5`, at most three decimals.
    #[arg(long, value_parser = parse::<Scale>)]
    #[schemars(with = "Option<f64>", range(min = 0.25, max = 10.0))]
    pub scale: Option<Scale>,

    /// Rotation and reflection.
    #[arg(long, value_parser = parse::<Transform>)]
    #[schemars(with = "Option<TransformName>")]
    pub transform: Option<Transform>,

    /// Variable refresh rate.
    #[arg(long, value_parser = parse::<AdaptiveSync>)]
    #[schemars(with = "Option<AdaptiveSyncName>")]
    pub adaptive_sync: Option<AdaptiveSync>,

    /// Mirror this output instead of extending the desktop.
    #[arg(long, value_name = "OUTPUT", conflicts_with = "no_mirror", value_parser = validate::output_name)]
    pub mirror: Option<String>,

    /// Stop mirroring.
    #[arg(long)]
    #[schemars(default)]
    pub no_mirror: bool,

    /// Make this the primary output.
    #[arg(long)]
    #[schemars(default)]
    pub primary: bool,

    /// Seconds to wait for confirmation on a terminal before reverting.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=600))]
    #[schemars(default = "default_timeout", range(min = 1, max = 600))]
    pub timeout: u64,
    /// Once the change is kept, write niri `output` blocks for the whole layout
    /// to this file, which niri's config includes; never niri's `config.kdl`.
    #[arg(long, value_name = "FILE")]
    #[schemars(with = "Option<String>", length(min = 1, max = 4096))]
    pub persist: Option<PathBuf>,
    /// Have GNOME save the change so it outlasts the session: GNOME shows its
    /// own keep-or-revert dialog in place of Vernier's prompt, and restores the
    /// previous configuration unless someone answers Keep within 20 seconds.
    #[arg(long, conflicts_with_all = ["persist", "timeout"])]
    #[schemars(default)]
    pub save: bool,
}

/// Arguments of `vnr profile save`.
#[derive(Debug, Args, JsonSchema)]
pub struct SaveArgs {
    /// Where to write the profile, inside the working directory or `~/.config/vernier`.
    #[schemars(with = "String", length(min = 1, max = 4096))]
    pub path: PathBuf,
}

/// Arguments of `vnr profile apply`.
#[derive(Debug, Args, JsonSchema)]
pub struct ApplyArgs {
    /// The profile to apply.
    #[schemars(with = "String", length(min = 1, max = 4096))]
    pub path: PathBuf,

    /// Seconds to wait for confirmation on a terminal before reverting.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=600))]
    #[schemars(default = "default_timeout", range(min = 1, max = 600))]
    pub timeout: u64,
    /// Once the change is kept, write niri `output` blocks for the whole layout
    /// to this file, which niri's config includes; never niri's `config.kdl`.
    #[arg(long, value_name = "FILE")]
    #[schemars(with = "Option<String>", length(min = 1, max = 4096))]
    pub persist: Option<PathBuf>,
    /// Have GNOME save the change so it outlasts the session: GNOME shows its
    /// own keep-or-revert dialog in place of Vernier's prompt, and restores the
    /// previous configuration unless someone answers Keep within 20 seconds.
    #[arg(long, conflicts_with_all = ["persist", "timeout"])]
    #[schemars(default)]
    pub save: bool,
}

/// Configuration languages, for the schema.
#[derive(Debug, JsonSchema)]
#[schemars(rename_all = "lowercase")]
#[expect(dead_code, reason = "schema-only mirror of vnr_core::export::Format")]
enum ExportName {
    Niri,
    Sway,
    Kanshi,
    Shikane,
    #[schemars(rename = "home-manager")]
    HomeManager,
}

/// Arguments of `vnr layout export`.
#[derive(Debug, Args, JsonSchema)]
pub struct ExportArgs {
    /// Configuration language to produce.
    #[arg(long, value_parser = parse::<ExportFormat>)]
    #[schemars(with = "ExportName")]
    pub to: ExportFormat,

    /// Export this profile instead of the current state.
    #[arg(long)]
    #[schemars(with = "Option<String>", length(min = 1, max = 4096))]
    pub profile: Option<PathBuf>,

    /// Write to this file, e.g. one your niri config includes, instead of printing.
    #[arg(long)]
    #[schemars(with = "Option<String>", length(min = 1, max = 4096))]
    pub write: Option<PathBuf>,
}

/// Arguments of `vnr schema`.
#[derive(Debug, Default, Args, JsonSchema)]
pub struct SchemaArgs {
    /// Limit to one noun, e.g. `output`.
    #[arg(value_parser = bounded::<32>)]
    pub noun: Option<String>,

    /// Limit to one verb of that noun, e.g. `set`.
    #[arg(value_parser = bounded::<32>)]
    pub verb: Option<String>,
}

const fn default_timeout() -> u64 {
    15
}

fn parse<T>(text: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    text.parse().map_err(|e: T::Err| e.to_string())
}

fn bounded<const MAX: usize>(text: &str) -> Result<String, String> {
    if text.is_empty() || text.len() > MAX {
        Err(format!("must be 1 to {MAX} bytes"))
    } else {
        Ok(text.to_owned())
    }
}

fn parse_position(text: &str) -> Result<Position, String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("invalid position {text:?}: expected X,Y"))?;
    let coordinate = |value: &str| {
        value
            .trim()
            .parse::<i32>()
            .map_err(|e| format!("invalid position {text:?}: {e}"))
    };
    Ok(Position::new(coordinate(x)?, coordinate(y)?))
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn grammar_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn set_parses_wlr_randr_style_arguments() {
        let cli = Cli::parse_from([
            "vnr",
            "output",
            "set",
            "DP-1",
            "--mode",
            "2560x1440@144",
            "--pos",
            "-1920,0",
            "--scale",
            "1.25",
            "--transform",
            "90",
            "--yes",
        ]);
        let Some(Command::Output {
            verb: OutputVerb::Set(set),
        }) = cli.command
        else {
            panic!("expected output set");
        };
        assert_eq!(set.mode, Some(Mode::new(2560, 1440, 144_000)));
        assert_eq!(set.position, Some(Position::new(-1920, 0)));
        assert_eq!(set.scale.map(Scale::millis), Some(1250));
        assert_eq!(set.transform, Some(Transform::Rotate90));
        assert!(cli.global.yes);
    }

    #[test]
    fn global_flags_work_after_the_verb() {
        let cli = Cli::parse_from(["vnr", "output", "list", "--json", "--fields", "name,mode"]);
        assert!(cli.global.json);
        assert_eq!(
            cli.global.fields,
            Some(vec!["name".to_owned(), "mode".to_owned()])
        );
    }
}
