// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for GNOME (Mutter).
//!
//! **Status: not implemented yet; scheduled for milestone M4 in `PLAN.md`.**
//! The type and its [`Backend`] implementation exist so the rest of Vernier
//! (detection, CLI, GUI) is wired for this backend from the start;
//! [`MutterBackend::connect`] returns an error until the milestone lands.
//!
//! Mutter is configured over D-Bus: `org.gnome.Mutter.DisplayConfig`
//! `GetCurrentState` returns monitors, modes and logical monitors, and
//! `ApplyMonitorsConfig` takes a serial and a method (verify, temporary or
//! persistent). Verify maps to [`Backend::test`]; persistent writes
//! `~/.config/monitors.xml`, so [`Capabilities::persistent`] will be true.
//! Mirroring is a logical monitor holding several monitors. The connection
//! will use `zbus`, which is pure Rust.

// Rust guideline compliant 2026-05-18

use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout};

/// The message every operation returns until the backend is implemented.
const NOT_YET: &str = "the GNOME (Mutter) backend is not implemented yet (PLAN.md milestone M4)";

/// A connection to GNOME Mutter display configuration.
#[derive(Debug)]
pub struct MutterBackend {
    _connection: (),
}

impl MutterBackend {
    /// Connects to the running session.
    ///
    /// # Errors
    ///
    /// Currently always returns [`BackendError::is_unsupported`].
    pub fn connect() -> Result<Self, BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }
}

impl Backend for MutterBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Mutter
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
        let err = MutterBackend::connect().expect_err("backend is a stub");
        assert!(err.is_unsupported());
    }
}
