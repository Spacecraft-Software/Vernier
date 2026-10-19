// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Input hardening (CLI Standard `validation-safety.md`).
//!
//! Arguments may come from an agent working on untrusted input, so every
//! argument is checked for encoding, control characters and length before
//! parsing, and every path is checked against an allow-list.

use std::env;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, ErrorCode};

/// Longest accepted argument, in bytes. A path on Linux is at most 4096
/// bytes (`PATH_MAX`); nothing `vnr` accepts is longer.
pub const MAX_ARG_BYTES: usize = 4096;

/// Longest accepted output (connector) name. Kernel connector names are
/// short (`HDMI-A-1`, `DP-10`); 64 leaves room for compositor-specific
/// names without admitting abuse.
pub const MAX_OUTPUT_NAME: usize = 64;

/// Placeholder in a hint that the caller replaces with the full command.
pub const RETRY: &str = "<the same command>";

/// Environment variable that widens the path allow-list, colon-separated.
pub const ALLOWED_PATHS_ENV: &str = "VNR_ALLOWED_PATHS";

/// Converts the process arguments to UTF-8 and rejects control characters
/// and oversized values.
///
/// # Errors
///
/// Returns `INVALID_ARGUMENT` (exit 2) naming the first bad argument.
pub fn arguments(raw: Vec<OsString>) -> Result<Vec<String>, AppError> {
    raw.into_iter()
        .enumerate()
        .map(|(index, arg)| {
            let arg = arg.into_string().map_err(|_bad| {
                AppError::new(
                    ErrorCode::InvalidArgument,
                    format!("argument {index} is not valid UTF-8"),
                    "vnr --help",
                )
            })?;
            text(&arg, MAX_ARG_BYTES).map_err(|e| e.with("argument_index", index))?;
            Ok(arg)
        })
        .collect()
}

/// Rejects control characters (tab, LF and CR excepted) and values longer
/// than `max_bytes`.
///
/// # Errors
///
/// Returns `INVALID_ARGUMENT` (exit 2).
pub fn text(value: &str, max_bytes: usize) -> Result<(), AppError> {
    if value.len() > max_bytes {
        return Err(AppError::new(
            ErrorCode::InvalidArgument,
            format!(
                "argument is {} bytes, longer than the limit of {max_bytes}",
                value.len()
            ),
            "vnr --help",
        ));
    }
    if let Some((offset, byte)) = value
        .bytes()
        .enumerate()
        .find(|(_, b)| matches!(b, 0x00..=0x08 | 0x0B..=0x0C | 0x0E..=0x1F | 0x7F))
    {
        return Err(AppError::new(
            ErrorCode::InvalidArgument,
            format!("control character 0x{byte:02X} at byte offset {offset} in an argument"),
            "vnr --help",
        ));
    }
    Ok(())
}

/// clap value parser for output names.
///
/// # Errors
///
/// Returns a message when the name is empty, too long or not a plausible
/// connector name.
pub fn output_name(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > MAX_OUTPUT_NAME {
        return Err(format!("output names are 1 to {MAX_OUTPUT_NAME} bytes"));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("output names contain no spaces or control characters".to_owned());
    }
    Ok(value.to_owned())
}

/// The directories a path argument may point into.
#[derive(Debug, Clone)]
pub struct Roots {
    roots: Vec<PathBuf>,
    widened: Vec<PathBuf>,
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

impl Roots {
    /// The default set: the working directory and Vernier's XDG config,
    /// data and cache directories, plus anything in `VNR_ALLOWED_PATHS`.
    #[must_use]
    pub fn standard() -> Self {
        let mut roots = Vec::new();
        if let Ok(cwd) = env::current_dir() {
            roots.push(cwd);
        }
        for (var, fallback) in [
            ("XDG_CONFIG_HOME", ".config"),
            ("XDG_DATA_HOME", ".local/share"),
            ("XDG_CACHE_HOME", ".cache"),
        ] {
            if let Some(dir) = xdg(var, fallback) {
                roots.push(dir.join("vernier"));
            }
        }
        let widened: Vec<PathBuf> = env::var_os(ALLOWED_PATHS_ENV)
            .map(|v| env::split_paths(&v).filter(|p| p.is_absolute()).collect())
            .unwrap_or_default();
        roots.extend(widened.iter().cloned());
        Self {
            roots: roots.iter().map(|r| canonical_dir(r)).collect(),
            widened,
        }
    }

    /// Adds compositor config directories (`niri`, `sway`) under
    /// `$XDG_CONFIG_HOME`, for `layout export --write`.
    #[must_use]
    pub fn with_compositor_configs(mut self) -> Self {
        if let Some(config) = xdg("XDG_CONFIG_HOME", ".config") {
            for name in ["niri", "sway"] {
                let dir = config.join(name);
                self.widened.push(dir.clone());
                self.roots.push(canonical_dir(&dir));
            }
        }
        self
    }

    /// Directories added beyond the default set, for an `[INFO]` line.
    #[must_use]
    pub fn widened(&self) -> &[PathBuf] {
        &self.widened
    }

    /// Resolves `input` and checks it lies inside an allowed root.
    ///
    /// `..` components are resolved lexically and the parent directory is
    /// canonicalized when it exists, so neither traversal nor a symlinked
    /// directory can escape the set.
    ///
    /// # Errors
    ///
    /// Returns `PERMISSION_DENIED` (exit 4) naming an allowed location.
    pub fn check(&self, input: &Path) -> Result<PathBuf, AppError> {
        let absolute = std::path::absolute(input).map_err(|e| {
            AppError::new(
                ErrorCode::InvalidArgument,
                format!("invalid path `{}`: {e}", input.display()),
                "vnr profile save ./<name>.toml",
            )
        })?;
        let lexical = normalize(&absolute);
        let resolved = match (lexical.parent(), lexical.file_name()) {
            (Some(parent), Some(name)) if parent.exists() => canonical_dir(parent).join(name),
            _ => lexical,
        };
        if self.roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(resolved)
        } else {
            Err(AppError::new(
                ErrorCode::PermissionDenied,
                format!(
                    "path `{}` is outside the allowed locations",
                    resolved.display()
                ),
                format!(
                    "{ALLOWED_PATHS_ENV}={} {RETRY}",
                    resolved.parent().unwrap_or(&resolved).display()
                ),
            )
            .with(
                "allowed_paths",
                self.roots
                    .iter()
                    .map(|r| r.display().to_string())
                    .collect::<Vec<_>>(),
            ))
        }
    }
}

/// Canonicalizes a directory when it exists, else returns it normalized.
fn canonical_dir(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_missing| normalize(dir))
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_rejected() {
        for bad in ["a\0b", "\x1b[31m", "x\x7f"] {
            let err = text(bad, 100).expect_err("control character must be rejected");
            assert_eq!(err.exit_code, 2);
        }
        text("tab\tand\nnewline", 100).expect("tab and newline are allowed");
        let err = text(&"x".repeat(101), 100).expect_err("too long");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn traversal_cannot_escape() {
        let base = std::env::temp_dir().canonicalize().unwrap();
        let roots = Roots {
            roots: vec![base.join("vernier-root")],
            widened: Vec::new(),
        };
        let escape = base.join("vernier-root/../../etc/passwd");
        let err = roots
            .check(&escape)
            .expect_err("traversal must be rejected");
        assert_eq!(err.exit_code, 4);
        let inside = base.join("vernier-root/./a/../desk.toml");
        assert_eq!(
            roots.check(&inside).unwrap(),
            base.join("vernier-root/desk.toml")
        );
    }

    #[test]
    fn output_names_are_bounded() {
        assert_eq!(output_name("HDMI-A-1").as_deref(), Ok("HDMI-A-1"));
        for bad in ["", "bad name", &"x".repeat(65)] {
            output_name(bad).expect_err("must be rejected");
        }
    }
}
