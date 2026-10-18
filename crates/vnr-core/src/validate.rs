// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks a layout before it is sent to a backend.

use std::fmt;

use serde::Serialize;

use crate::model::{Capabilities, Layout};

/// How serious an [`Issue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Allowed, but probably not what the user wants.
    Warning,
    /// The backend would reject it, or the result would be unusable.
    Error,
}

/// A problem found in a layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "issue", rename_all = "kebab-case")]
pub enum Issue {
    /// Every output is disabled; the session would have no screen.
    NoEnabledOutput,
    /// An enabled output has no mode.
    NoMode {
        /// The output.
        output: String,
    },
    /// The mode is not one the display advertises.
    UnsupportedMode {
        /// The output.
        output: String,
        /// The requested mode, formatted.
        mode: String,
        /// Whether the backend accepts custom modes (then this is a warning).
        custom_allowed: bool,
    },
    /// Two outputs cover the same area.
    Overlap {
        /// First output.
        a: String,
        /// Second output.
        b: String,
    },
    /// An output shares no edge with any other, so the pointer cannot reach it.
    Detached {
        /// The output.
        output: String,
    },
    /// Mirroring was requested but the backend cannot mirror.
    MirrorUnsupported {
        /// The output.
        output: String,
    },
    /// The mirror source is unknown, disabled, itself a mirror, or the output itself.
    MirrorSourceInvalid {
        /// The output.
        output: String,
        /// The requested source.
        source: String,
    },
    /// More than one output is marked primary.
    MultiplePrimary,
}

impl Issue {
    /// How serious this issue is.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::Overlap { .. } | Self::Detached { .. } => Severity::Warning,
            Self::UnsupportedMode { custom_allowed, .. } if *custom_allowed => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEnabledOutput => f.write_str("every output is disabled"),
            Self::NoMode { output } => write!(f, "{output} is enabled but has no mode"),
            Self::UnsupportedMode { output, mode, .. } => {
                write!(f, "{output} does not advertise mode {mode}")
            }
            Self::Overlap { a, b } => write!(f, "{a} and {b} overlap"),
            Self::Detached { output } => {
                write!(f, "{output} does not touch any other output")
            }
            Self::MirrorUnsupported { output } => {
                write!(
                    f,
                    "{output} mirrors another output, which this backend cannot do"
                )
            }
            Self::MirrorSourceInvalid { output, source } => {
                write!(f, "{output} cannot mirror {source}")
            }
            Self::MultiplePrimary => f.write_str("more than one output is marked primary"),
        }
    }
}

/// Lists every problem in `layout` for a backend with `caps`.
///
/// An empty result means the layout is safe to send. Errors should block
/// apply; warnings should be shown.
#[must_use]
pub fn validate(layout: &Layout, caps: &Capabilities) -> Vec<Issue> {
    let mut issues = Vec::new();

    if !layout.outputs.iter().any(|o| o.enabled) {
        issues.push(Issue::NoEnabledOutput);
    }

    for output in layout.outputs.iter().filter(|o| o.enabled) {
        match output.mode {
            None => issues.push(Issue::NoMode {
                output: output.name.clone(),
            }),
            Some(mode) if !output.modes.is_empty() && output.find_mode(&mode).is_none() => {
                issues.push(Issue::UnsupportedMode {
                    output: output.name.clone(),
                    mode: mode.to_string(),
                    custom_allowed: caps.custom_modes,
                });
            }
            Some(_) => {}
        }

        if let Some(source) = &output.mirror_of {
            if !caps.mirroring {
                issues.push(Issue::MirrorUnsupported {
                    output: output.name.clone(),
                });
            }
            let valid = *source != output.name
                && layout
                    .get(source)
                    .is_some_and(|s| s.enabled && s.mirror_of.is_none());
            if !valid {
                issues.push(Issue::MirrorSourceInvalid {
                    output: output.name.clone(),
                    source: source.clone(),
                });
            }
        }
    }

    let placed: Vec<_> = layout.placed_rects().collect();
    for (n, (i, a)) in placed.iter().enumerate() {
        for (j, b) in &placed[n + 1..] {
            if a.intersects(b) {
                issues.push(Issue::Overlap {
                    a: layout.outputs[*i].name.clone(),
                    b: layout.outputs[*j].name.clone(),
                });
            }
        }
    }
    if placed.len() > 1 {
        for (i, a) in &placed {
            if !placed.iter().any(|(j, b)| i != j && a.touches(b)) {
                issues.push(Issue::Detached {
                    output: layout.outputs[*i].name.clone(),
                });
            }
        }
    }

    if layout
        .outputs
        .iter()
        .filter(|o| o.enabled && o.primary)
        .count()
        > 1
    {
        issues.push(Issue::MultiplePrimary);
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Output, Position};

    fn output(name: &str, x: i32) -> Output {
        let mut o = Output::new(name);
        o.modes = vec![Mode::new(1920, 1080, 60_000)];
        o.mode = o.modes.first().copied();
        o.position = Position::new(x, 0);
        o
    }

    #[test]
    fn side_by_side_is_clean() {
        let layout = Layout::new(vec![output("A", 0), output("B", 1920)]);
        assert!(validate(&layout, &Capabilities::default()).is_empty());
    }

    #[test]
    fn overlap_and_gap_are_warnings() {
        let overlap = Layout::new(vec![output("A", 0), output("B", 1000)]);
        let issues = validate(&overlap, &Capabilities::default());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity(), Severity::Warning);

        let gap = Layout::new(vec![output("A", 0), output("B", 2000)]);
        let issues = validate(&gap, &Capabilities::default());
        assert_eq!(issues.len(), 2, "both outputs are detached: {issues:?}");
    }

    #[test]
    fn all_disabled_is_an_error() {
        let mut a = output("A", 0);
        a.enabled = false;
        let issues = validate(&Layout::new(vec![a]), &Capabilities::default());
        assert_eq!(issues, vec![Issue::NoEnabledOutput]);
    }

    #[test]
    fn unknown_mode_depends_on_custom_mode_support() {
        let mut a = output("A", 0);
        a.mode = Some(Mode::new(1600, 900, 60_000));
        let layout = Layout::new(vec![a]);
        let strict = validate(&layout, &Capabilities::default());
        assert_eq!(strict[0].severity(), Severity::Error);
        let caps = Capabilities {
            custom_modes: true,
            ..Capabilities::default()
        };
        assert_eq!(validate(&layout, &caps)[0].severity(), Severity::Warning);
    }

    #[test]
    fn mirror_rules() {
        let mut b = output("B", 0);
        b.mirror_of = Some("A".into());
        let layout = Layout::new(vec![output("A", 0), b.clone()]);
        let caps = Capabilities {
            mirroring: true,
            ..Capabilities::default()
        };
        assert!(
            validate(&layout, &caps).is_empty(),
            "a mirror is not placed, so no overlap"
        );
        assert!(matches!(
            validate(&layout, &Capabilities::default())[0],
            Issue::MirrorUnsupported { .. }
        ));

        b.mirror_of = Some("B".into());
        let self_mirror = Layout::new(vec![output("A", 0), b]);
        assert!(
            validate(&self_mirror, &caps)
                .iter()
                .any(|i| matches!(i, Issue::MirrorSourceInvalid { .. }))
        );
    }
}
