// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `Steelbore` theme (Steelbore Standard §11.1) and its resolution
//! (§11.6, §18.1), shared by Vernier's command line and editor.
//!
//! Color values are never typed into this crate: they are read from
//! `assets/steelbore.toml`, a verbatim copy of the palette skill's
//! canonical file (§11.4), embedded at build time. Callers refer to role
//! tokens only.
//!
//! # Examples
//!
//! ```
//! let resolution = vnr_theme::resolve(&vnr_theme::Request::default());
//! assert!(!resolution.tokens.slug.is_empty());
//! ```

// Rust guideline compliant 2026-05-18

use std::env;
use std::fs;
use std::path::PathBuf;

use toml::Table;

/// The canonical palette file, embedded at build time.
const PALETTE_TOML: &str = include_str!("../assets/steelbore.toml");

/// The project's default palette (§11.4 Modern).
const DEFAULT_THEME: &str = "steelbore";

/// The palette-independent monochrome theme (§11.1.1).
const MONO_THEME: &str = "steelbore-mono";

/// An sRGB color with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// Creates a color.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    const BLACK: Self = Self::new(0, 0, 0);
    const WHITE: Self = Self::new(0xFF, 0xFF, 0xFF);
}

/// The eleven §11.1 role tokens, resolved to colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steelbore {
    /// Slug of the theme these tokens came from.
    pub slug: String,
    /// Canvas.
    pub background: Rgb,
    /// Raised surface (fill only, never text).
    pub surface: Rgb,
    /// Alternate surface (fill only, never text).
    pub surface_alt: Rgb,
    /// Body text.
    pub foreground: Rgb,
    /// Primary action.
    pub accent: Rgb,
    /// Structural lines and secondary emphasis.
    pub structure: Rgb,
    /// Success state.
    pub success: Rgb,
    /// Error state.
    pub error: Rgb,
    /// Warning state.
    pub warning: Rgb,
    /// Keyboard focus and selection.
    pub focus: Rgb,
    /// Borders.
    pub border: Rgb,
}

/// Which theme was chosen and why, for `--verbose` (§11.6, §18.1).
#[derive(Debug, Clone)]
pub struct Resolution {
    /// The tokens to render with.
    pub tokens: Steelbore,
    /// The stage-1 palette slug.
    pub base: String,
    /// The stage-1 source that decided it.
    pub base_source: &'static str,
    /// The stage-2 overlay applied, if any.
    pub overlay: &'static str,
    /// Whether accessible mode is on.
    pub accessible: bool,
    /// The §18.1 source that decided accessible mode.
    pub accessible_source: &'static str,
}

impl Steelbore {
    /// Whether this is `steelbore-mono`, which surrenders color: a terminal
    /// renders it with no color escapes at all.
    #[must_use]
    pub fn is_mono(&self) -> bool {
        self.slug == MONO_THEME
    }
}

impl Resolution {
    /// One line per decision, for `--verbose`.
    #[must_use]
    pub fn report(&self) -> String {
        format!(
            "theme: {} (palette {} from {}; variant: {})\naccessible mode: {} (from {})",
            self.tokens.slug,
            self.base,
            self.base_source,
            self.overlay,
            if self.accessible { "on" } else { "off" },
            self.accessible_source,
        )
    }
}

/// Inputs to theme resolution that come from the command line.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// `--theme`.
    pub theme: Option<String>,
    /// `--accessible` / `--no-accessible`.
    pub accessible: Option<bool>,
}

/// The parts of the environment resolution reads, separated so tests can
/// supply them.
#[derive(Debug, Clone, Default)]
struct Environment {
    spacecraft_theme: Option<String>,
    spacecraft_a11y: Option<String>,
    no_color: bool,
    term_dumb: bool,
    gtk_atk: bool,
    declared_active: Option<String>,
    declared_high_contrast: Option<bool>,
}

impl Environment {
    fn read() -> Self {
        let var = |name| env::var(name).ok().filter(|v| !v.is_empty());
        let (declared_active, declared_high_contrast) = read_declaration();
        Self {
            spacecraft_theme: var("SPACECRAFT_THEME"),
            spacecraft_a11y: var("SPACECRAFT_A11Y"),
            no_color: var("NO_COLOR").is_some(),
            term_dumb: var("TERM").as_deref() == Some("dumb"),
            gtk_atk: var("GTK_MODULES").is_some_and(|m| m.contains("gail:atk")),
            declared_active,
            declared_high_contrast,
        }
    }
}

/// Reads `active` and `high-contrast` from the §11.6.4 declaration files,
/// each key from the highest-precedence file that supplies it.
fn read_declaration() -> (Option<String>, Option<bool>) {
    let user = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("steelbore/theme.toml"));
    let system = Some(PathBuf::from("/etc/steelbore/theme.toml"));
    let tables: Vec<Table> = [user, system]
        .into_iter()
        .flatten()
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| text.parse::<Table>().ok())
        .collect();
    let key = |name: &str| {
        tables
            .iter()
            .find_map(|t| t.get("theme").and_then(|theme| theme.get(name)).cloned())
    };
    (
        key("active").and_then(|v| v.as_str().map(str::to_owned)),
        key("high-contrast").and_then(|v| v.as_bool()),
    )
}

/// The parsed palette file.
#[derive(Debug)]
struct Palette {
    table: Table,
}

impl Palette {
    fn load() -> Self {
        // The file is embedded and covered by tests; failing to parse it is
        // a build defect, not a run-time condition.
        let table = PALETTE_TOML
            .parse::<Table>()
            .expect("embedded steelbore.toml is valid TOML");
        Self { table }
    }

    fn meta_list(&self, key: &str) -> Vec<&str> {
        self.table
            .get("meta")
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(|i| i.as_str()).collect())
            .unwrap_or_default()
    }

    /// Whether `slug` may be adopted (registered, not a fidelity palette).
    fn is_registered(&self, slug: &str) -> bool {
        self.meta_list("registered-set").contains(&slug)
    }

    fn is_fidelity(&self, slug: &str) -> bool {
        self.meta_list("fidelity-palettes").contains(&slug)
    }

    fn tokens(&self, slug: &str) -> Option<Steelbore> {
        if slug == MONO_THEME {
            return Some(mono());
        }
        let theme = self.table.get("themes")?.get(slug)?;
        let token = |name: &str| theme.get(name)?.as_str().and_then(parse_hex);
        Some(Steelbore {
            slug: slug.to_owned(),
            background: token("background")?,
            surface: token("surface")?,
            surface_alt: token("surface-alt")?,
            foreground: token("foreground")?,
            accent: token("accent")?,
            structure: token("structure")?,
            success: token("success")?,
            error: token("error")?,
            warning: token("warning")?,
            focus: token("focus")?,
            border: token("border")?,
        })
    }
}

fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let [_, r, g, b] = value.to_be_bytes();
    Some(Rgb::new(r, g, b))
}

/// Renders `steelbore-mono` where a concrete color is unavoidable (a GUI).
///
/// The mono theme names terminal attributes (`default`, `bright-white`,
/// `reverse-video`), not colors, because it exists to surrender color. A
/// GUI has no terminal default, so it renders pure black, white and two
/// grays: every text pairing is white or black on black or white (21:1),
/// and the grays are used only for fills and lines.
fn mono() -> Steelbore {
    Steelbore {
        slug: MONO_THEME.to_owned(),
        background: Rgb::BLACK,
        surface: Rgb::new(0x26, 0x26, 0x26),
        surface_alt: Rgb::new(0x12, 0x12, 0x12),
        foreground: Rgb::WHITE,
        accent: Rgb::WHITE,
        structure: Rgb::new(0xB0, 0xB0, 0xB0),
        success: Rgb::WHITE,
        error: Rgb::WHITE,
        warning: Rgb::WHITE,
        focus: Rgb::WHITE,
        border: Rgb::new(0xB0, 0xB0, 0xB0),
    }
}

/// Resolves the theme from the command line and the environment.
#[must_use]
pub fn resolve(request: &Request) -> Resolution {
    resolve_with(request, &Environment::read(), &Palette::load())
}

fn resolve_with(request: &Request, env: &Environment, palette: &Palette) -> Resolution {
    // §18.1: flag, then environment, then (no project config yet) hints.
    let (accessible, accessible_source) = match (request.accessible, env.spacecraft_a11y.as_deref())
    {
        (Some(on), _) => (on, "command line"),
        (None, Some("1")) => (true, "SPACECRAFT_A11Y"),
        (None, Some("0")) => (false, "SPACECRAFT_A11Y"),
        _ if env.term_dumb => (true, "TERM=dumb"),
        _ if env.no_color => (true, "NO_COLOR"),
        _ if env.gtk_atk => (true, "GTK_MODULES"),
        _ => (false, "default"),
    };

    // §11.6 stage 1. Unusable slugs fall through; fidelity palettes are
    // honored only when asked for explicitly on the command line.
    let usable = |slug: &str| palette.is_registered(slug);
    let (base, base_source) = if let Some(slug) = request
        .theme
        .as_deref()
        .filter(|s| usable(s) || palette.is_fidelity(s))
    {
        (slug.to_owned(), "--theme")
    } else if let Some(slug) = env.spacecraft_theme.as_deref().filter(|s| usable(s)) {
        (slug.to_owned(), "SPACECRAFT_THEME")
    } else if let Some(slug) = env.declared_active.as_deref().filter(|s| usable(s)) {
        (slug.to_owned(), "steelbore/theme.toml")
    } else {
        (DEFAULT_THEME.to_owned(), "project default")
    };

    // §11.6 stage 2.
    let pinned = base.ends_with("-high-contrast") || base == MONO_THEME;
    let high_contrast = format!("{base}-high-contrast");
    let (slug, overlay) = if pinned && base_source != "steelbore/theme.toml" {
        (base.clone(), "pinned")
    } else if env.no_color {
        (MONO_THEME.to_owned(), "NO_COLOR")
    } else if accessible && palette.is_registered(&high_contrast) {
        (high_contrast, "accessible mode")
    } else if env.declared_high_contrast == Some(true) && palette.is_registered(&high_contrast) {
        (high_contrast, "declared high contrast")
    } else {
        (base.clone(), "none")
    };

    let tokens = palette
        .tokens(&slug)
        .or_else(|| palette.tokens(DEFAULT_THEME))
        .expect("the default theme is present in steelbore.toml");
    Resolution {
        tokens,
        base,
        base_source,
        overlay,
        accessible,
        accessible_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_theme_has_tokens() {
        let palette = Palette::load();
        let registered = palette.meta_list("registered-set");
        assert_eq!(registered.len(), 21, "§11.6.1 registers twenty-one themes");
        for slug in registered {
            assert!(palette.tokens(slug).is_some(), "{slug} lacks tokens");
        }
    }

    #[test]
    fn default_is_steelbore_modern() {
        let res = resolve_with(
            &Request::default(),
            &Environment::default(),
            &Palette::load(),
        );
        assert_eq!(res.tokens.slug, "steelbore");
        assert_eq!(res.base_source, "project default");
        assert!(!res.accessible);
    }

    #[test]
    fn precedence_and_overlays() {
        let palette = Palette::load();
        let env = Environment {
            spacecraft_theme: Some("steelbore-blue".into()),
            declared_active: Some("steelbore-green".into()),
            ..Environment::default()
        };
        let res = resolve_with(&Request::default(), &env, &palette);
        assert_eq!(
            (res.base.as_str(), res.base_source),
            ("steelbore-blue", "SPACECRAFT_THEME")
        );

        let request = Request {
            theme: Some("not-a-theme".into()),
            accessible: Some(true),
        };
        let res = resolve_with(&request, &env, &palette);
        assert_eq!(res.tokens.slug, "steelbore-blue-high-contrast");

        let no_color = Environment {
            no_color: true,
            ..env
        };
        assert_eq!(
            resolve_with(&request, &no_color, &palette).tokens.slug,
            MONO_THEME
        );
    }

    #[test]
    fn explicit_no_accessible_beats_hints() {
        let env = Environment {
            term_dumb: true,
            ..Environment::default()
        };
        let request = Request {
            accessible: Some(false),
            ..Request::default()
        };
        assert!(!resolve_with(&request, &env, &Palette::load()).accessible);
        assert!(resolve_with(&Request::default(), &env, &Palette::load()).accessible);
    }
}
