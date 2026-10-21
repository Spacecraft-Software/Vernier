// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier's configuration file, written in Nickel.
//!
//! The file lives at `$XDG_CONFIG_HOME/vernier/config.ncl` and is
//! optional: without it every setting has its default. It evaluates to a
//! record; today it holds key bindings:
//!
//! ```nickel
//! {
//!   keys = {
//!     apply = ["Ctrl+Enter", "Ctrl+s"],
//!     left = ["Left", "a"],
//!     keep = [],
//!   },
//! }
//! ```
//!
//! Each `keys` entry replaces that action's bindings, an empty list
//! unbinds it, and actions not named keep their defaults. Nickel's
//! `let`, imports and contracts are all available.
//!
//! Loading runs once when the editor starts or `vnr config check` runs;
//! it is small and serial, so it stays on the calling thread.

// Rust guideline compliant 2026-05-18

mod keys;

use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use nickel_lang_core::deserialize::EvalOrDeserError;
use nickel_lang_core::eval::cache::CacheImpl;
use nickel_lang_core::program::{Program, ProgramBuilder};
use serde::Deserialize;

pub use keys::{Action, Chord, Key, KeyMap};

/// The configuration file's name inside `$XDG_CONFIG_HOME/vernier`.
pub const FILE_NAME: &str = "config.ncl";

/// The settings the configuration file controls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Key bindings for the editor.
    pub keys: KeyMap,
}

/// The file's shape before validation.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    keys: BTreeMap<String, Vec<String>>,
}

impl Config {
    /// Loads and validates the configuration in `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, is not valid Nickel,
    /// does not evaluate to the expected record, or binds keys wrongly.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.is_file() {
            return Err(ConfigError::new(path, ConfigErrorKind::Missing));
        }
        let raw = evaluate(path)
            .map_err(|e| ConfigError::new(path, ConfigErrorKind::Evaluate(e.to_string())))?;
        Self::from_raw(&raw)
            .map_err(|problems| ConfigError::new(path, ConfigErrorKind::Invalid(problems)))
    }

    /// Loads the configuration from [`default_path`], or the defaults when
    /// there is no such file. Returns the file used, if any.
    ///
    /// # Errors
    ///
    /// As [`Config::load`], for a file that exists.
    pub fn load_default() -> Result<(Self, Option<PathBuf>), ConfigError> {
        match default_path() {
            Some(path) if path.is_file() => Self::load(&path).map(|c| (c, Some(path))),
            _ => Ok((Self::default(), None)),
        }
    }

    fn from_raw(raw: &Raw) -> Result<Self, Vec<String>> {
        let keys =
            KeyMap::with_overrides(raw.keys.iter().map(|(k, v)| (k.as_str(), v.as_slice())))?;
        Ok(Self { keys })
    }
}

/// The file's contract, merged into it so that Nickel reports a wrong type
/// with the field and the line it is on. Unknown fields are left to serde,
/// which names the expected ones.
const CONTRACT: &str = "{ keys | { _ : Array String } | optional }";

/// Evaluates the file, checked against [`CONTRACT`], and reads the result.
fn evaluate(path: &Path) -> Result<Raw, EvalOrDeserError> {
    let mut program: Program<CacheImpl> = ProgramBuilder::new()
        .add_path(path)
        .add_source_string(CONTRACT, "<vernier contract>")
        .build()?;
    let value = program
        .eval_full_for_export()
        .map_err(|error| EvalOrDeserError::Nickel {
            error,
            files: Some(program.files()),
        })?;
    Ok(Raw::deserialize(value)?)
}

/// `$XDG_CONFIG_HOME/vernier/config.ncl`, falling back to `~/.config`.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("vernier").join(FILE_NAME))
}

#[derive(Debug)]
enum ConfigErrorKind {
    Missing,
    Evaluate(String),
    Invalid(Vec<String>),
}

/// The configuration file could not be used.
#[derive(Debug)]
pub struct ConfigError {
    path: PathBuf,
    kind: ConfigErrorKind,
    backtrace: Backtrace,
}

impl ConfigError {
    fn new(path: &Path, kind: ConfigErrorKind) -> Self {
        Self {
            path: path.to_owned(),
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// The file that was being read.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file does not exist.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self.kind, ConfigErrorKind::Missing)
    }

    /// The individual problems: Nickel's report, or one line per bad binding.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        match &self.kind {
            ConfigErrorKind::Missing => vec![format!("{} does not exist", self.path.display())],
            ConfigErrorKind::Evaluate(report) => vec![report.trim_end().to_owned()],
            ConfigErrorKind::Invalid(problems) => problems.clone(),
        }
    }

    /// The backtrace captured when the error was created.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        match &self.kind {
            ConfigErrorKind::Missing => write!(f, "{path} does not exist"),
            ConfigErrorKind::Evaluate(report) => {
                write!(
                    f,
                    "{path} is not a valid configuration: {}",
                    report.trim_end()
                )
            }
            ConfigErrorKind::Invalid(problems) => {
                write!(
                    f,
                    "{path} has invalid key bindings: {}",
                    problems.join("; ")
                )
            }
        }
    }
}

impl Error for ConfigError {}

const _: () = {
    const fn assert_bounds<T: Send + Sync + 'static>() {}
    assert_bounds::<ConfigError>();
    assert_bounds::<Config>();
};

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn load(source: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, source).unwrap();
        Config::load(&path)
    }

    #[test]
    fn an_empty_record_means_defaults() {
        assert_eq!(load("{}").unwrap(), Config::default());
    }

    #[test]
    fn nickel_features_work() {
        let config = load(
            r#"
            let vim_left = ["h", "Left"] in
            {
              keys = {
                left = vim_left @ ["a"],
                keep = [],
                apply | Array String = ["Ctrl+Enter"],
              },
            }
            "#,
        )
        .unwrap();
        let a: Chord = "a".parse().unwrap();
        assert_eq!(config.keys.action(&a), Some(Action::Left));
        assert_eq!(config.keys.chords(Action::Keep), &[] as &[Chord]);
        assert_eq!(config.keys.chords(Action::Apply).len(), 1);
    }

    #[test]
    fn unknown_top_level_fields_are_rejected() {
        let error = load("{ kyes = {} }").unwrap_err();
        assert!(error.to_string().contains("kyes"), "{error}");
    }

    #[test]
    fn syntax_and_type_errors_name_the_file() {
        for source in ["{ keys = ", "{ keys = { left = \"h\" } }", "{ keys = 3 }"] {
            let error = load(source).unwrap_err();
            assert!(!error.is_missing());
            assert!(error.to_string().contains(FILE_NAME), "{source}: {error}");
        }
    }

    #[test]
    fn binding_problems_are_listed() {
        let error = load(r#"{ keys = { next = ["h"], jump = ["x"] } }"#).unwrap_err();
        let problems = error.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn type_errors_name_the_field() {
        let error = load("{ keys = { left = \"h\" } }").unwrap_err();
        let text = error.to_string();
        assert!(text.contains("left"), "{text}");
        assert!(text.contains("Array String"), "{text}");
    }

    #[test]
    fn a_missing_file_is_reported_as_missing() {
        let error = Config::load(Path::new("/nonexistent/vernier/config.ncl")).unwrap_err();
        assert!(error.is_missing());
    }
}
