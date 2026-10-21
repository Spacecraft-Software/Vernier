// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The records `vnr` prints in its `data` field.
//!
//! They are flat (depth at most two), use snake case, omit null fields, and
//! carry modes as text such as `2560x1440@59.951Hz`, so they convert cleanly
//! to Nushell tables, PowerShell objects and CSV rows. Each derives
//! `JsonSchema`; `vnr schema` publishes those schemas as `output_schema`.

use schemars::JsonSchema;
use serde::Serialize;
use vnr_core::{BackendKind, Capabilities, Issue, Layout, Output, Severity};

/// One output, as `output list` and `output get` print it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct OutputRecord {
    /// Connector name.
    pub name: String,
    /// Whether the output is on.
    pub enabled: bool,
    /// Active mode as `WIDTHxHEIGHT@RATEHz`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Active horizontal resolution in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Active vertical resolution in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Active refresh rate in millihertz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_mhz: Option<u32>,
    /// Left edge in logical pixels.
    pub x: i32,
    /// Top edge in logical pixels.
    pub y: i32,
    /// Scale factor.
    pub scale: f64,
    /// Rotation and reflection (`normal`, `90`, ..., `flipped-270`).
    pub transform: String,
    /// Adaptive sync state, when the backend reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_sync: Option<String>,
    /// The output this one mirrors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<String>,
    /// Whether this is the primary output.
    pub primary: bool,
    /// Manufacturer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// Model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Serial number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Description from the display server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Physical width in millimetres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_width_mm: Option<u32>,
    /// Physical height in millimetres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_height_mm: Option<u32>,
    /// The display's preferred mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_mode: Option<String>,
    /// Every advertised mode.
    pub modes: Vec<String>,
}

impl From<&Output> for OutputRecord {
    fn from(output: &Output) -> Self {
        Self {
            name: output.name.clone(),
            enabled: output.enabled,
            mode: output.mode.map(|m| m.to_string()),
            width: output.mode.map(|m| m.width),
            height: output.mode.map(|m| m.height),
            refresh_mhz: output.mode.map(|m| m.refresh_mhz).filter(|r| *r > 0),
            x: output.position.x,
            y: output.position.y,
            scale: output.scale.as_f64(),
            transform: output.transform.to_string(),
            adaptive_sync: output.adaptive_sync.map(|a| a.to_string()),
            mirror_of: output.mirror_of.clone(),
            primary: output.primary,
            make: output.make.clone(),
            model: output.model.clone(),
            serial: output.serial.clone(),
            description: Some(output.description.clone()).filter(|d| !d.is_empty()),
            physical_width_mm: output.physical_size_mm.map(|(w, _)| w),
            physical_height_mm: output.physical_size_mm.map(|(_, h)| h),
            preferred_mode: output.preferred_mode().map(|m| m.to_string()),
            modes: output.modes.iter().map(ToString::to_string).collect(),
        }
    }
}

/// One setting that differs between two layouts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ChangeRecord {
    /// Output connector name.
    pub output: String,
    /// Which setting: `enabled`, `mode`, `position`, `scale`, `transform`,
    /// `adaptive_sync`, `mirror_of` or `primary`.
    pub field: String,
    /// Current value; absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// New value; absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Lists the settings that differ between `before` and `after`.
#[must_use]
pub fn diff(before: &Layout, after: &Layout) -> Vec<ChangeRecord> {
    let mut changes = Vec::new();
    for new in &after.outputs {
        let Some(old) = before.get(&new.name) else {
            continue;
        };
        let mut push = |field: &str, from: Option<String>, to: Option<String>| {
            if from != to {
                changes.push(ChangeRecord {
                    output: new.name.clone(),
                    field: field.to_owned(),
                    from,
                    to,
                });
            }
        };
        push(
            "enabled",
            Some(old.enabled.to_string()),
            Some(new.enabled.to_string()),
        );
        if !new.enabled {
            continue;
        }
        let mode = |o: &Output| o.mode.map(|m| m.to_string());
        push("mode", mode(old).filter(|_| old.enabled), mode(new));
        push(
            "position",
            Some(old.position.to_string()),
            Some(new.position.to_string()),
        );
        push(
            "scale",
            Some(old.scale.to_string()),
            Some(new.scale.to_string()),
        );
        push(
            "transform",
            Some(old.transform.to_string()),
            Some(new.transform.to_string()),
        );
        push(
            "adaptive_sync",
            old.adaptive_sync.map(|a| a.to_string()),
            new.adaptive_sync.map(|a| a.to_string()),
        );
        push("mirror_of", old.mirror_of.clone(), new.mirror_of.clone());
        push(
            "primary",
            Some(old.primary.to_string()),
            Some(new.primary.to_string()),
        );
    }
    changes
}

/// A problem found in a layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct IssueRecord {
    /// `error` blocks applying; `warning` does not.
    pub severity: String,
    /// Stable kebab-case kind, e.g. `overlap`, `no-enabled-output`.
    pub code: String,
    /// What is wrong.
    pub message: String,
}

impl From<&Issue> for IssueRecord {
    fn from(issue: &Issue) -> Self {
        let code = serde_json::to_value(issue)
            .ok()
            .and_then(|v| v.get("issue").and_then(|c| c.as_str()).map(str::to_owned))
            .unwrap_or_default();
        Self {
            severity: match issue.severity() {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .to_owned(),
            code,
            message: issue.to_string(),
        }
    }
}

/// What `output set` and `profile apply` did, or would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ApplyReport {
    /// The backend used.
    pub backend: String,
    /// `planned` (dry run), `unchanged`, `applied` or `kept` (confirmed on a terminal).
    pub outcome: String,
    /// Settings that change.
    pub changes: Vec<ChangeRecord>,
    /// Problems found in the resulting layout.
    pub issues: Vec<IssueRecord>,
    /// Whether the display server accepted the layout in a test; absent when it cannot test.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_by_backend: Option<bool>,
    /// Whether the display server keeps the change across restarts by itself.
    pub persistent: bool,
    /// The niri include file `--persist` wrote, or would write on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persisted: Option<WriteReport>,
}

/// What `backend get` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a flat record of capability flags"
)]
pub struct BackendRecord {
    /// Backend identifier: `wlr`, `cosmic`, `kde`, `mutter`, `x11` or `fixture`.
    pub backend: String,
    /// Can make one output mirror another.
    pub mirroring: bool,
    /// Can toggle adaptive sync.
    pub adaptive_sync: bool,
    /// Distinguishes automatic from always-on adaptive sync.
    pub adaptive_sync_automatic: bool,
    /// Can mark a primary output.
    pub primary: bool,
    /// Accepts modes the display does not advertise.
    pub custom_modes: bool,
    /// Accepts non-integer scales.
    pub fractional_scale: bool,
    /// Can test a configuration without applying it.
    pub test_only: bool,
    /// Saves applied configurations by itself.
    pub persistent: bool,
}

impl BackendRecord {
    /// Builds the record.
    #[must_use]
    pub fn new(kind: BackendKind, caps: Capabilities) -> Self {
        Self {
            backend: kind.to_string(),
            mirroring: caps.mirroring,
            adaptive_sync: caps.adaptive_sync,
            adaptive_sync_automatic: caps.adaptive_sync_automatic,
            primary: caps.primary,
            custom_modes: caps.custom_modes,
            fractional_scale: caps.fractional_scale,
            test_only: caps.test_only,
            persistent: caps.persistent,
        }
    }
}

/// What `profile save`, `layout export --write` and `--persist` did, or would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WriteReport {
    /// The file, after resolving `.` and `..`.
    pub path: String,
    /// `planned` (dry run), `unchanged` (identical content already there) or `written`.
    pub outcome: String,
    /// Bytes in the file's new content.
    pub bytes: usize,
}

/// What `layout export` prints when it does not write a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ExportRecord {
    /// Configuration language.
    pub format: String,
    /// The configuration text.
    pub text: String,
}

/// What `vnr --version` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VersionRecord {
    /// Tool version.
    pub version: String,
    /// Maintainer.
    pub maintainer: String,
    /// Project page.
    pub website: String,
    /// SPDX license expression.
    pub license: String,
}

#[cfg(test)]
mod tests {
    use vnr_core::{Mode, Position, Scale};

    use super::*;

    fn layout() -> Layout {
        let mut a = Output::new("eDP-1");
        a.modes = vec![Mode::new(1920, 1080, 60_000)];
        a.mode = a.modes.first().copied();
        Layout::new(vec![a])
    }

    #[test]
    fn diff_lists_only_changed_settings() {
        let before = layout();
        let mut after = before.clone();
        after.outputs[0].scale = Scale::from_millis(1500).unwrap();
        after.outputs[0].position = Position::new(10, 0);
        let changes = diff(&before, &after);
        let fields: Vec<&str> = changes.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(fields, ["position", "scale"]);
        assert_eq!(changes[1].from.as_deref(), Some("1"));
        assert_eq!(changes[1].to.as_deref(), Some("1.5"));
        assert_eq!(diff(&before, &before), []);
    }

    #[test]
    fn disabling_reports_only_enabled() {
        let before = layout();
        let mut after = before.clone();
        after.outputs[0].enabled = false;
        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].field, "enabled");
    }

    #[test]
    fn records_omit_nulls() {
        let record = OutputRecord::from(&layout().outputs[0]);
        let json = serde_json::to_value(&record).unwrap();
        assert!(json.get("make").is_none());
        assert_eq!(json["mode"], "1920x1080@60.000Hz");
    }
}
