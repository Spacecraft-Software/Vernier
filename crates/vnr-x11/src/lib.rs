// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for X11.
//!
//! **Status: not implemented yet; scheduled for milestone M2 in `PLAN.md`.**
//! The type and its [`Backend`] implementation exist so the rest of Vernier
//! (detection, CLI, GUI) is wired for this backend from the start;
//! [`X11Backend::connect`] returns an error until the milestone lands.
//!
//! The X11 backend speaks `RandR` 1.5 directly through `x11rb` instead of
//! running `xrandr` and parsing its text, so mode names such as `1080p`
//! cannot break it. Outputs map to CRTCs; scale is emulated with a CRTC
//! transform; mirroring is two outputs on one CRTC position; the primary
//! output is `RRSetOutputPrimary`. `ScreenChangeNotify` events give live
//! change tracking. Testing is possible headless with `Xvfb`.

// Rust guideline compliant 2026-05-18

use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout};

/// The message every operation returns until the backend is implemented.
const NOT_YET: &str = "the X11 backend is not implemented yet (PLAN.md milestone M2)";

/// A connection to X11 `RandR`.
#[derive(Debug)]
pub struct X11Backend {
    _connection: (),
}

impl X11Backend {
    /// Connects to the running session.
    ///
    /// # Errors
    ///
    /// Currently always returns [`BackendError::is_unsupported`].
    pub fn connect() -> Result<Self, BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }
}

impl Backend for X11Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }

    fn test(&mut self, _desired: &Layout) -> Result<(), BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }

    fn apply(&mut self, _desired: &Layout) -> Result<(), BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_reports_unsupported() {
        let err = X11Backend::connect().expect_err("backend is a stub");
        assert!(err.is_unsupported());
    }
}
