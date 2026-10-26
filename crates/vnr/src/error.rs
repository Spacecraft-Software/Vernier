// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structured errors and the exit-code map (CLI Standard §4, `exit-codes-errors.md`).
//!
//! Every failure in `vnr` becomes one [`AppError`]. In machine mode it is
//! written to stderr as a single line of JSON; in human mode as an
//! `[ERROR]` line with a `hint:` continuation. Its `exit_code` is the
//! process exit code, always.

use serde::Serialize;
use vnr_core::{BackendError, ProfileError};

use crate::time;

/// Stable error codes. The canonical ones come first; `vnr`-specific codes
/// use the 10–17 exit-code band and are listed in `vnr schema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// An argument value failed validation.
    InvalidArgument,
    /// A required argument was not supplied.
    MissingArgument,
    /// The named output, profile or file does not exist.
    NotFound,
    /// A path is outside the allowed set, or the file system refused access.
    PermissionDenied,
    /// The target already exists, or the outputs changed while applying.
    Conflict,
    /// An unexpected failure inside `vnr`.
    InternalError,
    /// No display server, or it lacks the required protocol.
    BackendUnavailable,
    /// The backend cannot express the requested change.
    Unsupported,
    /// The display server refused the configuration.
    Rejected,
    /// The display server sent something unexpected, or the connection failed.
    ProtocolError,
    /// The change was not confirmed in time and the previous one was restored.
    Reverted,
    /// The resulting layout has errors and was not sent.
    InvalidLayout,
    /// Restoring the previous configuration failed.
    RevertFailed,
    /// A shared library the editor loads at run time cannot be found.
    LibraryMissing,
}

impl ErrorCode {
    /// Every code, in exit-code order, for `vnr schema`.
    pub const ALL: [Self; 14] = [
        Self::InternalError,
        Self::InvalidArgument,
        Self::MissingArgument,
        Self::NotFound,
        Self::PermissionDenied,
        Self::Conflict,
        Self::BackendUnavailable,
        Self::Unsupported,
        Self::Rejected,
        Self::ProtocolError,
        Self::Reverted,
        Self::InvalidLayout,
        Self::RevertFailed,
        Self::LibraryMissing,
    ];

    /// The process exit code for this error.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::InternalError => 1,
            Self::InvalidArgument | Self::MissingArgument => 2,
            Self::NotFound => 3,
            Self::PermissionDenied => 4,
            Self::Conflict => 5,
            Self::BackendUnavailable => 10,
            Self::Unsupported => 11,
            Self::Rejected => 12,
            Self::ProtocolError => 13,
            Self::Reverted => 14,
            Self::InvalidLayout => 15,
            Self::RevertFailed => 16,
            Self::LibraryMissing => 17,
        }
    }

    /// The upper-snake-case name used in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::MissingArgument => "MISSING_ARGUMENT",
            Self::NotFound => "NOT_FOUND",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Conflict => "CONFLICT",
            Self::InternalError => "INTERNAL_ERROR",
            Self::BackendUnavailable => "BACKEND_UNAVAILABLE",
            Self::Unsupported => "UNSUPPORTED",
            Self::Rejected => "REJECTED",
            Self::ProtocolError => "PROTOCOL_ERROR",
            Self::Reverted => "REVERTED",
            Self::InvalidLayout => "INVALID_LAYOUT",
            Self::RevertFailed => "REVERT_FAILED",
            Self::LibraryMissing => "LIBRARY_MISSING",
        }
    }

    /// One-line meaning, for the `exit_codes` map in `vnr schema`.
    #[must_use]
    pub const fn meaning(self) -> &'static str {
        match self {
            Self::InvalidArgument => "an argument value failed validation",
            Self::MissingArgument => "a required argument was not supplied",
            Self::NotFound => "the named output, profile or file does not exist",
            Self::PermissionDenied => "a path is outside the allowed set or access was refused",
            Self::Conflict => "the target already exists, or the outputs changed while applying",
            Self::InternalError => "an unexpected failure inside vnr",
            Self::BackendUnavailable => "no display server, or it lacks the required protocol",
            Self::Unsupported => "the backend cannot express the requested change",
            Self::Rejected => "the display server refused the configuration",
            Self::ProtocolError => "the display server sent unexpected data or the link failed",
            Self::Reverted => "the change was not confirmed and the previous one was restored",
            Self::InvalidLayout => "the resulting layout has errors and was not sent",
            Self::RevertFailed => "restoring the previous configuration failed",
            Self::LibraryMissing => "a library the editor loads at run time cannot be found",
        }
    }
}

/// One failure, ready to report.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    /// Stable code.
    pub code: ErrorCode,
    /// Process exit code; always `code.exit_code()`.
    pub exit_code: u8,
    /// What happened, one sentence, lowercase, no trailing period.
    pub message: String,
    /// The exact command to run next.
    pub hint: String,
    /// When the error was produced, ISO 8601 UTC.
    pub timestamp: String,
    /// The invocation that failed.
    pub command: String,
    /// Extra structured context (for example `issues`).
    #[serde(flatten)]
    pub extensions: Box<serde_json::Map<String, serde_json::Value>>,
}

impl AppError {
    /// Creates an error; `command` is filled in by the reporter.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            exit_code: code.exit_code(),
            message: message.into(),
            hint: hint.into(),
            timestamp: time::now(),
            command: String::new(),
            extensions: Box::default(),
        }
    }

    /// Adds an extension field.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self {
        if let Ok(value) = serde_json::to_value(value) {
            self.extensions.insert(key.to_owned(), value);
        }
        self
    }

    /// Converts a backend failure, pointing the hint at the next useful command.
    #[must_use]
    pub fn backend(error: &BackendError) -> Self {
        let message = lower_first(&error.to_string());
        if error.is_unavailable() {
            Self::new(
                ErrorCode::BackendUnavailable,
                message,
                "vnr --backend <wlr|cosmic|kde|mutter|x11> output list --json",
            )
        } else if error.is_unsupported() {
            Self::new(ErrorCode::Unsupported, message, "vnr backend get --json")
        } else if error.is_rejected() {
            Self::new(ErrorCode::Rejected, message, "vnr output list --json")
        } else if error.is_cancelled() {
            Self::new(ErrorCode::Conflict, message, "vnr output list --json")
        } else {
            Self::new(
                ErrorCode::ProtocolError,
                message,
                "vnr --verbose output list",
            )
        }
    }

    /// The editor cannot open its window because `missing` libraries cannot
    /// be loaded. On NixOS a plain `cargo build` finds none of them; the Nix
    /// package wraps the binary with the right library path, so the hint
    /// builds it from the checkout. Elsewhere the hint asks the loader
    /// where the libraries are.
    #[must_use]
    pub fn missing_libraries(missing: &[&str], nixos: bool) -> Self {
        let hint = if nixos {
            "nix-build -E 'with import <nixpkgs> {}; callPackage ./packaging/default.nix {}' ; ./result/bin/vnr gui".to_owned()
        } else {
            let patterns: Vec<String> = missing.iter().map(|lib| format!("-e {lib}")).collect();
            format!("ldconfig -p | grep {}", patterns.join(" "))
        };
        let advice = match (nixos, missing.len()) {
            (true, _) => "this build is not wrapped with the Nix library path",
            (false, 1) => "install the distribution package that provides it",
            (false, _) => "install the distribution packages that provide them",
        };
        Self::new(
            ErrorCode::LibraryMissing,
            format!("the editor cannot load {}; {advice}", missing.join(", ")),
            hint,
        )
        .with("missing_libraries", missing)
    }

    /// Converts a profile failure.
    #[must_use]
    pub fn profile(error: &ProfileError, path: &str) -> Self {
        if let Some(name) = error.missing_output_name() {
            Self::new(
                ErrorCode::NotFound,
                format!("profile output `{name}` is not connected"),
                "vnr output list --fields name,make,model,serial --json",
            )
        } else if error.is_syntax() {
            Self::new(
                ErrorCode::InvalidArgument,
                format!(
                    "`{path}` is not a valid profile: {}",
                    lower_first(&error.to_string())
                ),
                "vnr schema profile apply",
            )
        } else {
            Self::new(
                ErrorCode::InternalError,
                lower_first(&error.to_string()),
                "vnr --verbose profile save <path>",
            )
        }
    }
}

/// Lowercases the first character, per the message style rules.
pub fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first)
            if first.is_uppercase() && !chars.clone().next().is_some_and(char::is_uppercase) =>
        {
            first.to_lowercase().chain(chars).collect()
        }
        _ => text.to_owned(),
    }
    .trim_end_matches('.')
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_libraries_are_named_with_a_runnable_hint() {
        let libs = ["libwayland-client.so.0", "libxkbcommon.so.0"];
        let e = AppError::missing_libraries(&libs, false);
        assert_eq!(e.code, ErrorCode::LibraryMissing);
        assert_eq!(e.exit_code, 17);
        assert!(
            e.message
                .contains("libwayland-client.so.0, libxkbcommon.so.0")
        );
        assert_eq!(
            e.hint,
            "ldconfig -p | grep -e libwayland-client.so.0 -e libxkbcommon.so.0"
        );
        assert_eq!(e.extensions["missing_libraries"], serde_json::json!(libs));
        let nix = AppError::missing_libraries(&libs, true);
        assert!(nix.hint.contains("packaging/default.nix"), "{}", nix.hint);
        assert!(nix.message.contains("Nix library path"), "{}", nix.message);
    }

    #[test]
    fn exit_codes_are_unique_and_outside_reserved_ranges() {
        for code in ErrorCode::ALL {
            let exit = code.exit_code();
            assert!(exit < 126, "{code:?} uses a POSIX-reserved code");
        }
        let tool_specific: Vec<u8> = ErrorCode::ALL
            .iter()
            .map(|c| c.exit_code())
            .filter(|e| *e >= 6)
            .collect();
        let mut sorted = tool_specific.clone();
        sorted.dedup();
        assert_eq!(sorted, tool_specific, "tool-specific codes are distinct");
    }

    #[test]
    fn serialized_code_matches_as_str() {
        for code in ErrorCode::ALL {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, code.as_str());
        }
    }

    #[test]
    fn messages_are_lowercased_without_period() {
        assert_eq!(lower_first("No Wayland session."), "no Wayland session");
        assert_eq!(lower_first("COSMIC rejected"), "COSMIC rejected");
    }
}
