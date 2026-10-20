// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Chooses and opens the right Vernier backend for the running session.
//!
//! Detection order, unless a backend is forced:
//!
//! 1. Wayland session: wlr-output-management (which reports COSMIC when the
//!    COSMIC extension is present). This covers niri, sway, Hyprland, river,
//!    labwc, Wayfire and COSMIC.
//! 2. Wayland session without it: KDE Plasma or GNOME, chosen from
//!    `XDG_CURRENT_DESKTOP`.
//! 3. X11 session (`DISPLAY` set, no `WAYLAND_DISPLAY`): `RandR`.
//!
//! When `VNR_FIXTURE` names a file, the fixture backend wins over all of
//! these, so tests never touch a real display.
//!
//! # Examples
//!
//! ```no_run
//! # fn main() -> Result<(), vnr_core::BackendError> {
//! let mut backend = vnr_backends::open(None)?;
//! let layout = backend.snapshot()?;
//! println!("{} outputs via {}", layout.outputs.len(), backend.kind());
//! # Ok(())
//! # }
//! ```

// Rust guideline compliant 2026-05-18

mod fixture;

use std::env;

use vnr_core::{Backend, BackendError, BackendKind};

#[doc(inline)]
pub use fixture::{FIXTURE_ENV, Fixture, FixtureBackend};

/// Environment variable that forces a backend (`wlr`, `cosmic`, `kde`,
/// `mutter`, `x11`, `fixture`), equivalent to `vnr --backend`.
pub const BACKEND_ENV: &str = "VNR_BACKEND";

/// Opens a backend: `forced` if given, otherwise the detected one.
///
/// # Errors
///
/// Returns [`BackendError::is_unavailable`] when there is no graphical
/// session or the forced backend is not present, and passes through any
/// error from connecting.
pub fn open(forced: Option<BackendKind>) -> Result<Box<dyn Backend>, BackendError> {
    match forced {
        Some(kind) => open_kind(kind),
        None => detect_and_open(),
    }
}

fn open_kind(kind: BackendKind) -> Result<Box<dyn Backend>, BackendError> {
    match kind {
        BackendKind::Wlr => Ok(Box::new(vnr_wlr::WlrBackend::connect()?)),
        BackendKind::Cosmic => {
            let backend = vnr_wlr::WlrBackend::connect()?;
            if backend.kind() == BackendKind::Cosmic {
                Ok(Box::new(backend))
            } else {
                Err(BackendError::unavailable(
                    "the compositor does not offer COSMIC's output management extension",
                ))
            }
        }
        BackendKind::Kde => Ok(Box::new(vnr_kde::KdeBackend::connect()?)),
        BackendKind::Mutter => Ok(Box::new(vnr_mutter::MutterBackend::connect()?)),
        BackendKind::X11 => Ok(Box::new(vnr_x11::X11Backend::connect()?)),
        BackendKind::Fixture => Ok(Box::new(FixtureBackend::connect()?)),
    }
}

fn detect_and_open() -> Result<Box<dyn Backend>, BackendError> {
    let session = Session::from_env();
    match session.candidates().as_slice() {
        [] => Err(BackendError::unavailable(
            "no graphical session: neither `WAYLAND_DISPLAY` nor `DISPLAY` is set",
        )),
        candidates => {
            // Report the first candidate's reason: it is the desktop's own
            // backend, or wlr, and so the most telling.
            let mut first_error = None;
            for kind in candidates {
                match open_kind(*kind) {
                    Ok(backend) => return Ok(backend),
                    Err(e) if e.is_unavailable() => {
                        first_error.get_or_insert(e);
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(first_error.unwrap_or_else(|| BackendError::unavailable("no backend matched")))
        }
    }
}

/// The parts of the environment detection looks at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Session {
    fixture: bool,
    wayland: bool,
    x11: bool,
    desktop: String,
}

impl Session {
    fn from_env() -> Self {
        let set = |name| env::var_os(name).is_some_and(|v| !v.is_empty());
        Self {
            fixture: set(FIXTURE_ENV),
            wayland: set("WAYLAND_DISPLAY"),
            x11: set("DISPLAY"),
            desktop: env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .to_ascii_lowercase(),
        }
    }

    /// Backends to try, in order.
    fn candidates(&self) -> Vec<BackendKind> {
        let desktop_has = |name: &str| self.desktop.split(':').any(|d| d == name);
        let mut kinds = Vec::new();
        if self.fixture {
            kinds.push(BackendKind::Fixture);
            return kinds;
        }
        if self.wayland {
            // The desktop's own protocol first, then wlr, KWin's and Mutter's,
            // each of which answers "unavailable" quickly elsewhere.
            if desktop_has("kde") {
                kinds.push(BackendKind::Kde);
            }
            if desktop_has("gnome") {
                kinds.push(BackendKind::Mutter);
            }
            for kind in [BackendKind::Wlr, BackendKind::Kde, BackendKind::Mutter] {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
        } else if self.x11 {
            kinds.push(BackendKind::X11);
        }
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_tries_wlr_then_desktop_backend() {
        let session = Session {
            wayland: true,
            x11: true,
            desktop: "kde".into(),
            ..Session::default()
        };
        assert_eq!(
            session.candidates(),
            [BackendKind::Kde, BackendKind::Wlr, BackendKind::Mutter]
        );
        let gnome = Session {
            desktop: "ubuntu:gnome".into(),
            ..session.clone()
        };
        assert_eq!(
            gnome.candidates(),
            [BackendKind::Mutter, BackendKind::Wlr, BackendKind::Kde]
        );
        let unknown = Session {
            desktop: "niri".into(),
            ..session
        };
        assert_eq!(
            unknown.candidates(),
            [BackendKind::Wlr, BackendKind::Kde, BackendKind::Mutter]
        );
    }

    #[test]
    fn x11_only_when_no_wayland() {
        let session = Session {
            x11: true,
            ..Session::default()
        };
        assert_eq!(session.candidates(), [BackendKind::X11]);
        assert!(Session::default().candidates().is_empty());
    }

    #[test]
    fn fixture_wins_over_sessions() {
        let session = Session {
            fixture: true,
            wayland: true,
            ..Session::default()
        };
        assert_eq!(session.candidates(), [BackendKind::Fixture]);
    }
}
