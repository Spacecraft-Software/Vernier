// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for GNOME (Mutter).
//!
//! Talks to `org.gnome.Mutter.DisplayConfig` on the session bus through
//! `zbus`, the same interface GNOME Settings uses:
//!
//! - `GetCurrentState` gives monitors, their modes and supported scales,
//!   and the logical monitors (position, scale, transform, primary).
//! - `ApplyMonitorsConfig` with method *verify* is a real test, so
//!   [`Backend::test`] asks Mutter; with method *temporary* it applies.
//! - Mirroring is several monitors in one logical monitor.
//!
//! Vernier applies *temporarily*: a *persistent* apply makes GNOME Shell
//! show its own keep-or-revert dialog, which would double Vernier's prompt
//! and revert `--yes` changes after 20 seconds. Changes therefore last until
//! the session ends; GNOME Settings makes them permanent.

// Rust guideline compliant 2026-10-20

mod state;

use std::collections::HashMap;

use vnr_core::{Backend, BackendError, BackendKind, Capabilities, Layout};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

pub use crate::state::{MLogical, MMode, MMonitor, MState, Request};

const DESTINATION: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";

/// `ApplyMonitorsConfig` methods.
const METHOD_VERIFY: u32 = 0;
const METHOD_TEMPORARY: u32 = 1;

/// Mutter's `layout-mode` value for physical layout.
const LAYOUT_PHYSICAL: u32 = 2;

type Props = HashMap<String, OwnedValue>;
type Spec = (String, String, String, String);
type WireMode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type WireMonitor = (Spec, Vec<WireMode>, Props);
type WireLogical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
type WireState = (u32, Vec<WireMonitor>, Vec<WireLogical>, Props);
type WireRequest<'a> = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, HashMap<String, Value<'a>>)>,
);

/// A connection to Mutter's display configuration.
#[derive(Debug)]
pub struct MutterBackend {
    conn: Connection,
    state: MState,
}

impl MutterBackend {
    /// Connects to the session bus and reads the display state.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unavailable`] when there is no session bus
    /// or Mutter is not on it.
    pub fn connect() -> Result<Self, BackendError> {
        Self::connect_to(None)
    }

    /// Connects to the D-Bus bus at `address` (e.g. `unix:path=/run/…`), or
    /// the session bus when `None`.
    ///
    /// # Errors
    ///
    /// As [`MutterBackend::connect`].
    pub fn connect_to(address: Option<&str>) -> Result<Self, BackendError> {
        let conn = match address {
            None => Connection::session(),
            Some(address) => zbus::blocking::connection::Builder::address(address)
                .and_then(zbus::blocking::connection::Builder::build),
        }
        .map_err(|e| BackendError::unavailable(format!("no D-Bus session bus: {e}")))?;
        let state = read(&conn)?;
        Ok(Self { conn, state })
    }

    fn send(&mut self, desired: &Layout, method: u32) -> Result<(), BackendError> {
        self.state = read(&self.conn)?;
        let requests = self.state.to_request(desired)?;
        let wire: Vec<WireRequest<'_>> = requests
            .iter()
            .map(|r| {
                (
                    r.x,
                    r.y,
                    r.scale,
                    r.transform,
                    r.primary,
                    r.monitors
                        .iter()
                        .map(|(connector, mode)| (connector.clone(), mode.clone(), HashMap::new()))
                        .collect(),
                )
            })
            .collect();
        let properties: HashMap<String, Value<'_>> = HashMap::new();
        self.conn
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "ApplyMonitorsConfig",
                &(self.state.serial, method, wire, properties),
            )
            .map_err(dbus_error)?;
        if method != METHOD_VERIFY {
            self.state = read(&self.conn)?;
        }
        Ok(())
    }
}

impl Backend for MutterBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Mutter
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            mirroring: self.state.mirroring,
            adaptive_sync: false,
            adaptive_sync_automatic: false,
            primary: true,
            custom_modes: false,
            fractional_scale: !self.state.physical,
            test_only: true,
            persistent: false,
        }
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        self.state = read(&self.conn)?;
        Ok(self.state.to_layout())
    }

    fn test(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.send(desired, METHOD_VERIFY)
    }

    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.send(desired, METHOD_TEMPORARY)
    }
}

/// Calls `GetCurrentState` and converts the reply.
fn read(conn: &Connection) -> Result<MState, BackendError> {
    let reply = conn
        .call_method(
            Some(DESTINATION),
            PATH,
            Some(INTERFACE),
            "GetCurrentState",
            &(),
        )
        .map_err(dbus_error)?;
    let (serial, monitors, logical, properties): WireState = reply
        .body()
        .deserialize()
        .map_err(|e| BackendError::protocol(format!("unexpected GetCurrentState reply: {e}")))?;
    Ok(MState {
        serial,
        monitors: monitors.into_iter().map(monitor).collect(),
        logical: logical
            .into_iter()
            .map(|(x, y, scale, transform, primary, specs, _)| MLogical {
                x,
                y,
                scale,
                transform,
                primary,
                monitors: specs.into_iter().map(|s| s.0).collect(),
            })
            .collect(),
        physical: prop_u32(&properties, "layout-mode") == Some(LAYOUT_PHYSICAL),
        global_scale: prop_bool(&properties, "global-scale-required").unwrap_or(false),
        mirroring: prop_bool(&properties, "supports-mirroring").unwrap_or(true),
    })
}

fn monitor((spec, modes, props): WireMonitor) -> MMonitor {
    let (connector, vendor, product, serial) = spec;
    MMonitor {
        connector,
        vendor,
        product,
        serial,
        display_name: prop_string(&props, "display-name"),
        modes: modes
            .into_iter()
            .map(|(id, width, height, refresh, _, scales, props)| MMode {
                id,
                width: u32::try_from(width).unwrap_or(0),
                height: u32::try_from(height).unwrap_or(0),
                refresh,
                scales,
                current: prop_bool(&props, "is-current").unwrap_or(false),
                preferred: prop_bool(&props, "is-preferred").unwrap_or(false),
            })
            .collect(),
    }
}

fn prop_bool(props: &Props, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(&**v).ok())
}

fn prop_u32(props: &Props, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| u32::try_from(&**v).ok())
}

fn prop_string(props: &Props, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(&**v).ok())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Maps a D-Bus failure to the matching [`BackendError`] kind.
#[expect(clippy::needless_pass_by_value, reason = "used as a `map_err` adapter")]
fn dbus_error(error: zbus::Error) -> BackendError {
    match &error {
        zbus::Error::MethodError(name, detail, _) => {
            let detail = detail.clone().unwrap_or_default();
            match name.as_str() {
                "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner" => BackendError::unavailable(
                    "Mutter's display configuration is not on the session bus (not GNOME?)",
                ),
                "org.freedesktop.DBus.Error.InvalidArgs" => {
                    BackendError::rejected(format!("Mutter rejected the configuration: {detail}"))
                }
                "org.freedesktop.DBus.Error.AccessDenied" if detail.contains("stale") => {
                    BackendError::cancelled("the monitors changed while applying; read them again")
                }
                _ => BackendError::protocol(format!("{name}: {detail}")),
            }
        }
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => {
            BackendError::unavailable(format!("no D-Bus session bus: {error}"))
        }
        _ => BackendError::protocol(format!("D-Bus call failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bus_is_unavailable() {
        let err = MutterBackend::connect_to(Some("unix:path=/nonexistent/vernier-bus"))
            .expect_err("no bus");
        assert!(err.is_unavailable(), "{err}");
    }

    #[test]
    fn properties_are_read_by_type() {
        let mut props: Props = HashMap::new();
        props.insert("is-current".into(), OwnedValue::from(true));
        props.insert("layout-mode".into(), OwnedValue::from(2u32));
        props.insert(
            "display-name".into(),
            OwnedValue::try_from(Value::from("Built-in display")).unwrap(),
        );
        assert_eq!(prop_bool(&props, "is-current"), Some(true));
        assert_eq!(prop_u32(&props, "layout-mode"), Some(2));
        assert_eq!(
            prop_string(&props, "display-name").as_deref(),
            Some("Built-in display")
        );
        assert_eq!(prop_bool(&props, "missing"), None);
    }
}
