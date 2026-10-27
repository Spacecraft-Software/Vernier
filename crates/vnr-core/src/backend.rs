// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The trait every display backend implements.

use crate::error::BackendError;
use crate::model::{BackendKind, Capabilities, Layout};

/// What became of a configuration a backend was asked to save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// The user confirmed it, and the server saved it.
    Kept,
    /// The user refused it, or did not answer in time, and the server
    /// restored the previous configuration.
    Restored,
    /// The new configuration stayed on screen, but the server gave no sign
    /// that it saved it; it may last only until the session ends.
    Unverified,
}

/// A connection to a display server that can read and change outputs.
///
/// Backends are deliberately synchronous and short-lived: each operation
/// is a handful of round trips, and the callers (CLI, GUI) open a fresh
/// backend per operation on a blocking thread. That keeps every apply
/// based on a just-taken snapshot, which the wlr protocol requires anyway
/// (configurations carry the serial of the state they were built from).
///
/// The trait is object-safe so `vnr-backends` can choose an implementation
/// at run time; the dynamic dispatch cost is irrelevant next to a display
/// server round trip.
pub trait Backend: Send {
    /// Which backend this is. A wlr backend reports [`BackendKind::Cosmic`]
    /// when the COSMIC extension is present.
    fn kind(&self) -> BackendKind;

    /// What this backend can express on this server.
    fn capabilities(&self) -> Capabilities;

    /// Reads the current configuration of every connected output.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be reached or sends malformed
    /// state.
    fn snapshot(&mut self) -> Result<Layout, BackendError>;

    /// Asks the server whether `desired` would be accepted, without
    /// changing anything.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_rejected`] if the server refuses the
    /// configuration, [`BackendError::is_unsupported`] if it cannot test,
    /// or another error if the request could not be made.
    fn test(&mut self, desired: &Layout) -> Result<(), BackendError>;

    /// Applies `desired` to every output it names.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_rejected`] if the server refuses the
    /// configuration, [`BackendError::is_cancelled`] if the outputs changed
    /// in the meantime, or another error if the request could not be made.
    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError>;

    /// Applies `desired` and asks the server to keep it across sessions.
    ///
    /// The server shows its own keep-or-revert prompt and restores the
    /// previous configuration unless the user confirms, so callers must
    /// not prompt as well. Blocks until the server has decided. Only
    /// backends whose [`Capabilities::save_on_request`] is set support it.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unsupported`] by default, or the errors
    /// of [`Backend::apply`].
    fn apply_and_save(&mut self, desired: &Layout) -> Result<Saved, BackendError> {
        let _ = desired;
        Err(BackendError::unsupported(
            "this display server cannot save a configuration on request",
        ))
    }
}
