// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Error types (M-ERRORS-CANONICAL-STRUCTS).

use std::backtrace::Backtrace;
use std::error::Error;
use std::fmt;

/// A value in text form could not be parsed (a mode, scale, transform, ...).
#[derive(Debug)]
pub struct ParseError {
    what: &'static str,
    input: String,
    backtrace: Backtrace,
}

impl ParseError {
    pub(crate) fn new(what: &'static str, input: &str) -> Self {
        Self {
            what,
            input: input.to_owned(),
            backtrace: Backtrace::capture(),
        }
    }

    /// The text that failed to parse.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The backtrace captured when the error was created.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {:?}", self.what, self.input)
    }
}

impl Error for ParseError {}

#[derive(Debug)]
enum ProfileErrorKind {
    Read(Box<toml::de::Error>),
    Write(Box<toml::ser::Error>),
    MissingOutput(String),
}

/// A profile could not be read, written, or matched to the connected outputs.
#[derive(Debug)]
pub struct ProfileError {
    kind: ProfileErrorKind,
    backtrace: Backtrace,
}

impl ProfileError {
    pub(crate) fn read(source: toml::de::Error) -> Self {
        Self::with_kind(ProfileErrorKind::Read(Box::new(source)))
    }

    pub(crate) fn write(source: toml::ser::Error) -> Self {
        Self::with_kind(ProfileErrorKind::Write(Box::new(source)))
    }

    pub(crate) fn missing_output(name: &str) -> Self {
        Self::with_kind(ProfileErrorKind::MissingOutput(name.to_owned()))
    }

    fn with_kind(kind: ProfileErrorKind) -> Self {
        Self {
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// Whether the profile text is not valid profile TOML.
    #[must_use]
    pub fn is_syntax(&self) -> bool {
        matches!(self.kind, ProfileErrorKind::Read(_))
    }

    /// The profile names an output that is not connected, if that is the error.
    #[must_use]
    pub fn missing_output_name(&self) -> Option<&str> {
        match &self.kind {
            ProfileErrorKind::MissingOutput(name) => Some(name),
            _ => None,
        }
    }

    /// The backtrace captured when the error was created.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ProfileErrorKind::Read(e) => write!(f, "invalid profile: {e}"),
            ProfileErrorKind::Write(e) => write!(f, "could not serialize profile: {e}"),
            ProfileErrorKind::MissingOutput(name) => {
                write!(f, "profile output {name:?} is not connected")
            }
        }
    }
}

impl Error for ProfileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ProfileErrorKind::Read(e) => Some(e.as_ref()),
            ProfileErrorKind::Write(e) => Some(e.as_ref()),
            ProfileErrorKind::MissingOutput(_) => None,
        }
    }
}

#[derive(Debug)]
enum BackendErrorKind {
    Unavailable,
    Unsupported,
    Rejected,
    Cancelled,
    Protocol,
    Io(std::io::Error),
}

/// A backend could not read or change the display configuration.
///
/// Backend crates construct it with the associated constructors; callers
/// branch on the `is_*` methods.
#[derive(Debug)]
pub struct BackendError {
    kind: BackendErrorKind,
    message: String,
    backtrace: Backtrace,
}

impl BackendError {
    fn with_kind(kind: BackendErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// The display server or the required protocol is not available.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Unavailable, message)
    }

    /// The backend cannot express the requested change.
    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Unsupported, message)
    }

    /// The compositor or server refused the configuration.
    #[must_use]
    pub fn rejected(message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Rejected, message)
    }

    /// The state changed while the request was in flight (hotplug, another
    /// client); taking a new snapshot and retrying may succeed.
    #[must_use]
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Cancelled, message)
    }

    /// The server sent something the backend did not expect.
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Protocol, message)
    }

    /// Talking to the server failed at the transport level.
    #[must_use]
    pub fn io(source: std::io::Error, message: impl Into<String>) -> Self {
        Self::with_kind(BackendErrorKind::Io(source), message)
    }

    /// Whether the backend's server or protocol is missing.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self.kind, BackendErrorKind::Unavailable)
    }

    /// Whether the backend cannot express the request.
    #[must_use]
    pub fn is_unsupported(&self) -> bool {
        matches!(self.kind, BackendErrorKind::Unsupported)
    }

    /// Whether the configuration was refused.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        matches!(self.kind, BackendErrorKind::Rejected)
    }

    /// Whether the request raced a state change and may be retried.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.kind, BackendErrorKind::Cancelled)
    }

    /// Whether the server violated the backend's expectations.
    #[must_use]
    pub fn is_protocol(&self) -> bool {
        matches!(self.kind, BackendErrorKind::Protocol)
    }

    /// The backtrace captured when the error was created.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            BackendErrorKind::Io(e) => Some(e),
            _ => None,
        }
    }
}

// Backend errors cross thread boundaries (the GUI runs backends on a
// blocking task) and are wrapped in `anyhow::Error` by the binary.
const _: () = {
    const fn assert_bounds<T: Send + Sync + 'static>() {}
    assert_bounds::<BackendError>();
    assert_bounds::<ProfileError>();
    assert_bounds::<ParseError>();
};
