// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The display layout model shared by every backend.

use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ParseError;
use crate::geometry::Rect;

/// A display mode: a resolution and a refresh rate.
///
/// Two modes describe the same timing when [`Mode::same_timing`] is true;
/// the `preferred` flag is metadata reported by the display and is ignored
/// for that comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mode {
    /// Horizontal resolution in physical pixels.
    pub width: u32,
    /// Vertical resolution in physical pixels.
    pub height: u32,
    /// Refresh rate in millihertz (60 Hz is `60_000`). Zero means unknown.
    pub refresh_mhz: u32,
    /// Whether the display reports this as its preferred (native) mode.
    #[serde(default)]
    pub preferred: bool,
}

/// Refresh rates closer than this are treated as the same rate.
///
/// Displays report rates such as 59.951 Hz and 60.000 Hz for what users
/// call "60 Hz"; a profile written on one machine must still match on the
/// next. Half a hertz separates every pair of distinct rates seen in
/// practice (e.g. 59.94 vs 60.00 is 60 mHz apart and still matches, while
/// 60 vs 75 never collide).
const REFRESH_TOLERANCE_MHZ: u32 = 500;

impl Mode {
    /// Creates a non-preferred mode.
    #[must_use]
    pub const fn new(width: u32, height: u32, refresh_mhz: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz,
            preferred: false,
        }
    }

    /// Whether both modes have the same resolution and refresh rate.
    #[must_use]
    pub const fn same_timing(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.refresh_mhz == other.refresh_mhz
    }

    /// Whether `other` has the same resolution and a refresh rate within
    /// half a hertz, or either refresh rate is unknown.
    #[must_use]
    pub const fn approx_eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && (self.refresh_mhz == 0
                || other.refresh_mhz == 0
                || self.refresh_mhz.abs_diff(other.refresh_mhz) <= REFRESH_TOLERANCE_MHZ)
    }

    /// Formats the refresh rate as hertz with three decimals, e.g. `60.000`.
    #[must_use]
    pub fn refresh_hz(&self) -> String {
        format!("{}.{:03}", self.refresh_mhz / 1000, self.refresh_mhz % 1000)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.refresh_mhz == 0 {
            write!(f, "{}x{}", self.width, self.height)
        } else {
            write!(f, "{}x{}@{}Hz", self.width, self.height, self.refresh_hz())
        }
    }
}

impl FromStr for Mode {
    type Err = ParseError;

    /// Parses `WIDTHxHEIGHT`, optionally followed by `@RATE` or `@RATEHz`,
    /// where `RATE` is in hertz with up to three decimals.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseError::new("mode", s);
        let (size, rate) = match s.split_once('@') {
            Some((size, rate)) => (size, Some(rate)),
            None => (s, None),
        };
        let (w, h) = size.trim().split_once(['x', 'X']).ok_or_else(invalid)?;
        let width = w.trim().parse().map_err(|_e| invalid())?;
        let height = h.trim().parse().map_err(|_e| invalid())?;
        let refresh_mhz = match rate {
            None => 0,
            Some(rate) => {
                let rate = rate.trim();
                let rate = rate
                    .strip_suffix("Hz")
                    .or_else(|| rate.strip_suffix("hz"))
                    .unwrap_or(rate);
                parse_millis(rate).ok_or_else(invalid)?
            }
        };
        if width == 0 || height == 0 {
            return Err(invalid());
        }
        Ok(Self::new(width, height, refresh_mhz))
    }
}

/// Parses a non-negative decimal with up to three fractional digits into
/// thousandths, without going through floating point.
fn parse_millis(s: &str) -> Option<u32> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int: u32 = int.parse().ok()?;
    let mut frac_value = 0;
    for (i, digit) in frac.bytes().enumerate() {
        // The loop runs at most three times (checked above), so the place
        // values are 100, 10 and 1.
        let place = 10_u32.pow(2 - u32::try_from(i).ok()?);
        frac_value += u32::from(digit - b'0') * place;
    }
    int.checked_mul(1000)?.checked_add(frac_value)
}

/// Rotation and reflection applied to an output, counter-clockwise, as in
/// `wl_output.transform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Transform {
    /// No transform.
    #[default]
    #[serde(rename = "normal")]
    Normal,
    /// Rotated 90 degrees.
    #[serde(rename = "90")]
    Rotate90,
    /// Rotated 180 degrees.
    #[serde(rename = "180")]
    Rotate180,
    /// Rotated 270 degrees.
    #[serde(rename = "270")]
    Rotate270,
    /// Flipped around the vertical axis.
    #[serde(rename = "flipped")]
    Flipped,
    /// Flipped, then rotated 90 degrees.
    #[serde(rename = "flipped-90")]
    Flipped90,
    /// Flipped, then rotated 180 degrees.
    #[serde(rename = "flipped-180")]
    Flipped180,
    /// Flipped, then rotated 270 degrees.
    #[serde(rename = "flipped-270")]
    Flipped270,
}

impl Transform {
    /// Every transform, in `wl_output.transform` order.
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Rotate90,
        Self::Rotate180,
        Self::Rotate270,
        Self::Flipped,
        Self::Flipped90,
        Self::Flipped180,
        Self::Flipped270,
    ];

    /// Whether width and height swap under this transform.
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }

    /// The name used by profiles, niri and sway (`normal`, `90`, `flipped-90`, ...).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        }
    }
}

impl fmt::Display for Transform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Transform {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| ParseError::new("transform", s))
    }
}

/// An output scale factor, stored as thousandths.
///
/// Fixed point keeps scales comparable and hashable, and matches the
/// precision COSMIC's `scale_1000` extension exposes. Profiles and the
/// command line read and write it as a decimal such as `1.25`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scale(NonZeroU32);

impl Scale {
    /// Scale 1.0.
    pub const ONE: Self = Self(NonZeroU32::new(1000).expect("1000 is not zero"));

    /// Scales offered by the GUI's preset list.
    pub const PRESETS: [u32; 9] = [1000, 1250, 1500, 1750, 2000, 2250, 2500, 2750, 3000];

    /// Creates a scale from thousandths; returns `None` for zero.
    #[must_use]
    pub const fn from_millis(millis: u32) -> Option<Self> {
        match NonZeroU32::new(millis) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Creates a scale from a floating-point factor, rounded to thousandths.
    ///
    /// Returns `None` for non-finite, zero, negative or absurdly large values.
    #[must_use]
    pub fn from_f64(factor: f64) -> Option<Self> {
        if !factor.is_finite() || factor <= 0.0 || factor > 100.0 {
            return None;
        }
        let millis = (factor * 1000.0).round();
        // Bounded by the range check above: 0 < millis <= 100_000.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range checked above"
        )]
        let millis = millis as u32;
        Self::from_millis(millis)
    }

    /// The scale in thousandths (1.25 is 1250).
    #[must_use]
    pub const fn millis(self) -> u32 {
        self.0.get()
    }

    /// The scale as a floating-point factor.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.0.get()) / 1000.0
    }

    /// Divides a physical length by this scale, rounding to the nearest pixel.
    #[must_use]
    pub fn unscale(self, physical: u32) -> u32 {
        let millis = u64::from(self.millis());
        let logical = (u64::from(physical) * 1000 + millis / 2) / millis;
        u32::try_from(logical).unwrap_or(u32::MAX)
    }
}

impl Default for Scale {
    fn default() -> Self {
        Self::ONE
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let millis = self.millis();
        let int = millis / 1000;
        let frac = millis % 1000;
        if frac == 0 {
            write!(f, "{int}")
        } else {
            let digits = format!("{frac:03}");
            write!(f, "{int}.{}", digits.trim_end_matches('0'))
        }
    }
}

impl FromStr for Scale {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_millis(s.trim())
            .and_then(Self::from_millis)
            .ok_or_else(|| ParseError::new("scale", s))
    }
}

impl Serialize for Scale {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.as_f64())
    }
}

impl<'de> Deserialize<'de> for Scale {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let factor = f64::deserialize(deserializer)?;
        Self::from_f64(factor)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid scale {factor}")))
    }
}

/// Variable refresh rate (adaptive sync, `FreeSync`, `G-Sync`) state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdaptiveSync {
    /// Always use a fixed refresh rate.
    #[default]
    Disabled,
    /// Let the compositor enable it for full-screen content (COSMIC, niri `on-demand`).
    Automatic,
    /// Always enabled.
    Always,
}

impl AdaptiveSync {
    /// Every state, from least to most aggressive.
    pub const ALL: [Self; 3] = [Self::Disabled, Self::Automatic, Self::Always];
}

impl fmt::Display for AdaptiveSync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disabled => "disabled",
            Self::Automatic => "automatic",
            Self::Always => "always",
        })
    }
}

impl FromStr for AdaptiveSync {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" => Ok(Self::Disabled),
            "automatic" | "auto" | "on-demand" => Ok(Self::Automatic),
            "always" | "on" => Ok(Self::Always),
            _ => Err(ParseError::new("adaptive sync state", s)),
        }
    }
}

/// A position in the compositor's global (logical) coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Position {
    /// Horizontal offset in logical pixels.
    pub x: i32,
    /// Vertical offset in logical pixels.
    pub y: i32,
}

impl Position {
    /// Creates a position.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

/// One physical display connector and its configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Connector name, unique per session (`eDP-1`, `HDMI-A-1`).
    pub name: String,
    /// Human-readable description from the compositor.
    #[serde(default)]
    pub description: String,
    /// Manufacturer, when known.
    #[serde(default)]
    pub make: Option<String>,
    /// Model name, when known.
    #[serde(default)]
    pub model: Option<String>,
    /// Serial number, when known.
    #[serde(default)]
    pub serial: Option<String>,
    /// Physical width and height in millimeters, when known.
    #[serde(default)]
    pub physical_size_mm: Option<(u32, u32)>,
    /// Modes the display advertises.
    #[serde(default)]
    pub modes: Vec<Mode>,
    /// Whether the output is turned on.
    pub enabled: bool,
    /// The active mode; `None` while disabled or when unknown.
    #[serde(default)]
    pub mode: Option<Mode>,
    /// Top-left corner in the global coordinate space.
    #[serde(default)]
    pub position: Position,
    /// Rotation and reflection.
    #[serde(default)]
    pub transform: Transform,
    /// Scale factor.
    #[serde(default)]
    pub scale: Scale,
    /// Adaptive sync state; `None` when the display or backend cannot report it.
    #[serde(default)]
    pub adaptive_sync: Option<AdaptiveSync>,
    /// Name of the output this one mirrors, if any.
    #[serde(default)]
    pub mirror_of: Option<String>,
    /// Whether this is the primary output (X11, or Xwayland on COSMIC/KDE).
    #[serde(default)]
    pub primary: bool,
}

impl Output {
    /// Creates an enabled output with no modes at the origin.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            make: None,
            model: None,
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            enabled: true,
            mode: None,
            position: Position::default(),
            transform: Transform::Normal,
            scale: Scale::ONE,
            adaptive_sync: None,
            mirror_of: None,
            primary: false,
        }
    }

    /// The display's preferred mode, falling back to the largest one.
    #[must_use]
    pub fn preferred_mode(&self) -> Option<Mode> {
        self.modes
            .iter()
            .find(|m| m.preferred)
            .copied()
            .or_else(|| {
                self.modes
                    .iter()
                    .max_by_key(|m| (u64::from(m.width) * u64::from(m.height), m.refresh_mhz))
                    .copied()
            })
    }

    /// Finds the advertised mode that best matches `wanted`: the same
    /// resolution and the nearest refresh rate within half a hertz, or the
    /// highest rate when `wanted` has no rate.
    #[must_use]
    pub fn find_mode(&self, wanted: &Mode) -> Option<Mode> {
        self.modes
            .iter()
            .filter(|m| m.approx_eq(wanted))
            .min_by_key(|m| {
                if wanted.refresh_mhz == 0 {
                    u32::MAX - m.refresh_mhz
                } else {
                    m.refresh_mhz.abs_diff(wanted.refresh_mhz)
                }
            })
            .copied()
    }

    /// Whether this output takes up its own area of the desktop: enabled,
    /// with a mode, and not mirroring another output.
    #[must_use]
    pub fn is_placed(&self) -> bool {
        self.enabled && self.mode.is_some() && self.mirror_of.is_none()
    }

    /// Size in logical pixels after transform and scale, if it has a mode.
    #[must_use]
    pub fn logical_size(&self) -> Option<(u32, u32)> {
        let mode = self.mode?;
        let (w, h) = if self.transform.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        Some((self.scale.unscale(w), self.scale.unscale(h)))
    }

    /// The area this output covers in the global space, if it is placed.
    #[must_use]
    pub fn logical_rect(&self) -> Option<Rect> {
        if !self.is_placed() {
            return None;
        }
        let (width, height) = self.logical_size()?;
        Some(Rect::new(self.position.x, self.position.y, width, height))
    }

    /// A short label for the output: make and model when known, else the
    /// description, else the connector name.
    #[must_use]
    pub fn label(&self) -> String {
        match (&self.make, &self.model) {
            (Some(make), Some(model)) => format!("{make} {model}"),
            _ if !self.description.is_empty() => self.description.clone(),
            _ => self.name.clone(),
        }
    }
}

/// The full set of outputs a backend reports, or a desired configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Layout {
    /// Every connected output, in the order the backend reported them.
    pub outputs: Vec<Output>,
}

impl Layout {
    /// Creates a layout from outputs.
    #[must_use]
    pub fn new(outputs: Vec<Output>) -> Self {
        Self { outputs }
    }

    /// Finds an output by connector name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name == name)
    }

    /// Finds an output by connector name, mutably.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.name == name)
    }

    /// Logical rectangles of every placed output, with its index.
    pub fn placed_rects(&self) -> impl Iterator<Item = (usize, Rect)> + '_ {
        self.outputs
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.logical_rect().map(|r| (i, r)))
    }

    /// The smallest rectangle containing every placed output.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.placed_rects()
            .map(|(_, r)| r)
            .reduce(|a, b| a.union(&b))
    }

    /// Enables an output with its preferred mode, placed to the right of
    /// everything else. Does nothing for an unknown name.
    pub fn enable(&mut self, name: &str) {
        let x = self
            .placed_rects()
            .filter(|(i, _)| self.outputs[*i].name != name)
            .map(|(_, r)| r.right())
            .max()
            .unwrap_or(0);
        if let Some(output) = self.get_mut(name) {
            output.enabled = true;
            if output.mode.is_none() {
                output.mode = output.preferred_mode();
            }
            output.mirror_of = None;
            output.position = Position::new(x, 0);
        }
    }
}

/// Which display backend produced or will apply a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// wlr-output-management (niri, sway, Hyprland, river, labwc, Wayfire).
    Wlr,
    /// wlr-output-management with COSMIC's `zcosmic_output_manager_v1` extension.
    Cosmic,
    /// KDE Plasma's `kde_output_management_v2`.
    Kde,
    /// GNOME Mutter's `org.gnome.Mutter.DisplayConfig` D-Bus interface.
    Mutter,
    /// X11 `RandR`.
    X11,
    /// A JSON file standing in for a display server, named by `VNR_FIXTURE`.
    /// Used by tests and demonstrations; never chosen by detection unless
    /// that variable is set.
    Fixture,
}

impl BackendKind {
    /// Every backend, in detection order.
    pub const ALL: [Self; 6] = [
        Self::Fixture,
        Self::Cosmic,
        Self::Wlr,
        Self::Kde,
        Self::Mutter,
        Self::X11,
    ];

    /// The identifier used by `--backend` and `VNR_BACKEND`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wlr => "wlr",
            Self::Cosmic => "cosmic",
            Self::Kde => "kde",
            Self::Mutter => "mutter",
            Self::X11 => "x11",
            Self::Fixture => "fixture",
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BackendKind {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| ParseError::new("backend", s))
    }
}

/// What a backend can express. The GUI hides controls a backend cannot
/// apply, and [`crate::validate()`] flags settings it would reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent capability flags, serialized as-is for `vnr backend get --json`"
)]
pub struct Capabilities {
    /// Can make one output mirror another.
    pub mirroring: bool,
    /// Can toggle adaptive sync at all.
    pub adaptive_sync: bool,
    /// Distinguishes [`AdaptiveSync::Automatic`] from [`AdaptiveSync::Always`].
    pub adaptive_sync_automatic: bool,
    /// Can mark a primary output.
    pub primary: bool,
    /// Accepts modes the display does not advertise.
    pub custom_modes: bool,
    /// Accepts non-integer scales.
    pub fractional_scale: bool,
    /// Can test a configuration without applying it.
    pub test_only: bool,
    /// Saves applied configurations itself (KDE, GNOME); otherwise Vernier
    /// must write them to the compositor's config.
    pub persistent: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_round_trips_through_text() {
        let mode: Mode = "2560x1440@143.973Hz".parse().unwrap();
        assert_eq!(mode, Mode::new(2560, 1440, 143_973));
        assert_eq!(mode.to_string(), "2560x1440@143.973Hz");
        assert_eq!("1920x1080".parse::<Mode>().unwrap().refresh_mhz, 0);
        assert_eq!("1920x1080@60".parse::<Mode>().unwrap().refresh_mhz, 60_000);
        assert_eq!("800X600@59.9".parse::<Mode>().unwrap().refresh_mhz, 59_900);
    }

    #[test]
    fn mode_rejects_garbage() {
        for bad in [
            "",
            "1920",
            "x1080",
            "0x0",
            "1920x1080@",
            "1920x1080@60.0001",
            "axb",
        ] {
            let _err = bad
                .parse::<Mode>()
                .expect_err("malformed mode text must not parse");
        }
    }

    #[test]
    fn scale_formats_and_parses() {
        let scale: Scale = "1.25".parse().unwrap();
        assert_eq!(scale.millis(), 1250);
        assert_eq!(scale.to_string(), "1.25");
        assert_eq!(Scale::ONE.to_string(), "1");
        assert_eq!(Scale::from_f64(1.333_333).unwrap().millis(), 1333);
        assert!(Scale::from_f64(0.0).is_none());
        assert!(Scale::from_f64(f64::NAN).is_none());
        let _err = "0".parse::<Scale>().expect_err("zero is not a scale");
    }

    #[test]
    fn logical_size_applies_transform_and_scale() {
        let mut output = Output::new("DP-1");
        output.mode = Some(Mode::new(3840, 2160, 60_000));
        output.scale = Scale::from_millis(2000).unwrap();
        assert_eq!(output.logical_size(), Some((1920, 1080)));
        output.transform = Transform::Rotate90;
        assert_eq!(output.logical_size(), Some((1080, 1920)));
        output.scale = Scale::from_millis(1500).unwrap();
        assert_eq!(output.logical_size(), Some((1440, 2560)));
    }

    #[test]
    fn find_mode_tolerates_refresh_drift() {
        let mut output = Output::new("DP-1");
        output.modes = vec![
            Mode::new(1920, 1080, 59_940),
            Mode::new(1920, 1080, 74_973),
            Mode::new(1280, 720, 60_000),
        ];
        let found = output.find_mode(&Mode::new(1920, 1080, 60_000)).unwrap();
        assert_eq!(found.refresh_mhz, 59_940);
        let highest = output.find_mode(&Mode::new(1920, 1080, 0)).unwrap();
        assert_eq!(highest.refresh_mhz, 74_973);
        assert!(output.find_mode(&Mode::new(1920, 1080, 120_000)).is_none());
    }

    #[test]
    fn mirrored_and_disabled_outputs_are_not_placed() {
        let mut output = Output::new("DP-1");
        output.mode = Some(Mode::new(1920, 1080, 60_000));
        assert!(output.is_placed());
        output.mirror_of = Some("eDP-1".into());
        assert!(!output.is_placed());
        output.mirror_of = None;
        output.enabled = false;
        assert!(output.logical_rect().is_none());
    }

    #[test]
    fn enable_places_output_right_of_the_rest() {
        let mut a = Output::new("A");
        a.mode = Some(Mode::new(1920, 1080, 60_000));
        let mut b = Output::new("B");
        b.enabled = false;
        b.modes = vec![Mode::new(1280, 1024, 60_000)];
        let mut layout = Layout::new(vec![a, b]);
        layout.enable("B");
        let b = layout.get("B").unwrap();
        assert!(b.enabled);
        assert_eq!(b.position, Position::new(1920, 0));
        assert_eq!(b.mode, Some(Mode::new(1280, 1024, 60_000)));
    }
}
