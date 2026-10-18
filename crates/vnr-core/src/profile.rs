// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The TOML profile format written by `vnr save` and read by `vnr apply`.
//!
//! ```toml
//! [[output]]
//! name = "eDP-1"
//! enabled = true
//! mode = "1920x1080@60.000Hz"
//! position = { x = 0, y = 0 }
//! scale = 1.25
//! transform = "normal"
//! adaptive-sync = "automatic"
//!
//! [[output]]
//! name = "HDMI-A-1"
//! make = "Dell Inc."
//! model = "DELL U2720Q"
//! enabled = true
//! mirror = "eDP-1"
//! ```
//!
//! An entry matches a connected output by make, model and serial when the
//! entry gives all three, so a profile survives a monitor moving to another
//! port; otherwise it matches by connector name.

use serde::{Deserialize, Serialize};

use crate::error::ProfileError;
use crate::model::{AdaptiveSync, Layout, Mode, Output, Position, Scale, Transform};

/// A saved display configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// One entry per output the profile configures.
    #[serde(rename = "output", default)]
    pub outputs: Vec<ProfileOutput>,
}

/// One output's settings in a [`Profile`]. Unset fields keep the output's
/// current value when the profile is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ProfileOutput {
    /// Connector name.
    pub name: String,
    /// Manufacturer, for matching across connectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// Model, for matching across connectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Serial number, for matching across connectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Whether the output is on.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Mode, e.g. `1920x1080@60.000Hz`; the rate may be omitted.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mode_text")]
    pub mode: Option<Mode>,
    /// Top-left corner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// Scale factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
    /// Rotation and reflection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// Adaptive sync state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_sync: Option<AdaptiveSync>,
    /// Connector name of the output to mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    /// Whether this is the primary output.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
}

const fn enabled_by_default() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature fixed by serde"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// Serializes modes as their text form so profiles stay hand-editable.
mod mode_text {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::model::Mode;

    #[expect(clippy::ref_option, reason = "signature fixed by serde")]
    pub(super) fn serialize<S: Serializer>(
        mode: &Option<Mode>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match mode {
            Some(mode) => serializer.serialize_str(&mode.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Mode>, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map(Some).map_err(serde::de::Error::custom)
    }
}

impl ProfileOutput {
    /// Whether this entry describes `output`.
    #[must_use]
    pub fn matches(&self, output: &Output) -> bool {
        match (&self.make, &self.model, &self.serial) {
            (Some(make), Some(model), Some(serial)) => {
                output.make.as_ref() == Some(make)
                    && output.model.as_ref() == Some(model)
                    && output.serial.as_ref() == Some(serial)
            }
            _ => output.name == self.name,
        }
    }

    fn from_output(output: &Output) -> Self {
        Self {
            name: output.name.clone(),
            make: output.make.clone(),
            model: output.model.clone(),
            serial: output.serial.clone(),
            enabled: output.enabled,
            mode: output.enabled.then_some(output.mode).flatten(),
            position: output.enabled.then_some(output.position),
            scale: output.enabled.then_some(output.scale),
            transform: output.enabled.then_some(output.transform),
            adaptive_sync: output.adaptive_sync.filter(|_| output.enabled),
            mirror: output.mirror_of.clone(),
            primary: output.primary,
        }
    }
}

impl Profile {
    /// Captures every output of `layout`.
    #[must_use]
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            outputs: layout
                .outputs
                .iter()
                .map(ProfileOutput::from_output)
                .collect(),
        }
    }

    /// Parses a profile.
    ///
    /// # Errors
    ///
    /// Returns an error if `text` is not valid profile TOML.
    pub fn from_toml(text: &str) -> Result<Self, ProfileError> {
        toml::from_str(text).map_err(ProfileError::read)
    }

    /// Serializes the profile as TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails, which only happens for
    /// values TOML cannot represent.
    pub fn to_toml(&self) -> Result<String, ProfileError> {
        toml::to_string_pretty(self).map_err(ProfileError::write)
    }

    /// Builds the layout that results from applying this profile on top of
    /// `current`. Outputs the profile does not mention keep their state.
    /// Mirror targets are rewritten to the connector names of the matched
    /// outputs.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first entry with no connected output.
    pub fn resolve(&self, current: &Layout) -> Result<Layout, ProfileError> {
        let mut desired = current.clone();
        let mut renames = Vec::with_capacity(self.outputs.len());
        for entry in &self.outputs {
            let output = desired
                .outputs
                .iter_mut()
                .find(|o| entry.matches(o))
                .ok_or_else(|| ProfileError::missing_output(&entry.name))?;
            renames.push((entry.name.clone(), output.name.clone()));
            output.enabled = entry.enabled;
            if let Some(mode) = entry.mode {
                output.mode = Some(output.find_mode(&mode).unwrap_or(mode));
            } else if entry.enabled && output.mode.is_none() {
                output.mode = output.preferred_mode();
            }
            if let Some(position) = entry.position {
                output.position = position;
            }
            if let Some(scale) = entry.scale {
                output.scale = scale;
            }
            if let Some(transform) = entry.transform {
                output.transform = transform;
            }
            if entry.adaptive_sync.is_some() {
                output.adaptive_sync = entry.adaptive_sync;
            }
            output.mirror_of.clone_from(&entry.mirror);
            output.primary = entry.primary;
        }
        for output in &mut desired.outputs {
            if let Some(source) = &output.mirror_of
                && let Some((_, actual)) = renames.iter().find(|(saved, _)| saved == source)
            {
                output.mirror_of = Some(actual.clone());
            }
        }
        Ok(desired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> Layout {
        let mut laptop = Output::new("eDP-1");
        laptop.modes = vec![Mode::new(1920, 1080, 60_008)];
        laptop.mode = laptop.modes.first().copied();
        let mut dell = Output::new("DP-3");
        dell.make = Some("Dell Inc.".into());
        dell.model = Some("U2720Q".into());
        dell.serial = Some("ABC123".into());
        dell.enabled = false;
        dell.modes = vec![Mode::new(3840, 2160, 59_997), Mode::new(1920, 1080, 60_000)];
        Layout::new(vec![laptop, dell])
    }

    #[test]
    fn round_trip_preserves_profile() {
        let profile = Profile::from_layout(&connected());
        let text = profile.to_toml().unwrap();
        assert_eq!(Profile::from_toml(&text).unwrap(), profile);
    }

    #[test]
    fn resolve_matches_by_identity_across_connectors() {
        let text = r#"
            [[output]]
            name = "HDMI-A-1"
            make = "Dell Inc."
            model = "U2720Q"
            serial = "ABC123"
            mode = "3840x2160@60Hz"
            position = { x = 1920, y = 0 }
            scale = 2.0

            [[output]]
            name = "eDP-1"
            mirror = "HDMI-A-1"
        "#;
        let profile = Profile::from_toml(text).unwrap();
        let desired = profile.resolve(&connected()).unwrap();
        let dell = desired.get("DP-3").unwrap();
        assert!(dell.enabled);
        assert_eq!(dell.mode, Some(Mode::new(3840, 2160, 59_997)));
        assert_eq!(dell.position, Position::new(1920, 0));
        assert_eq!(dell.scale.millis(), 2000);
        assert_eq!(
            desired.get("eDP-1").unwrap().mirror_of.as_deref(),
            Some("DP-3")
        );
    }

    #[test]
    fn missing_output_is_reported() {
        let profile = Profile::from_toml("[[output]]\nname = \"DP-9\"\n").unwrap();
        let err = profile.resolve(&connected()).unwrap_err();
        assert_eq!(err.missing_output_name(), Some("DP-9"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = Profile::from_toml("[[output]]\nname = \"A\"\nbogus = 1\n").unwrap_err();
        assert!(err.is_syntax());
    }
}
