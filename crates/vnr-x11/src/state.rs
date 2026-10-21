// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `RandR` state Vernier reads, and the plan that turns a desired
//! [`Layout`] into `RandR` requests.
//!
//! Everything here is plain data with no connection, so the translation
//! both ways is unit-tested without an X server.

use vnr_core::{BackendError, Layout, Mode, Output, Position, Scale, Transform};

use crate::edid::Identity;

/// `RandR` rotation and reflection bits (`RRRotate_0` ... `RRReflect_Y`).
pub mod rotation {
    /// No rotation.
    pub const ROTATE_0: u16 = 1;
    /// 90 degrees counter-clockwise.
    pub const ROTATE_90: u16 = 2;
    /// 180 degrees.
    pub const ROTATE_180: u16 = 4;
    /// 270 degrees counter-clockwise.
    pub const ROTATE_270: u16 = 8;
    /// Mirror left-right.
    pub const REFLECT_X: u16 = 16;
    /// Mirror top-bottom.
    pub const REFLECT_Y: u16 = 32;
}

/// One `RandR` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XMode {
    /// Mode XID.
    pub id: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Refresh rate in millihertz; zero when the timings do not give one.
    pub refresh_mhz: u32,
}

impl XMode {
    /// Computes the refresh rate the way `xrandr` does: dot clock over the
    /// total frame, doubled for interlaced and halved for double-scan modes.
    #[must_use]
    pub fn refresh_mhz(
        dot_clock: u32,
        htotal: u16,
        vtotal: u16,
        interlace: bool,
        double_scan: bool,
    ) -> u32 {
        // Interlaced modes show two fields per frame; double-scan modes
        // send each line twice. Kept in integers to avoid rounding.
        let fields = if interlace { 2 } else { 1 };
        let scans = if double_scan { 2 } else { 1 };
        let frame = u64::from(htotal) * u64::from(vtotal) * scans;
        if frame == 0 {
            return 0;
        }
        let mhz = (u64::from(dot_clock) * 1000 * fields + frame / 2) / frame;
        u32::try_from(mhz).unwrap_or(0)
    }

    fn mode(self, preferred: bool) -> Mode {
        Mode {
            width: self.width,
            height: self.height,
            refresh_mhz: self.refresh_mhz,
            preferred,
        }
    }
}

/// One CRTC (scan-out engine) and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XCrtc {
    /// CRTC XID.
    pub id: u32,
    /// Left edge in screen pixels.
    pub x: i32,
    /// Top edge in screen pixels.
    pub y: i32,
    /// Width in screen pixels, after rotation and transform.
    pub width: u32,
    /// Height in screen pixels.
    pub height: u32,
    /// Mode XID; zero when off.
    pub mode: u32,
    /// Rotation and reflection bits.
    pub rotation: u16,
    /// Outputs it drives.
    pub outputs: Vec<u32>,
    /// Scale, read back from the CRTC transform.
    pub scale: Scale,
    /// Whether the driver supports CRTC transforms (needed for scaling).
    pub has_transforms: bool,
    /// Rotation and reflection bits the CRTC supports.
    pub rotations: u16,
}

/// One `RandR` output (connector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XOutput {
    /// Output XID.
    pub id: u32,
    /// Connector name, e.g. `DP-1`.
    pub name: String,
    /// Whether a monitor is attached.
    pub connected: bool,
    /// The CRTC driving it; zero when none.
    pub crtc: u32,
    /// CRTCs that can drive it.
    pub crtcs: Vec<u32>,
    /// Mode XIDs it supports; the first `preferred` are preferred.
    pub modes: Vec<u32>,
    /// How many leading entries of `modes` are preferred.
    pub preferred: usize,
    /// Physical size in millimetres; zero when unknown.
    pub mm: (u32, u32),
    /// From the EDID.
    pub identity: Identity,
}

/// Everything read from the server in one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// Screen size in pixels.
    pub size: (u32, u32),
    /// Screen size in millimetres.
    pub mm: (u32, u32),
    /// Smallest screen size the server accepts.
    pub min: (u32, u32),
    /// Largest screen size the server accepts.
    pub max: (u32, u32),
    /// Outputs in server order.
    pub outputs: Vec<XOutput>,
    /// CRTCs in server order.
    pub crtcs: Vec<XCrtc>,
    /// Every mode the server knows.
    pub modes: Vec<XMode>,
    /// Primary output XID; zero when none.
    pub primary: u32,
}

impl Server {
    fn crtc(&self, id: u32) -> Option<&XCrtc> {
        self.crtcs.iter().find(|c| c.id == id)
    }

    fn mode(&self, id: u32) -> Option<XMode> {
        self.modes.iter().copied().find(|m| m.id == id)
    }

    /// Whether every CRTC can scale.
    #[must_use]
    pub fn can_scale(&self) -> bool {
        !self.crtcs.is_empty() && self.crtcs.iter().all(|c| c.has_transforms)
    }

    /// The Vernier view of this state. Disconnected outputs are left out.
    #[must_use]
    pub fn to_layout(&self) -> Layout {
        let mut outputs: Vec<Output> = Vec::new();
        for x in self.outputs.iter().filter(|o| o.connected) {
            let mut output = Output::new(x.name.clone());
            output.make.clone_from(&x.identity.make);
            output.model.clone_from(&x.identity.model);
            output.serial.clone_from(&x.identity.serial);
            output.description = match (&x.identity.make, &x.identity.model) {
                (Some(make), Some(model)) => format!("{make} {model} ({})", x.name),
                _ => x.name.clone(),
            };
            output.physical_size_mm = (x.mm.0 > 0 && x.mm.1 > 0).then_some(x.mm);
            output.modes = x
                .modes
                .iter()
                .enumerate()
                .filter_map(|(i, id)| self.mode(*id).map(|m| m.mode(i < x.preferred)))
                .collect();
            output.primary = x.id == self.primary;
            match self.crtc(x.crtc).filter(|c| c.mode != 0) {
                Some(crtc) => {
                    output.enabled = true;
                    output.mode = self.mode(crtc.mode).map(|m| {
                        let preferred = output
                            .modes
                            .iter()
                            .any(|o| o.preferred && o.same_timing(&m.mode(true)));
                        m.mode(preferred)
                    });
                    output.position = Position::new(crtc.x, crtc.y);
                    output.transform = transform_from_bits(crtc.rotation);
                    output.scale = crtc.scale;
                    // An earlier output covering exactly the same area is
                    // the source of a mirror (`xrandr --same-as`).
                    output.mirror_of = outputs
                        .iter()
                        .find(|o| {
                            o.enabled
                                && o.mirror_of.is_none()
                                && o.logical_rect().is_some()
                                && o.logical_rect() == output.logical_rect()
                        })
                        .map(|o| o.name.clone());
                }
                None => output.enabled = false,
            }
            outputs.push(output);
        }
        Layout::new(outputs)
    }
}

/// `RandR` bits for a transform. Vernier's `flipped-N` (flip, then rotate)
/// is `RandR`'s reflect-X combined with rotate-N.
#[must_use]
pub fn bits_from_transform(transform: Transform) -> u16 {
    use rotation::{REFLECT_X, ROTATE_0, ROTATE_90, ROTATE_180, ROTATE_270};
    match transform {
        Transform::Normal => ROTATE_0,
        Transform::Rotate90 => ROTATE_90,
        Transform::Rotate180 => ROTATE_180,
        Transform::Rotate270 => ROTATE_270,
        Transform::Flipped => ROTATE_0 | REFLECT_X,
        Transform::Flipped90 => ROTATE_90 | REFLECT_X,
        Transform::Flipped180 => ROTATE_180 | REFLECT_X,
        Transform::Flipped270 => ROTATE_270 | REFLECT_X,
    }
}

/// The transform for `RandR` bits. A top-bottom reflection is a left-right
/// one turned 180 degrees, and both together are a plain 180 degree turn.
#[must_use]
pub fn transform_from_bits(bits: u16) -> Transform {
    use rotation::{REFLECT_X, REFLECT_Y, ROTATE_90, ROTATE_180, ROTATE_270};
    let mut quarter = if bits & ROTATE_90 != 0 {
        1
    } else if bits & ROTATE_180 != 0 {
        2
    } else if bits & ROTATE_270 != 0 {
        3
    } else {
        0
    };
    let mut flipped = bits & REFLECT_X != 0;
    if bits & REFLECT_Y != 0 {
        flipped = !flipped;
        quarter = (quarter + 2) % 4;
    }
    match (flipped, quarter) {
        (false, 0) => Transform::Normal,
        (false, 1) => Transform::Rotate90,
        (false, 2) => Transform::Rotate180,
        (false, _) => Transform::Rotate270,
        (true, 0) => Transform::Flipped,
        (true, 1) => Transform::Flipped90,
        (true, 2) => Transform::Flipped180,
        (true, _) => Transform::Flipped270,
    }
}

/// What one CRTC should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrtcConfig {
    /// CRTC XID.
    pub crtc: u32,
    /// Left edge in screen pixels.
    pub x: i32,
    /// Top edge in screen pixels.
    pub y: i32,
    /// Mode XID.
    pub mode: u32,
    /// Rotation bits.
    pub rotation: u16,
    /// Outputs to drive.
    pub outputs: Vec<u32>,
    /// Scale; anything but 1 needs a CRTC transform.
    pub scale: Scale,
    /// Extent in screen pixels, for sizing the screen.
    pub extent: (u32, u32),
}

/// The requests that turn the current state into the desired one, in the
/// order they must be sent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// CRTCs to switch off first: those no output uses any more, and those
    /// that would not fit while the screen is resized.
    pub disable: Vec<u32>,
    /// New screen size in pixels and millimetres, when it changes.
    pub screen: Option<((u32, u32), (u32, u32))>,
    /// CRTCs to configure, after the resize.
    pub configure: Vec<CrtcConfig>,
    /// New primary output XID (zero for none), when it changes.
    pub primary: Option<u32>,
}

impl Plan {
    /// Whether the plan sends nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.disable.is_empty()
            && self.screen.is_none()
            && self.configure.is_empty()
            && self.primary.is_none()
    }
}

/// Plans the requests that turn `server` into `desired`.
///
/// # Errors
///
/// Returns [`BackendError::is_cancelled`] if `desired` names an output that
/// is no longer connected, [`BackendError::is_unsupported`] for a mode the
/// output does not advertise or a scale the driver cannot do, and
/// [`BackendError::is_rejected`] when CRTCs run out or the screen would be
/// too large.
pub fn plan(server: &Server, desired: &Layout) -> Result<Plan, BackendError> {
    for output in &desired.outputs {
        if !server
            .outputs
            .iter()
            .any(|x| x.connected && x.name == output.name)
        {
            return Err(BackendError::cancelled(format!(
                "`{}` is no longer connected; read the outputs again",
                output.name
            )));
        }
    }
    let wanted = resolve(server, desired)?;
    let crtcs = assign(&wanted)?;
    let configs: Vec<CrtcConfig> = wanted
        .iter()
        .zip(crtcs)
        .map(|(want, crtc)| want.config(desired, crtc))
        .collect();
    for (config, want) in configs.iter().zip(&wanted) {
        let supported = server.crtc(config.crtc).map_or(0, |c| c.rotations);
        if config.rotation & supported != config.rotation {
            return Err(BackendError::unsupported(format!(
                "the X driver cannot show `{}` with rotation `{}`",
                want.x.name, want.output.transform
            )));
        }
    }
    let (width, height) = screen_size(server, &configs)?;

    let mut plan = Plan::default();
    if (width, height) != server.size {
        plan.screen = Some(((width, height), keep_dpi(server, width, height)));
    }
    for crtc in &server.crtcs {
        let target = configs.iter().find(|c| c.crtc == crtc.id);
        if target.is_some_and(|c| c.matches(crtc)) {
            continue;
        }
        let on = crtc.mode != 0;
        let fits = i64::from(crtc.x) + i64::from(crtc.width) <= i64::from(width)
            && i64::from(crtc.y) + i64::from(crtc.height) <= i64::from(height);
        match target {
            None if on => plan.disable.push(crtc.id),
            None => {}
            Some(config) => {
                // A CRTC that would hang off the resized screen goes dark
                // until it is configured again.
                if on && !fits {
                    plan.disable.push(crtc.id);
                }
                plan.configure.push(config.clone());
            }
        }
    }
    plan.primary = primary(server, desired);
    Ok(plan)
}

/// An enabled output with the `RandR` mode it will show.
struct Wanted<'a> {
    x: &'a XOutput,
    output: &'a Output,
    mode: XMode,
}

impl Wanted<'_> {
    fn config(&self, desired: &Layout, crtc: u32) -> CrtcConfig {
        let source = self
            .output
            .mirror_of
            .as_deref()
            .and_then(|s| desired.get(s));
        let position = source.map_or(self.output.position, |s| s.position);
        let mut shown = Output::clone(self.output);
        shown.mode = Some(self.mode.mode(false));
        CrtcConfig {
            crtc,
            x: position.x,
            y: position.y,
            mode: self.mode.id,
            rotation: bits_from_transform(self.output.transform),
            outputs: vec![self.x.id],
            scale: self.output.scale,
            extent: shown
                .logical_size()
                .unwrap_or((self.mode.width, self.mode.height)),
        }
    }
}

impl CrtcConfig {
    fn matches(&self, crtc: &XCrtc) -> bool {
        crtc.mode == self.mode
            && crtc.x == self.x
            && crtc.y == self.y
            && crtc.rotation == self.rotation
            && crtc.scale == self.scale
            && crtc.outputs == self.outputs
    }
}

/// Finds the `RandR` mode for every enabled output.
fn resolve<'a>(server: &'a Server, desired: &'a Layout) -> Result<Vec<Wanted<'a>>, BackendError> {
    let mut wanted = Vec::new();
    for x in server.outputs.iter().filter(|o| o.connected) {
        let Some(output) = desired.get(&x.name).filter(|o| o.enabled) else {
            continue;
        };
        let source = output.mirror_of.as_deref().and_then(|s| desired.get(s));
        let target = output
            .mode
            .or_else(|| output.preferred_mode())
            .ok_or_else(|| BackendError::rejected(format!("`{}` has no mode to show", x.name)))?;
        let modes = || x.modes.iter().filter_map(|id| server.mode(*id));
        // A mirror shows its source's resolution when it can.
        let mode = source
            .and_then(|s| s.mode)
            .and_then(|s| modes().find(|m| m.width == s.width && m.height == s.height))
            .or_else(|| modes().find(|m| m.mode(false).approx_eq(&target)))
            .ok_or_else(|| {
                BackendError::unsupported(format!(
                    "`{}` does not advertise {target}, and adding modes on X11 is not supported yet",
                    x.name
                ))
            })?;
        if output.scale != Scale::ONE && !server.can_scale() {
            return Err(BackendError::unsupported(format!(
                "this X driver cannot scale outputs (no CRTC transforms); `{}` must stay at scale 1",
                x.name
            )));
        }
        wanted.push(Wanted { x, output, mode });
    }
    Ok(wanted)
}

/// Keeps each output on its current CRTC when possible, so unchanged
/// outputs are left alone, and gives the rest the first free one.
fn assign(wanted: &[Wanted<'_>]) -> Result<Vec<u32>, BackendError> {
    let mut assigned: Vec<Option<u32>> = vec![None; wanted.len()];
    let mut taken: Vec<u32> = Vec::new();
    for (slot, want) in assigned.iter_mut().zip(wanted) {
        let current = want.x.crtc;
        if current != 0 && want.x.crtcs.contains(&current) && !taken.contains(&current) {
            *slot = Some(current);
            taken.push(current);
        }
    }
    assigned
        .into_iter()
        .zip(wanted)
        .map(|(slot, want)| {
            if let Some(crtc) = slot {
                return Ok(crtc);
            }
            let free = want
                .x
                .crtcs
                .iter()
                .copied()
                .find(|c| !taken.contains(c))
                .ok_or_else(|| {
                    BackendError::rejected(format!(
                        "no free CRTC can drive `{}`; turn another output off first",
                        want.x.name
                    ))
                })?;
            taken.push(free);
            Ok(free)
        })
        .collect()
}

/// The bounding box of every CRTC, within the server's limits.
fn screen_size(server: &Server, configs: &[CrtcConfig]) -> Result<(u32, u32), BackendError> {
    if configs.iter().any(|c| c.x < 0 || c.y < 0) {
        return Err(BackendError::rejected(
            "X11 screen coordinates cannot be negative",
        ));
    }
    let right = |c: &CrtcConfig| i64::from(c.x) + i64::from(c.extent.0);
    let bottom = |c: &CrtcConfig| i64::from(c.y) + i64::from(c.extent.1);
    let clamp = |v: i64, min: u32| u32::try_from(v).unwrap_or(u32::MAX).max(min);
    let width = clamp(configs.iter().map(right).max().unwrap_or(0), server.min.0);
    let height = clamp(configs.iter().map(bottom).max().unwrap_or(0), server.min.1);
    if width > server.max.0 || height > server.max.1 {
        return Err(BackendError::rejected(format!(
            "the layout needs a {width}x{height} screen, but this X server allows at most {}x{}",
            server.max.0, server.max.1
        )));
    }
    Ok((width, height))
}

/// The new primary output, when it changes.
fn primary(server: &Server, desired: &Layout) -> Option<u32> {
    let wanted = server
        .outputs
        .iter()
        .find(|x| desired.get(&x.name).is_some_and(|o| o.primary && o.enabled))
        .map_or(0, |x| x.id);
    let any = desired.outputs.iter().any(|o| o.primary);
    (wanted != server.primary && (any || server.primary != 0)).then_some(wanted)
}

/// Millimetre size for a new pixel size at the current DPI (96 when the
/// server reports none).
fn keep_dpi(server: &Server, width: u32, height: u32) -> (u32, u32) {
    let scale = |px: u32, cur_px: u32, cur_mm: u32| {
        let mm = if cur_px > 0 && cur_mm > 0 {
            u64::from(px) * u64::from(cur_mm) / u64::from(cur_px)
        } else {
            u64::from(px) * 254 / 960
        };
        u32::try_from(mm).unwrap_or(u32::MAX).max(1)
    };
    (
        scale(width, server.size.0, server.mm.0),
        scale(height, server.size.1, server.mm.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: u32 = 0x40;
    const HD: u32 = 0x41;
    const QHD: u32 = 0x42;

    fn crtc(id: u32, x: i32, mode: u32, outputs: Vec<u32>) -> XCrtc {
        let (width, height) = match mode {
            FHD => (1920, 1080),
            QHD => (2560, 1440),
            HD => (1280, 720),
            _ => (0, 0),
        };
        XCrtc {
            id,
            x,
            y: 0,
            width,
            height,
            mode,
            rotation: rotation::ROTATE_0,
            outputs,
            scale: Scale::ONE,
            has_transforms: true,
            rotations: 0x3F,
        }
    }

    fn output(id: u32, name: &str, crtc: u32, modes: Vec<u32>) -> XOutput {
        XOutput {
            id,
            name: name.into(),
            connected: true,
            crtc,
            crtcs: vec![0x10, 0x11, 0x12],
            modes,
            preferred: 1,
            mm: (0, 0),
            identity: Identity::default(),
        }
    }

    /// eDP-1 at 1920x1080 on CRTC 0x10; DP-1 at 2560x1440 right of it on
    /// CRTC 0x11; HDMI-1 disconnected.
    fn server() -> Server {
        let mut hdmi = output(3, "HDMI-1", 0, vec![]);
        hdmi.connected = false;
        Server {
            size: (4480, 1440),
            mm: (1185, 381),
            min: (320, 200),
            max: (8192, 8192),
            outputs: vec![
                output(1, "eDP-1", 0x10, vec![FHD, HD]),
                output(2, "DP-1", 0x11, vec![QHD, FHD]),
                hdmi,
            ],
            crtcs: vec![
                crtc(0x10, 0, FHD, vec![1]),
                crtc(0x11, 1920, QHD, vec![2]),
                crtc(0x12, 0, 0, vec![]),
            ],
            modes: vec![
                XMode {
                    id: FHD,
                    width: 1920,
                    height: 1080,
                    refresh_mhz: 60_000,
                },
                XMode {
                    id: HD,
                    width: 1280,
                    height: 720,
                    refresh_mhz: 60_000,
                },
                XMode {
                    id: QHD,
                    width: 2560,
                    height: 1440,
                    refresh_mhz: 59_951,
                },
            ],
            primary: 1,
        }
    }

    #[test]
    fn refresh_matches_xrandr() {
        // CEA 1080p60: 148.5 MHz over 2200 x 1125.
        assert_eq!(
            XMode::refresh_mhz(148_500_000, 2200, 1125, false, false),
            60_000
        );
        assert_eq!(
            XMode::refresh_mhz(74_250_000, 2200, 1125, true, false),
            60_000
        );
        assert_eq!(XMode::refresh_mhz(1, 0, 0, false, false), 0);
    }

    #[test]
    fn layout_reads_connected_outputs() {
        let layout = server().to_layout();
        let names: Vec<&str> = layout.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["eDP-1", "DP-1"]);
        let dp = layout.get("DP-1").unwrap();
        assert_eq!(dp.position, Position::new(1920, 0));
        assert_eq!(
            dp.mode,
            Some(Mode {
                width: 2560,
                height: 1440,
                refresh_mhz: 59_951,
                preferred: true
            })
        );
        assert_eq!(dp.modes.len(), 2);
        assert!(layout.get("eDP-1").unwrap().primary);
    }

    #[test]
    fn same_area_reads_as_mirror() {
        let mut s = server();
        s.crtcs[1] = crtc(0x11, 0, FHD, vec![2]);
        let layout = s.to_layout();
        assert_eq!(
            layout.get("DP-1").unwrap().mirror_of.as_deref(),
            Some("eDP-1")
        );
        assert_eq!(layout.get("eDP-1").unwrap().mirror_of, None);
    }

    #[test]
    fn unchanged_layout_plans_nothing() {
        let s = server();
        assert!(plan(&s, &s.to_layout()).unwrap().is_empty());
    }

    #[test]
    fn moving_left_reconfigures_both_and_keeps_screen() {
        let s = server();
        let mut layout = s.to_layout();
        layout.outputs[1].position = Position::new(0, 0);
        layout.outputs[0].position = Position::new(2560, 0);
        let p = plan(&s, &layout).unwrap();
        assert_eq!(p.screen, None);
        assert_eq!(p.configure.len(), 2);
        // eDP-1 at 2560..4480 fits the unchanged 4480 x 1440 screen.
        assert_eq!(p.disable, []);
    }

    #[test]
    fn disabling_frees_the_crtc_and_shrinks_the_screen() {
        let s = server();
        let mut layout = s.to_layout();
        layout.outputs[1].enabled = false;
        let p = plan(&s, &layout).unwrap();
        assert_eq!(p.disable, [0x11]);
        assert_eq!(p.screen.map(|(px, _)| px), Some((1920, 1080)));
        assert_eq!(p.configure, []);
    }

    #[test]
    fn scale_and_rotation_set_the_extent() {
        let s = server();
        let mut layout = s.to_layout();
        let dp = &mut layout.outputs[1];
        dp.transform = Transform::Rotate90;
        dp.scale = Scale::from_millis(2000).unwrap();
        let p = plan(&s, &layout).unwrap();
        let config = p.configure.iter().find(|c| c.crtc == 0x11).unwrap();
        assert_eq!(config.extent, (720, 1280));
        assert_eq!(config.rotation, rotation::ROTATE_90);
        assert_eq!(p.screen.map(|(px, _)| px), Some((2640, 1280)));
        // The old 2560-wide CRTC does not fit the narrower screen.
        assert_eq!(p.disable, [0x11]);
    }

    #[test]
    fn mirrors_take_the_source_position_and_resolution() {
        let s = server();
        let mut layout = s.to_layout();
        layout.outputs[1].mirror_of = Some("eDP-1".into());
        let p = plan(&s, &layout).unwrap();
        let config = p.configure.iter().find(|c| c.crtc == 0x11).unwrap();
        assert_eq!((config.x, config.y, config.mode), (0, 0, FHD));
    }

    #[test]
    fn refusals_have_the_right_kind() {
        let s = server();
        let mut layout = s.to_layout();
        layout.outputs[0].mode = Some(Mode::new(800, 600, 60_000));
        assert!(plan(&s, &layout).unwrap_err().is_unsupported());

        let mut no_transforms = server();
        no_transforms
            .crtcs
            .iter_mut()
            .for_each(|c| c.has_transforms = false);
        let mut layout = no_transforms.to_layout();
        layout.outputs[0].scale = Scale::from_millis(1500).unwrap();
        assert!(plan(&no_transforms, &layout).unwrap_err().is_unsupported());

        let mut fixed = server();
        fixed
            .crtcs
            .iter_mut()
            .for_each(|c| c.rotations = rotation::ROTATE_0);
        let mut layout = fixed.to_layout();
        layout.outputs[0].transform = Transform::Rotate90;
        assert!(plan(&fixed, &layout).unwrap_err().is_unsupported());

        let mut layout = s.to_layout();
        layout.outputs.push(Output::new("DP-9"));
        assert!(plan(&s, &layout).unwrap_err().is_cancelled());

        let mut layout = s.to_layout();
        layout.outputs[1].position = Position::new(8000, 0);
        assert!(plan(&s, &layout).unwrap_err().is_rejected());
    }

    #[test]
    fn primary_moves() {
        let s = server();
        let mut layout = s.to_layout();
        layout.outputs[0].primary = false;
        layout.outputs[1].primary = true;
        assert_eq!(plan(&s, &layout).unwrap().primary, Some(2));
    }

    #[test]
    fn transform_bits_round_trip() {
        for t in [
            Transform::Normal,
            Transform::Rotate90,
            Transform::Rotate180,
            Transform::Rotate270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            assert_eq!(transform_from_bits(bits_from_transform(t)), t);
        }
        assert_eq!(
            transform_from_bits(rotation::ROTATE_0 | rotation::REFLECT_Y),
            Transform::Flipped180
        );
        assert_eq!(
            transform_from_bits(rotation::ROTATE_0 | rotation::REFLECT_X | rotation::REFLECT_Y),
            Transform::Rotate180
        );
    }
}
