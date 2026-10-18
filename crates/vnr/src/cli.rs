// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command-line grammar.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use vnr_core::export::Format;
use vnr_core::{AdaptiveSync, BackendKind, Mode, Position, Scale, Transform};

/// Attribution shown by `vnr --version` (Steelbore Standard §15).
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nMaintainer: Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>",
    "\nProject:    https://Vernier.SpacecraftSoftware.org/",
    "\nLicense:    GPL-3.0-or-later",
    "\nBuilt by Spacecraft Software"
);

/// Vernier: arrange, rotate, scale and mirror displays.
#[derive(Debug, Parser)]
#[command(name = "vnr", version, long_version = LONG_VERSION, about)]
pub struct Cli {
    /// Force a backend instead of detecting one.
    #[arg(long, global = true, env = vnr_backends::BACKEND_ENV, value_parser = parse::<BackendKind>)]
    pub backend: Option<BackendKind>,

    /// Report decisions such as the resolved theme and accessible mode.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// What to do; opens the editor when omitted.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the graphical editor (the default).
    Gui(GuiArgs),

    /// List connected outputs, their modes and settings.
    List {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Show which backend is in use and what it can do.
    Backend {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Save the current configuration as a profile (`-` for standard output).
    Save {
        /// Where to write the TOML profile.
        path: PathBuf,
        /// Overwrite an existing file.
        #[arg(short, long)]
        force: bool,
    },

    /// Apply a saved profile.
    Apply {
        /// The TOML profile to apply.
        path: PathBuf,
        #[command(flatten)]
        change: ChangeArgs,
    },

    /// Change one output, like wlr-randr or xrandr.
    Set(SetArgs),

    /// Print the current configuration as compositor config, to make it permanent.
    Export {
        /// Configuration language to write.
        #[arg(short, long, value_parser = parse::<Format>)]
        format: Format,
        /// Export this profile instead of the current state.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
}

/// Options for the graphical editor.
#[derive(Debug, Default, Args)]
pub struct GuiArgs {
    /// Theme slug (e.g. `steelbore`, `steelbore-navywhite`); overrides `SPACECRAFT_THEME`.
    #[arg(long)]
    pub theme: Option<String>,

    /// Turn accessible mode on (high-contrast theme, no animation).
    #[arg(long, overrides_with = "no_accessible")]
    pub accessible: bool,

    /// Turn accessible mode off, even if the environment asks for it.
    #[arg(long, overrides_with = "accessible")]
    pub no_accessible: bool,

    /// Edit a built-in sample layout without touching any display.
    #[arg(long)]
    pub demo: bool,
}

/// Options shared by every command that changes outputs.
#[derive(Debug, Args)]
pub struct ChangeArgs {
    /// Only ask the compositor whether it would accept the change.
    #[arg(long)]
    pub test: bool,

    /// Keep the change without asking.
    #[arg(short, long)]
    pub yes: bool,

    /// Seconds to wait for confirmation before reverting.
    ///
    /// The prompt appears only on a terminal; non-interactive runs keep
    /// the change.
    #[arg(long, value_name = "SECONDS", default_value_t = 15)]
    pub timeout: u64,
}

/// Changes to one output.
#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent command-line switch"
)]
pub struct SetArgs {
    /// Output connector name, e.g. `eDP-1`.
    pub output: String,

    /// Turn the output on.
    #[arg(long, conflicts_with = "off")]
    pub on: bool,

    /// Turn the output off.
    #[arg(long)]
    pub off: bool,

    /// Mode as `WIDTHxHEIGHT[@RATE[Hz]]`.
    #[arg(long, value_parser = parse::<Mode>)]
    pub mode: Option<Mode>,

    /// Use the display's preferred mode.
    #[arg(long, conflicts_with = "mode")]
    pub preferred: bool,

    /// Absolute position as X,Y in logical pixels.
    #[arg(long = "pos", value_parser = parse_position, allow_hyphen_values = true)]
    pub position: Option<Position>,

    /// Place left of another output, top edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative")]
    pub left_of: Option<String>,

    /// Place right of another output, top edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative")]
    pub right_of: Option<String>,

    /// Place above another output, left edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative")]
    pub above: Option<String>,

    /// Place below another output, left edges aligned.
    #[arg(long, value_name = "OUTPUT", group = "relative")]
    pub below: Option<String>,

    /// Scale factor, e.g. 1.5.
    #[arg(long, value_parser = parse::<Scale>)]
    pub scale: Option<Scale>,

    /// normal, 90, 180, 270, flipped, flipped-90, flipped-180 or flipped-270.
    #[arg(long, value_parser = parse::<Transform>)]
    pub transform: Option<Transform>,

    /// disabled, automatic or always.
    #[arg(long, value_parser = parse::<AdaptiveSync>)]
    pub adaptive_sync: Option<AdaptiveSync>,

    /// Mirror another output.
    #[arg(long, value_name = "OUTPUT", conflicts_with = "no_mirror")]
    pub mirror: Option<String>,

    /// Stop mirroring.
    #[arg(long)]
    pub no_mirror: bool,

    /// Make this the primary output.
    #[arg(long)]
    pub primary: bool,

    #[command(flatten)]
    pub change: ChangeArgs,
}

fn parse<T>(text: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    text.parse().map_err(|e: T::Err| e.to_string())
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
            "-y",
        ]);
        let Some(Command::Set(set)) = cli.command else {
            panic!("expected the set subcommand");
        };
        assert_eq!(set.mode, Some(Mode::new(2560, 1440, 144_000)));
        assert_eq!(set.position, Some(Position::new(-1920, 0)));
        assert_eq!(set.scale.map(Scale::millis), Some(1250));
        assert_eq!(set.transform, Some(Transform::Rotate90));
        assert!(set.change.yes);
    }
}
