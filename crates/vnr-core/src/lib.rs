// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backend-agnostic core of Vernier.
//!
//! Every display backend (wlr-output-management, COSMIC, KDE, Mutter, X11)
//! translates its native state into the [`Layout`] model defined here, and
//! translates a desired [`Layout`] back into native requests. Everything that
//! does not need a display server lives in this crate, so it can be tested
//! without one:
//!
//! - [`model`]: outputs, modes, transforms, scale, mirroring.
//! - [`geometry`]: logical rectangles, edge snapping, normalization.
//! - [`validate()`]: problems a layout has before it is sent anywhere.
//! - [`Profile`]: the TOML profile format used by `vnr save` / `vnr apply`.
//! - [`export`]: niri and sway configuration snippets.
//! - [`Backend`]: the trait every backend crate implements.
//!
//! # Examples
//!
//! ```
//! use vnr_core::{Layout, Mode, Output, Position};
//!
//! let mut laptop = Output::new("eDP-1");
//! laptop.modes.push(Mode::new(1920, 1080, 60_000));
//! laptop.mode = laptop.modes.first().copied();
//!
//! let mut external = Output::new("HDMI-A-1");
//! external.modes.push(Mode::new(2560, 1440, 60_000));
//! external.mode = external.modes.first().copied();
//! external.position = Position::new(1920, 0);
//!
//! let layout = Layout::new(vec![laptop, external]);
//! assert!(vnr_core::validate(&layout, &vnr_core::Capabilities::default()).is_empty());
//! ```

// Rust guideline compliant 2026-05-18

pub mod export;
pub mod geometry;
pub mod model;

mod backend;
mod error;
mod profile;
mod validate;

#[doc(inline)]
pub use backend::Backend;
#[doc(inline)]
pub use error::{BackendError, ParseError, ProfileError};
#[doc(inline)]
pub use geometry::Rect;
#[doc(inline)]
pub use model::{
    AdaptiveSync, BackendKind, Capabilities, Layout, Mode, Output, Position, Scale, Transform,
};
#[doc(inline)]
pub use profile::{Profile, ProfileOutput};
#[doc(inline)]
pub use validate::{Issue, Severity, validate};
