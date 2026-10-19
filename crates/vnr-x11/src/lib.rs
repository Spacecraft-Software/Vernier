// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for X11.
//!
//! Speaks `RandR` 1.3 or later directly through `x11rb`'s pure-Rust
//! connection instead of running `xrandr` and parsing its text, so mode
//! names such as `1080p` cannot break it.
//!
//! - Outputs map to CRTCs; an output keeps its CRTC when it can.
//! - Scale is a CRTC transform (the inverse of `xrandr --scale`, since
//!   Vernier's scale makes content larger, as on Wayland). Drivers without
//!   transforms, such as the dummy driver, cannot scale.
//! - Mirroring is a second CRTC at the source's position, the way
//!   `xrandr --same-as` does it.
//! - The primary output is `RRSetOutputPrimary`.
//! - Monitor identity comes from the `EDID` output property.
//!
//! `RandR` has no test-only request, so [`Backend::test`] is unsupported;
//! the plan is still checked against modes, CRTCs and the screen size
//! limits before anything is sent, and every change is applied with the
//! server grabbed so other clients never see a half-applied layout.

// Rust guideline compliant 2026-10-19

mod edid;
mod state;

use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout, Scale};
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::errors::{ConnectionError, ReplyError};
use x11rb::protocol::ErrorKind;
use x11rb::protocol::randr::{self, ConnectionExt as _, ModeFlag, Rotation, SetConfig};
use x11rb::protocol::render::{Fixed, Transform as Matrix};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

pub use crate::edid::Identity;
use crate::state::{Plan, Server, XCrtc, XMode, XOutput};

/// Oldest `RandR` with everything Vernier uses (transforms, primary output,
/// cheap current-state queries).
const MIN_VERSION: (u32, u32) = (1, 3);

/// 1.0 in `RandR`'s 16.16 fixed point.
const FIXED_ONE: Fixed = 1 << 16;

/// EDID base block plus one extension; only the base block is parsed.
const EDID_LONGS: u32 = 64;

/// A connection to X11 `RandR`.
#[derive(Debug)]
pub struct X11Backend {
    conn: RustConnection,
    root: Window,
    can_scale: bool,
}

impl X11Backend {
    /// Connects to the X server named by `DISPLAY`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unavailable`] when no X server answers and
    /// [`BackendError::is_unsupported`] when it lacks `RandR` 1.3.
    pub fn connect() -> Result<Self, BackendError> {
        Self::connect_to(None)
    }

    /// Connects to the named display (`:1`), or `DISPLAY` when `None`.
    ///
    /// # Errors
    ///
    /// As [`X11Backend::connect`].
    pub fn connect_to(display: Option<&str>) -> Result<Self, BackendError> {
        let (conn, screen) = x11rb::connect(display).map_err(|e| {
            BackendError::unavailable(format!("cannot connect to the X server: {e}"))
        })?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|s| s.root)
            .ok_or_else(|| {
                BackendError::protocol(format!("the X server has no screen {screen}"))
            })?;
        if conn
            .extension_information(randr::X11_EXTENSION_NAME)
            .map_err(connection)?
            .is_none()
        {
            return Err(BackendError::unsupported(
                "the X server has no RandR extension",
            ));
        }
        let version = conn
            .randr_query_version(MIN_VERSION.0, MIN_VERSION.1)
            .map_err(connection)?
            .reply()
            .map_err(reply)?;
        if (version.major_version, version.minor_version) < MIN_VERSION {
            return Err(BackendError::unsupported(format!(
                "RandR {}.{} is too old; Vernier needs {}.{}",
                version.major_version, version.minor_version, MIN_VERSION.0, MIN_VERSION.1
            )));
        }
        let mut backend = Self {
            conn,
            root,
            can_scale: false,
        };
        backend.can_scale = backend.read()?.can_scale();
        Ok(backend)
    }

    /// Reads screen, outputs, CRTCs and modes.
    ///
    /// Uses the probing `GetScreenResources`, as `xrandr` does: the cheaper
    /// `...Current` variant misses monitors plugged in since the last probe.
    fn read(&self) -> Result<Server, BackendError> {
        let conn = &self.conn;
        let resources = conn
            .randr_get_screen_resources(self.root)
            .map_err(connection)?
            .reply()
            .map_err(reply)?;
        let config = resources.config_timestamp;
        let geometry = conn
            .get_geometry(self.root)
            .map_err(connection)?
            .reply()
            .map_err(reply)?;
        let screen = &conn.setup().roots;
        let mm = screen
            .iter()
            .find(|s| s.root == self.root)
            .map_or((0, 0), |s| {
                (
                    u32::from(s.width_in_millimeters),
                    u32::from(s.height_in_millimeters),
                )
            });
        let range = conn
            .randr_get_screen_size_range(self.root)
            .map_err(connection)?
            .reply()
            .map_err(reply)?;
        let primary = conn
            .randr_get_output_primary(self.root)
            .map_err(connection)?
            .reply()
            .map_err(reply)?
            .output;
        let edid_atom = conn
            .intern_atom(true, b"EDID")
            .map_err(connection)?
            .reply()
            .map_err(reply)?
            .atom;

        let modes = read_modes(&resources.modes);
        let crtcs = self.read_crtcs(&resources.crtcs, config)?;
        let outputs = self.read_outputs(&resources.outputs, config, edid_atom)?;

        Ok(Server {
            size: (u32::from(geometry.width), u32::from(geometry.height)),
            mm,
            min: (u32::from(range.min_width), u32::from(range.min_height)),
            max: (u32::from(range.max_width), u32::from(range.max_height)),
            outputs,
            crtcs,
            modes,
            primary,
        })
    }

    fn read_crtcs(&self, ids: &[randr::Crtc], config: u32) -> Result<Vec<XCrtc>, BackendError> {
        let conn = &self.conn;
        let mut crtcs = Vec::with_capacity(ids.len());
        for &id in ids {
            let info = conn
                .randr_get_crtc_info(id, config)
                .map_err(connection)?
                .reply()
                .map_err(reply)?;
            let transform = conn
                .randr_get_crtc_transform(id)
                .map_err(connection)?
                .reply()
                .map_err(reply)?;
            crtcs.push(XCrtc {
                id,
                x: i32::from(info.x),
                y: i32::from(info.y),
                width: u32::from(info.width),
                height: u32::from(info.height),
                mode: info.mode,
                rotation: u16::from(info.rotation),
                outputs: info.outputs,
                scale: scale_from_matrix(&transform.current_transform),
                has_transforms: transform.has_transforms,
                rotations: u16::from(info.rotations),
            });
        }

        Ok(crtcs)
    }

    fn read_outputs(
        &self,
        ids: &[randr::Output],
        config: u32,
        edid_atom: u32,
    ) -> Result<Vec<XOutput>, BackendError> {
        let conn = &self.conn;
        let mut outputs = Vec::with_capacity(ids.len());
        for &id in ids {
            let info = conn
                .randr_get_output_info(id, config)
                .map_err(connection)?
                .reply()
                .map_err(reply)?;
            let identity = if edid_atom == 0 {
                Identity::default()
            } else {
                conn.randr_get_output_property(
                    id,
                    edid_atom,
                    AtomEnum::ANY,
                    0,
                    EDID_LONGS,
                    false,
                    false,
                )
                .map_err(connection)?
                .reply()
                .map(|p| edid::parse(&p.data))
                .unwrap_or_default()
            };
            outputs.push(XOutput {
                id,
                name: String::from_utf8_lossy(&info.name).into_owned(),
                connected: info.connection == randr::Connection::CONNECTED,
                crtc: info.crtc,
                crtcs: info.crtcs,
                modes: info.modes,
                preferred: usize::from(info.num_preferred),
                mm: (info.mm_width, info.mm_height),
                identity,
            });
        }

        Ok(outputs)
    }

    /// Sends `plan` with the server grabbed.
    fn send(&self, plan: &Plan) -> Result<(), BackendError> {
        let conn = &self.conn;
        let resources = conn
            .randr_get_screen_resources_current(self.root)
            .map_err(connection)?
            .reply()
            .map_err(reply)?;
        let config = resources.config_timestamp;
        let time = x11rb::CURRENT_TIME;
        let rotate_0 = Rotation::from(state::rotation::ROTATE_0);

        conn.grab_server().map_err(connection)?;
        let result = (|| -> Result<(), BackendError> {
            for &crtc in &plan.disable {
                set_config(conn.randr_set_crtc_config(crtc, time, config, 0, 0, 0, rotate_0, &[]))?;
            }
            if let Some(((width, height), (mm_width, mm_height))) = plan.screen {
                let narrow = |v: u32| {
                    u16::try_from(v).map_err(|e| {
                        BackendError::rejected(format!("screen size {v} is out of range: {e}"))
                    })
                };
                conn.randr_set_screen_size(
                    self.root,
                    narrow(width)?,
                    narrow(height)?,
                    mm_width,
                    mm_height,
                )
                .map_err(connection)?
                .check()
                .map_err(reply)?;
            }
            for c in &plan.configure {
                let (matrix, filter) = matrix_for(c.scale);
                if self.can_scale {
                    conn.randr_set_crtc_transform(c.crtc, matrix, filter, &[])
                        .map_err(connection)?
                        .check()
                        .map_err(reply)?;
                }
                let coordinate = |v: i32| {
                    i16::try_from(v).map_err(|e| {
                        BackendError::rejected(format!("position {v} is out of range: {e}"))
                    })
                };
                set_config(conn.randr_set_crtc_config(
                    c.crtc,
                    time,
                    config,
                    coordinate(c.x)?,
                    coordinate(c.y)?,
                    c.mode,
                    Rotation::from(c.rotation),
                    &c.outputs,
                ))?;
            }
            if let Some(primary) = plan.primary {
                conn.randr_set_output_primary(self.root, primary)
                    .map_err(connection)?
                    .check()
                    .map_err(reply)?;
            }
            Ok(())
        })();
        let ungrab = conn
            .ungrab_server()
            .map_err(connection)
            .and_then(|_| conn.flush().map_err(connection));
        result.and(ungrab)
    }
}

impl Backend for X11Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            mirroring: true,
            adaptive_sync: false,
            adaptive_sync_automatic: false,
            primary: true,
            custom_modes: false,
            fractional_scale: self.can_scale,
            test_only: false,
            persistent: false,
        }
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        Ok(self.read()?.to_layout())
    }

    fn test(&mut self, desired: &Layout) -> Result<(), BackendError> {
        // No test request exists, but a plan that cannot be made would be
        // refused, which is worth reporting.
        state::plan(&self.read()?, desired)?;
        Err(BackendError::unsupported(
            "RandR cannot test a configuration without applying it",
        ))
    }

    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError> {
        let plan = state::plan(&self.read()?, desired)?;
        if plan.is_empty() {
            return Ok(());
        }
        self.send(&plan)
    }
}

/// Converts `RandR` mode infos, computing each refresh rate.
fn read_modes(modes: &[randr::ModeInfo]) -> Vec<XMode> {
    modes
        .iter()
        .map(|m| {
            let flags = u32::from(m.mode_flags);
            XMode {
                id: m.id,
                width: u32::from(m.width),
                height: u32::from(m.height),
                refresh_mhz: XMode::refresh_mhz(
                    m.dot_clock,
                    m.htotal,
                    m.vtotal,
                    flags & u32::from(ModeFlag::INTERLACE) != 0,
                    flags & u32::from(ModeFlag::DOUBLE_SCAN) != 0,
                ),
            }
        })
        .collect()
}

/// Waits for a `SetCrtcConfig` reply and maps its status.
fn set_config(
    cookie: Result<
        x11rb::cookie::Cookie<'_, RustConnection, randr::SetCrtcConfigReply>,
        ConnectionError,
    >,
) -> Result<(), BackendError> {
    let status = cookie.map_err(connection)?.reply().map_err(reply)?.status;
    match status {
        SetConfig::SUCCESS => Ok(()),
        SetConfig::INVALID_CONFIG_TIME | SetConfig::INVALID_TIME => Err(BackendError::cancelled(
            "the outputs changed while applying; read them again",
        )),
        _ => Err(BackendError::rejected(
            "the X server refused the CRTC configuration",
        )),
    }
}

/// Vernier scale `s` shows `1/s` screen pixels per output pixel.
fn matrix_for(scale: Scale) -> (Matrix, &'static [u8]) {
    let factor = if scale == Scale::ONE {
        FIXED_ONE
    } else {
        let millis = i64::from(scale.millis());
        Fixed::try_from((i64::from(FIXED_ONE) * 1000 + millis / 2) / millis).unwrap_or(FIXED_ONE)
    };
    let matrix = Matrix {
        matrix11: factor,
        matrix12: 0,
        matrix13: 0,
        matrix21: 0,
        matrix22: factor,
        matrix23: 0,
        matrix31: 0,
        matrix32: 0,
        matrix33: FIXED_ONE,
    };
    let filter: &[u8] = if factor == FIXED_ONE {
        b"nearest"
    } else {
        b"bilinear"
    };
    (matrix, filter)
}

/// Reads a uniform scale back from a CRTC transform; anything else
/// (keystone, non-uniform) reads as 1.
fn scale_from_matrix(matrix: &Matrix) -> Scale {
    let uniform = matrix.matrix11 == matrix.matrix22
        && matrix.matrix11 > 0
        && matrix.matrix12 == 0
        && matrix.matrix21 == 0
        && matrix.matrix13 == 0
        && matrix.matrix23 == 0
        && matrix.matrix31 == 0
        && matrix.matrix32 == 0
        && matrix.matrix33 == FIXED_ONE;
    if !uniform {
        return Scale::ONE;
    }
    let millis =
        (i64::from(FIXED_ONE) * 1000 + i64::from(matrix.matrix11) / 2) / i64::from(matrix.matrix11);
    u32::try_from(millis)
        .ok()
        .and_then(|m| Scale::from_millis((m + 5) / 10 * 10))
        .unwrap_or(Scale::ONE)
}

#[expect(clippy::needless_pass_by_value, reason = "used as a `map_err` adapter")]
fn connection(error: ConnectionError) -> BackendError {
    BackendError::protocol(format!("X connection failed: {error}"))
}

fn reply(error: ReplyError) -> BackendError {
    match error {
        ReplyError::ConnectionError(e) => connection(e),
        ReplyError::X11Error(e) => match e.error_kind {
            ErrorKind::Value | ErrorKind::Match => BackendError::rejected(format!(
                "the X server refused the request ({:?})",
                e.error_kind
            )),
            kind => {
                BackendError::protocol(format!("X error {kind:?} in request {}", e.major_opcode))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_matrix_round_trips() {
        for millis in [1000, 1250, 1500, 2000, 3000] {
            let scale = Scale::from_millis(millis).unwrap();
            let (matrix, _) = matrix_for(scale);
            assert_eq!(scale_from_matrix(&matrix), scale, "{millis}");
        }
        // `xrandr --scale 1.5x1.5` (more desktop) reads as 0.67.
        let (mut matrix, _) = matrix_for(Scale::ONE);
        matrix.matrix11 = FIXED_ONE * 3 / 2;
        matrix.matrix22 = matrix.matrix11;
        assert_eq!(scale_from_matrix(&matrix).millis(), 670);
    }

    #[test]
    fn connect_without_a_server_is_unavailable() {
        let err = X11Backend::connect_to(Some("vnr-no-such-host/unix:99")).expect_err("no server");
        assert!(err.is_unavailable(), "{err}");
    }
}
