// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier's graphical display-arrangement editor, built with iced.
//!
//! Outputs appear as rectangles on a canvas; drag them to arrange (edges
//! snap, hold Shift to place freely), and set mode, scale, rotation,
//! mirroring, adaptive sync and the primary output in the side panel.
//! Applying asks for confirmation and reverts after fifteen seconds
//! without an answer, so a configuration that blanks every screen fixes
//! itself.
//!
//! Component system (Steelbore Standard §13): Material Design, as iced is
//! custom-drawn, themed through the named `Steelbore` theme (§11.1).
//!
//! Backend calls run on short-lived worker threads; the UI thread only
//! edits the in-memory layout. Nothing here is CPU-bound, so there is no
//! further parallelism to exploit (§3.2).

// Rust guideline compliant 2026-05-18

mod app;
mod arrangement;
mod overlay;
pub mod theme;

use iced::{Font, Size};
use vnr_core::BackendKind;

/// Default body font (Steelbore Standard §12). Resolved through the system
/// font database; when it is not installed the platform default is used.
const BODY_FONT: Font = Font::with_name("Inconsolata");

/// How the editor should start.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Force a backend instead of detecting one.
    pub backend: Option<BackendKind>,
    /// Theme slug requested on the command line.
    pub theme: Option<String>,
    /// Accessible mode requested on the command line.
    pub accessible: Option<bool>,
    /// Edit a built-in sample layout instead of the real displays.
    pub demo: bool,
    /// Report theme and accessibility decisions on standard error.
    pub verbose: bool,
    /// Settings from the configuration file, or the defaults.
    pub config: vnr_config::Config,
    /// Why the configuration file was not used, shown in the status line.
    pub config_problem: Option<String>,
}

/// Opens the editor and blocks until its window closes.
///
/// # Errors
///
/// Returns an error if no window can be created (no graphical session, no
/// usable renderer).
pub fn run(options: Options) -> anyhow::Result<()> {
    iced::application(
        move || app::App::new(options.clone()),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .theme(app::App::theme)
    .subscription(app::App::subscription)
    .default_font(BODY_FONT)
    .window_size(Size::new(1100.0, 680.0))
    .run()
    .map_err(|e| anyhow::anyhow!("cannot open the Vernier window: {e}"))
}
