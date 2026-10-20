// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mutter's display model and its translation to and from a [`Layout`].
//!
//! Mutter groups monitors into *logical monitors*: one position, scale,
//! transform and primary flag shared by every monitor in the group, so a
//! group of two is a mirror. Plain data only; unit-tested without D-Bus.
//!
//! In *physical* layout mode (Mutter's default unless the
//! `scale-monitor-framebuffer` feature is on) a logical monitor covers its
//! mode's full pixel size whatever the scale, while Vernier always reasons
//! in logical sizes (pixels divided by scale). [`reflow`] moves positions
//! between the two so outputs that touch in one still touch in the other.

use vnr_core::{BackendError, Layout, Mode, Output, Position, Scale, Transform};

/// How far a requested scale may be from one Mutter supports.
const SCALE_TOLERANCE: f64 = 0.01;

/// One mode of one monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct MMode {
    /// Mutter's mode id, e.g. `1920x1080@60.000`.
    pub id: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Refresh rate in hertz.
    pub refresh: f64,
    /// Scales Mutter accepts with this mode.
    pub scales: Vec<f64>,
    /// In use now.
    pub current: bool,
    /// The monitor's preferred mode.
    pub preferred: bool,
}

impl MMode {
    fn mode(&self) -> Mode {
        let mhz = (self.refresh * 1000.0).round();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "refresh rates are small positive numbers; clamped to 0 when not"
        )]
        let refresh_mhz = if mhz.is_finite() && mhz > 0.0 {
            mhz as u32
        } else {
            0
        };
        Mode {
            width: self.width,
            height: self.height,
            refresh_mhz,
            preferred: self.preferred,
        }
    }

    /// The supported scale closest to `wanted`, if close enough.
    fn scale_for(&self, wanted: Scale) -> Option<f64> {
        let wanted = wanted.as_f64();
        self.scales
            .iter()
            .copied()
            .filter(|s| (s - wanted).abs() <= SCALE_TOLERANCE)
            .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()))
    }
}

/// One physical monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct MMonitor {
    /// Connector, e.g. `DP-1`.
    pub connector: String,
    /// Vendor (PNP id) from the EDID.
    pub vendor: String,
    /// Product name.
    pub product: String,
    /// Serial number.
    pub serial: String,
    /// Human-readable name, e.g. `Built-in display`.
    pub display_name: Option<String>,
    /// Every mode.
    pub modes: Vec<MMode>,
}

/// A group of monitors sharing position, scale, transform and primary flag.
#[derive(Debug, Clone, PartialEq)]
pub struct MLogical {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Scale.
    pub scale: f64,
    /// `wl_output` transform, 0 to 7.
    pub transform: u32,
    /// The primary logical monitor.
    pub primary: bool,
    /// Connectors of the monitors it shows.
    pub monitors: Vec<String>,
}

/// Everything `GetCurrentState` returned.
#[derive(Debug, Clone, PartialEq)]
pub struct MState {
    /// Serial to pass back with a configuration.
    pub serial: u32,
    /// Monitors, connected or not in use.
    pub monitors: Vec<MMonitor>,
    /// Logical monitors in use.
    pub logical: Vec<MLogical>,
    /// Physical (not logical) layout mode.
    pub physical: bool,
    /// Every logical monitor must have the same scale.
    pub global_scale: bool,
    /// Mutter allows several monitors in one logical monitor.
    pub mirroring: bool,
}

/// One logical monitor to send to `ApplyMonitorsConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Left edge, in Mutter's coordinates.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// A scale from the mode's supported list.
    pub scale: f64,
    /// `wl_output` transform.
    pub transform: u32,
    /// Primary flag.
    pub primary: bool,
    /// (connector, mode id) for each monitor.
    pub monitors: Vec<(String, String)>,
}

fn transform_from(value: u32) -> Transform {
    match value {
        1 => Transform::Rotate90,
        2 => Transform::Rotate180,
        3 => Transform::Rotate270,
        4 => Transform::Flipped,
        5 => Transform::Flipped90,
        6 => Transform::Flipped180,
        7 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

fn transform_to(transform: Transform) -> u32 {
    match transform {
        Transform::Normal => 0,
        Transform::Rotate90 => 1,
        Transform::Rotate180 => 2,
        Transform::Rotate270 => 3,
        Transform::Flipped => 4,
        Transform::Flipped90 => 5,
        Transform::Flipped180 => 6,
        Transform::Flipped270 => 7,
    }
}

/// Mutter's size for an output: the rotated mode, divided by the scale only
/// in logical layout mode.
fn mutter_size(output: &Output, physical: bool) -> Option<(u32, u32)> {
    if physical {
        let mode = output.mode?;
        Some(if output.transform.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        })
    } else {
        output.logical_size()
    }
}

/// A width and height.
type Size = (u32, u32);

/// One span on one axis: start and length in the source coordinates, and
/// length in the target coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start in the source coordinates.
    pub start: i32,
    /// Length in the source coordinates.
    pub from_len: u32,
    /// Length in the target coordinates.
    pub to_len: u32,
}

/// Moves span starts from one coordinate system to another so that spans
/// that touch or share an edge keep doing so. Gaps keep their size.
#[must_use]
pub fn reflow(spans: &[Span]) -> Vec<i32> {
    let wide = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|i| spans[*i].start);
    let mut edges: Vec<(i32, i32)> = Vec::new();
    let mut result = vec![0; spans.len()];
    for i in order {
        let span = spans[i];
        let to = edges
            .iter()
            .find(|(from, _)| *from == span.start)
            .map(|(_, to)| *to)
            .or_else(|| {
                edges
                    .iter()
                    .filter(|(from, _)| *from < span.start)
                    .max_by_key(|(from, _)| *from)
                    .map(|(from, to)| to + (span.start - from))
            })
            .unwrap_or(span.start);
        result[i] = to;
        for edge in [
            (span.start, to),
            (
                span.start.saturating_add(wide(span.from_len)),
                to.saturating_add(wide(span.to_len)),
            ),
        ] {
            if !edges.iter().any(|(from, _)| *from == edge.0) {
                edges.push(edge);
            }
        }
    }
    result
}

/// Reflows the positions of the placed outputs in `positions` along both
/// axes; `sizes` gives (source size, target size) per output.
fn reflow_positions(positions: &mut [Position], sizes: &[(Size, Size)]) {
    let xs = reflow(
        &positions
            .iter()
            .zip(sizes)
            .map(|(p, (from, to))| Span {
                start: p.x,
                from_len: from.0,
                to_len: to.0,
            })
            .collect::<Vec<_>>(),
    );
    let ys = reflow(
        &positions
            .iter()
            .zip(sizes)
            .map(|(p, (from, to))| Span {
                start: p.y,
                from_len: from.1,
                to_len: to.1,
            })
            .collect::<Vec<_>>(),
    );
    for ((p, x), y) in positions.iter_mut().zip(xs).zip(ys) {
        *p = Position::new(x, y);
    }
}

impl MState {
    fn monitor(&self, connector: &str) -> Option<&MMonitor> {
        self.monitors.iter().find(|m| m.connector == connector)
    }

    /// The Vernier view of this state.
    #[must_use]
    pub fn to_layout(&self) -> Layout {
        let mut outputs: Vec<Output> = self
            .monitors
            .iter()
            .map(|m| {
                let mut output = Output::new(m.connector.clone());
                let known = |v: &str| (!v.is_empty() && v != "unknown").then(|| v.to_owned());
                output.make = known(&m.vendor);
                output.model = known(&m.product);
                output.serial = known(&m.serial);
                output.description = m
                    .display_name
                    .clone()
                    .unwrap_or_else(|| m.connector.clone());
                output.modes = m.modes.iter().map(MMode::mode).collect();
                output.enabled = false;
                if let Some(logical) = self
                    .logical
                    .iter()
                    .find(|l| l.monitors.contains(&m.connector))
                {
                    output.enabled = true;
                    output.mode = m.modes.iter().find(|x| x.current).map(MMode::mode);
                    output.position = Position::new(logical.x, logical.y);
                    output.transform = transform_from(logical.transform);
                    output.scale = Scale::from_f64(logical.scale).unwrap_or(Scale::ONE);
                    output.mirror_of = logical
                        .monitors
                        .first()
                        .filter(|first| **first != m.connector)
                        .cloned();
                    // The group's primary flag belongs to its source alone.
                    output.primary = logical.primary && output.mirror_of.is_none();
                }
                output
            })
            .collect();
        if self.physical {
            let placed: Vec<usize> = (0..outputs.len())
                .filter(|i| outputs[*i].enabled && outputs[*i].mirror_of.is_none())
                .collect();
            let sizes: Vec<_> = placed
                .iter()
                .map(|i| {
                    let o = &outputs[*i];
                    (
                        mutter_size(o, true).unwrap_or_default(),
                        o.logical_size().unwrap_or_default(),
                    )
                })
                .collect();
            let mut positions: Vec<Position> =
                placed.iter().map(|i| outputs[*i].position).collect();
            reflow_positions(&mut positions, &sizes);
            for (i, p) in placed.iter().zip(positions) {
                outputs[*i].position = p;
            }
            // Mirrors sit on their source.
            for i in 0..outputs.len() {
                if let Some(source) = outputs[i].mirror_of.clone()
                    && let Some(p) = outputs
                        .iter()
                        .find(|o| o.name == source)
                        .map(|o| o.position)
                {
                    outputs[i].position = p;
                }
            }
        }
        Layout::new(outputs)
    }

    /// The logical monitors to send for `desired`.
    ///
    /// # Errors
    ///
    /// [`BackendError::is_cancelled`] for an output that is gone,
    /// [`BackendError::is_unsupported`] for a mode, scale or mirror Mutter
    /// does not offer, [`BackendError::is_rejected`] for a mirror of a
    /// disabled output or nothing enabled.
    pub fn to_request(&self, desired: &Layout) -> Result<Vec<Request>, BackendError> {
        for output in &desired.outputs {
            if self.monitor(&output.name).is_none() {
                return Err(BackendError::cancelled(format!(
                    "`{}` is no longer connected; read the outputs again",
                    output.name
                )));
            }
        }
        let enabled: Vec<&Output> = desired.outputs.iter().filter(|o| o.enabled).collect();
        if enabled.is_empty() {
            return Err(BackendError::rejected(
                "Mutter needs at least one enabled monitor",
            ));
        }
        if !self.mirroring && enabled.iter().any(|o| o.mirror_of.is_some()) {
            return Err(BackendError::unsupported(
                "this Mutter does not allow mirroring",
            ));
        }

        let sources: Vec<&Output> = enabled
            .iter()
            .copied()
            .filter(|o| o.mirror_of.is_none())
            .collect();
        let mut requests = Vec::with_capacity(sources.len());
        for source in &sources {
            let monitor = self
                .monitor(&source.name)
                .unwrap_or_else(|| unreachable!("checked above"));
            let mode = pick_mode(monitor, source, None)?;
            let scale = mode.scale_for(source.scale).ok_or_else(|| {
                let offered: Vec<String> = mode.scales.iter().map(|s| format!("{s:.2}")).collect();
                BackendError::unsupported(format!(
                    "Mutter cannot scale `{}` by {} with mode {}; it offers {}",
                    source.name,
                    source.scale,
                    mode.id,
                    offered.join(", ")
                ))
            })?;
            let mut monitors = vec![(source.name.clone(), mode.id.clone())];
            for mirror in enabled
                .iter()
                .filter(|o| o.mirror_of.as_deref() == Some(source.name.as_str()))
            {
                let m = self
                    .monitor(&mirror.name)
                    .unwrap_or_else(|| unreachable!("checked above"));
                let mirror_mode = pick_mode(m, mirror, Some(mode))?;
                monitors.push((mirror.name.clone(), mirror_mode.id.clone()));
            }
            requests.push(Request {
                x: source.position.x,
                y: source.position.y,
                scale,
                transform: transform_to(source.transform),
                primary: source.primary,
                monitors,
            });
        }
        if let Some(orphan) = enabled.iter().find(|o| {
            o.mirror_of
                .as_deref()
                .is_some_and(|s| !sources.iter().any(|src| src.name == s))
        }) {
            return Err(BackendError::rejected(format!(
                "`{}` mirrors an output that is off or itself a mirror",
                orphan.name
            )));
        }
        if self.global_scale
            && requests
                .windows(2)
                .any(|w| (w[0].scale - w[1].scale).abs() > f64::EPSILON)
        {
            return Err(BackendError::unsupported(
                "Mutter here requires every monitor to use the same scale",
            ));
        }
        if !requests.iter().any(|r| r.primary) {
            requests[0].primary = true;
        }
        if self.physical {
            physical_positions(&sources, &mut requests);
        }
        Ok(requests)
    }
}

/// Moves request positions from Vernier's logical sizes to Mutter's
/// physical ones.
fn physical_positions(sources: &[&Output], requests: &mut [Request]) {
    let sizes: Vec<(Size, Size)> = sources
        .iter()
        .map(|o| {
            (
                o.logical_size().unwrap_or_default(),
                mutter_size(o, true).unwrap_or_default(),
            )
        })
        .collect();
    let mut positions: Vec<Position> = requests.iter().map(|r| Position::new(r.x, r.y)).collect();
    reflow_positions(&mut positions, &sizes);
    for (r, p) in requests.iter_mut().zip(positions) {
        r.x = p.x;
        r.y = p.y;
    }
}

/// The mode for `output`; a mirror prefers its source's resolution.
fn pick_mode<'a>(
    monitor: &'a MMonitor,
    output: &Output,
    source: Option<&MMode>,
) -> Result<&'a MMode, BackendError> {
    let target = output
        .mode
        .or_else(|| output.preferred_mode())
        .ok_or_else(|| BackendError::rejected(format!("`{}` has no mode to show", output.name)))?;
    source
        .and_then(|s| {
            monitor
                .modes
                .iter()
                .find(|m| m.width == s.width && m.height == s.height)
        })
        .or_else(|| monitor.modes.iter().find(|m| m.mode().same_timing(&target)))
        .or_else(|| {
            monitor
                .modes
                .iter()
                .filter(|m| m.mode().approx_eq(&target))
                .min_by_key(|m| m.mode().refresh_mhz.abs_diff(target.refresh_mhz))
        })
        .ok_or_else(|| {
            BackendError::unsupported(format!(
                "`{}` does not offer {target}, and Mutter cannot add modes",
                output.name
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str, w: u32, h: u32, current: bool) -> MMode {
        MMode {
            id: id.into(),
            width: w,
            height: h,
            refresh: 60.0,
            scales: vec![1.0, 1.25, 1.5, 2.0],
            current,
            preferred: current,
        }
    }

    fn monitor(connector: &str, modes: Vec<MMode>) -> MMonitor {
        MMonitor {
            connector: connector.into(),
            vendor: "AUO".into(),
            product: "0x323D".into(),
            serial: "0x00000000".into(),
            display_name: Some(format!("Display {connector}")),
            modes,
        }
    }

    fn logical(x: i32, scale: f64, primary: bool, monitors: &[&str]) -> MLogical {
        MLogical {
            x,
            y: 0,
            scale,
            transform: 0,
            primary,
            monitors: monitors.iter().map(|m| (*m).to_owned()).collect(),
        }
    }

    /// eDP-1 1920x1080 and DP-1 2560x1440, side by side, both scale 1.
    fn state(physical: bool) -> MState {
        MState {
            serial: 7,
            monitors: vec![
                monitor("eDP-1", vec![mode("1920x1080@60", 1920, 1080, true)]),
                monitor(
                    "DP-1",
                    vec![
                        mode("2560x1440@60", 2560, 1440, true),
                        mode("1920x1080@60", 1920, 1080, false),
                    ],
                ),
            ],
            logical: vec![
                logical(0, 1.0, true, &["eDP-1"]),
                logical(1920, 1.0, false, &["DP-1"]),
            ],
            physical,
            global_scale: false,
            mirroring: true,
        }
    }

    #[test]
    fn reads_logical_monitors() {
        let layout = state(false).to_layout();
        let dp = layout.get("DP-1").unwrap();
        assert!(dp.enabled);
        assert_eq!(dp.position, Position::new(1920, 0));
        assert_eq!(
            dp.mode.map(|m| (m.width, m.refresh_mhz)),
            Some((2560, 60_000))
        );
        assert!(layout.get("eDP-1").unwrap().primary);
        assert_eq!(layout.get("eDP-1").unwrap().make.as_deref(), Some("AUO"));
    }

    #[test]
    fn round_trip_requests_match_state() {
        for physical in [false, true] {
            let s = state(physical);
            let requests = s.to_request(&s.to_layout()).unwrap();
            let got: Vec<(i32, bool, Vec<String>)> = requests
                .iter()
                .map(|r| {
                    (
                        r.x,
                        r.primary,
                        r.monitors.iter().map(|m| m.1.clone()).collect(),
                    )
                })
                .collect();
            assert_eq!(
                got,
                [
                    (0, true, vec!["1920x1080@60".to_owned()]),
                    (1920, false, vec!["2560x1440@60".to_owned()])
                ]
            );
        }
    }

    #[test]
    fn mirrors_become_one_logical_monitor() {
        let s = state(false);
        let mut layout = s.to_layout();
        layout.outputs[1].mirror_of = Some("eDP-1".into());
        let requests = s.to_request(&layout).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].monitors,
            [
                ("eDP-1".to_owned(), "1920x1080@60".to_owned()),
                ("DP-1".to_owned(), "1920x1080@60".to_owned())
            ]
        );
        let mut grouped = s.clone();
        grouped.logical = vec![logical(0, 1.0, true, &["eDP-1", "DP-1"])];
        let read = grouped.to_layout();
        assert_eq!(
            read.get("DP-1").unwrap().mirror_of.as_deref(),
            Some("eDP-1")
        );
        assert!(read.get("eDP-1").unwrap().primary && !read.get("DP-1").unwrap().primary);
    }

    #[test]
    fn physical_mode_keeps_scaled_outputs_adjacent() {
        // In physical mode a scale-2 eDP-1 still covers 1920 pixels, so
        // DP-1 sits at 1920 for Mutter but at 960 for Vernier.
        let mut s = state(true);
        s.logical[0].scale = 2.0;
        let layout = s.to_layout();
        assert_eq!(layout.get("DP-1").unwrap().position, Position::new(960, 0));
        let requests = s.to_request(&layout).unwrap();
        assert_eq!(requests[1].x, 1920);
    }

    #[test]
    fn scales_snap_to_supported_values() {
        let s = state(false);
        let mut layout = s.to_layout();
        layout.outputs[0].scale = Scale::from_millis(1250).unwrap();
        assert!((s.to_request(&layout).unwrap()[0].scale - 1.25).abs() < 1e-9);
        layout.outputs[0].scale = Scale::from_millis(1100).unwrap();
        assert!(s.to_request(&layout).unwrap_err().is_unsupported());
    }

    #[test]
    fn refusals() {
        let s = state(false);
        let mut layout = s.to_layout();
        layout.outputs[0].mode = Some(Mode::new(800, 600, 60_000));
        assert!(s.to_request(&layout).unwrap_err().is_unsupported());
        let mut layout = s.to_layout();
        layout.outputs.push(Output::new("HDMI-9"));
        assert!(s.to_request(&layout).unwrap_err().is_cancelled());
        let mut layout = s.to_layout();
        layout.outputs.iter_mut().for_each(|o| o.enabled = false);
        assert!(s.to_request(&layout).unwrap_err().is_rejected());
        let mut layout = s.to_layout();
        layout.outputs[0].enabled = false;
        layout.outputs[1].mirror_of = Some("eDP-1".into());
        assert!(s.to_request(&layout).unwrap_err().is_rejected());
    }

    #[test]
    fn reflow_keeps_touching_spans_touching() {
        let spans = [
            Span {
                start: 0,
                from_len: 960,
                to_len: 1920,
            },
            Span {
                start: 960,
                from_len: 2560,
                to_len: 2560,
            },
            Span {
                start: 3620,
                from_len: 100,
                to_len: 100,
            },
        ];
        assert_eq!(reflow(&spans), [0, 1920, 4580]);
    }
}
