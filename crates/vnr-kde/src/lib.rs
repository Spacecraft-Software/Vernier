// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for KDE Plasma.
//!
//! **Status: not implemented yet; scheduled for milestone M3 in `PLAN.md`.**
//! The type and its [`Backend`] implementation exist so the rest of Vernier
//! (detection, CLI, GUI) is wired for this backend from the start;
//! [`KdeBackend::connect`] returns an error until the milestone lands.
//!
//! `KWin` exposes outputs through `kde_output_device_v2` and changes them
//! through `kde_output_management_v2`, both bound from
//! `wayland-protocols-plasma`. Plasma saves applied configurations itself
//! (`KScreen`), so [`Capabilities::persistent`] will be true. The output
//! device also reports mirroring (`replication_source`), the primary output
//! order (`kde_output_order_v1`) and per-output VRR policy.

// Rust guideline compliant 2026-05-18

use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout};

/// The message every operation returns until the backend is implemented.
const NOT_YET: &str = "the KDE Plasma backend is not implemented yet (PLAN.md milestone M3)";

/// A connection to KDE Plasma output management.
#[derive(Debug)]
pub struct KdeBackend {
    _connection: (),
}

impl KdeBackend {
    /// Connects to the running session.
    ///
    /// # Errors
    ///
    /// Currently always returns [`BackendError::is_unsupported`].
    pub fn connect() -> Result<Self, BackendError> {
        Err(BackendError::unsupported(NOT_YET))
    }
}

impl Backend for KdeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Kde
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
        let err = KdeBackend::connect().expect_err("backend is a stub");
        assert!(err.is_unsupported());
    }
}
