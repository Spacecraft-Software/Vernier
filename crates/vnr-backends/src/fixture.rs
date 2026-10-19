// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A backend that keeps its outputs in a JSON file instead of a display
//! server.
//!
//! The file is named by `VNR_FIXTURE` and holds a [`Fixture`]. `snapshot`
//! reads it, `apply` writes the new layout back, and `test` only checks it.
//! Integration tests drive the real `vnr` binary against such a file, so
//! every command path is exercised without a compositor.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout};

/// Environment variable naming the fixture file.
pub const FIXTURE_ENV: &str = "VNR_FIXTURE";

/// The contents of a fixture file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// What the pretend server can do.
    #[serde(default = "full_capabilities")]
    pub capabilities: Capabilities,
    /// When true, every `test` and `apply` is rejected, as a compositor
    /// refusing a configuration would.
    #[serde(default)]
    pub reject: bool,
    /// The current outputs.
    #[serde(flatten)]
    pub layout: Layout,
}

fn full_capabilities() -> Capabilities {
    Capabilities {
        mirroring: true,
        adaptive_sync: true,
        adaptive_sync_automatic: true,
        primary: true,
        custom_modes: false,
        fractional_scale: true,
        test_only: true,
        persistent: false,
    }
}

/// The fixture backend.
#[derive(Debug)]
pub struct FixtureBackend {
    path: PathBuf,
    fixture: Fixture,
}

impl FixtureBackend {
    /// Opens the file named by `VNR_FIXTURE`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unavailable`] when the variable is unset,
    /// and an I/O or protocol error when the file cannot be read or parsed.
    pub fn connect() -> Result<Self, BackendError> {
        let path = std::env::var_os(FIXTURE_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| BackendError::unavailable(format!("{FIXTURE_ENV} is not set")))?;
        let fixture = read(&path)?;
        Ok(Self { path, fixture })
    }

    fn check(&self) -> Result<(), BackendError> {
        if self.fixture.reject {
            Err(BackendError::rejected(
                "the fixture rejects every configuration",
            ))
        } else {
            Ok(())
        }
    }
}

fn read(path: &PathBuf) -> Result<Fixture, BackendError> {
    let text = fs::read_to_string(path)
        .map_err(|e| BackendError::io(e, format!("cannot read fixture {}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| BackendError::protocol(format!("invalid fixture {}: {e}", path.display())))
}

impl Backend for FixtureBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Fixture
    }

    fn capabilities(&self) -> Capabilities {
        self.fixture.capabilities
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        self.fixture = read(&self.path)?;
        Ok(self.fixture.layout.clone())
    }

    fn test(&mut self, _desired: &Layout) -> Result<(), BackendError> {
        self.check()
    }

    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.check()?;
        let mut next = self.fixture.clone();
        next.layout = desired.clone();
        let text = serde_json::to_string_pretty(&next)
            .map_err(|e| BackendError::io(io::Error::other(e), "cannot serialize fixture"))?;
        fs::write(&self.path, text + "\n").map_err(|e| {
            BackendError::io(e, format!("cannot write fixture {}", self.path.display()))
        })?;
        self.fixture = next;
        Ok(())
    }
}
