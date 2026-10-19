// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `Steelbore` theme as iced colors.
//!
//! Resolution and the palette itself live in `vnr-theme`; this module only
//! converts the resolved role tokens into iced types.

use iced::Color;

#[doc(inline)]
pub use vnr_theme::Request;

/// The eleven §11.1 role tokens as iced colors.
#[derive(Debug, Clone, PartialEq)]
pub struct Steelbore {
    /// Slug of the theme these tokens came from.
    pub slug: String,
    /// Canvas.
    pub background: Color,
    /// Raised surface (fill only, never text).
    pub surface: Color,
    /// Alternate surface (fill only, never text).
    pub surface_alt: Color,
    /// Body text.
    pub foreground: Color,
    /// Primary action.
    pub accent: Color,
    /// Structural lines and secondary emphasis.
    pub structure: Color,
    /// Success state.
    pub success: Color,
    /// Error state.
    pub error: Color,
    /// Warning state.
    pub warning: Color,
    /// Keyboard focus and selection.
    pub focus: Color,
    /// Borders.
    pub border: Color,
}

fn color(rgb: vnr_theme::Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

impl From<&vnr_theme::Steelbore> for Steelbore {
    fn from(tokens: &vnr_theme::Steelbore) -> Self {
        Self {
            slug: tokens.slug.clone(),
            background: color(tokens.background),
            surface: color(tokens.surface),
            surface_alt: color(tokens.surface_alt),
            foreground: color(tokens.foreground),
            accent: color(tokens.accent),
            structure: color(tokens.structure),
            success: color(tokens.success),
            error: color(tokens.error),
            warning: color(tokens.warning),
            focus: color(tokens.focus),
            border: color(tokens.border),
        }
    }
}

/// The resolved theme, as the editor uses it.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// The tokens to render with.
    pub tokens: Steelbore,
    detail: vnr_theme::Resolution,
}

impl Resolution {
    /// One line per decision, for `--verbose`.
    #[must_use]
    pub fn report(&self) -> String {
        self.detail.report()
    }
}

/// Resolves the theme from the command line and the environment.
#[must_use]
pub fn resolve(request: &Request) -> Resolution {
    let detail = vnr_theme::resolve(request);
    Resolution {
        tokens: Steelbore::from(&detail.tokens),
        detail,
    }
}

/// The iced theme that styles built-in widgets from the role tokens.
#[must_use]
pub fn iced_theme(tokens: &Steelbore) -> iced::Theme {
    iced::Theme::custom(
        tokens.slug.clone(),
        iced::theme::Palette {
            background: tokens.background,
            text: tokens.foreground,
            primary: tokens.accent,
            success: tokens.success,
            warning: tokens.warning,
            danger: tokens.error,
        },
    )
}
