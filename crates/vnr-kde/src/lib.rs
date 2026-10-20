// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for KDE Plasma.
//!
//! `KWin` describes each output with its own `kde_output_device_v2` global
//! and changes them through `kde_output_management_v2`; both come from
//! `wayland-protocols-plasma`. The backend covers:
//!
//! - mode, position, rotation and reflection, and fractional scale;
//! - the variable refresh rate policy (never, always, automatic) on outputs
//!   that support it;
//! - the primary output, which Plasma models as output priority 1
//!   (`kde_output_order_v1` reports the order; management version 3 sets
//!   it, version 2 falls back to `set_primary_output`).
//!
//! Plasma saves an applied configuration itself, so changes survive a
//! restart ([`Capabilities::persistent`]). The management protocol has no
//! test request, no custom modes and, in the versions this crate binds, no
//! mirroring; those are refused before anything is sent.

// Rust guideline compliant 2026-10-20

mod plan;

use vnr_core::{
    AdaptiveSync, Backend, BackendError, BackendKind, Capabilities, Layout, Mode, Output, Position,
    Scale, Transform,
};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};
use wayland_protocols_plasma::output_device::v2::client::{
    kde_output_device_mode_v2::{self, KdeOutputDeviceModeV2},
    kde_output_device_v2::{self, KdeOutputDeviceV2},
};
use wayland_protocols_plasma::output_management::v2::client::{
    kde_output_configuration_v2::{self, KdeOutputConfigurationV2},
    kde_output_management_v2::{self, KdeOutputManagementV2},
};
use wayland_protocols_plasma::output_order::v1::client::kde_output_order_v1::{
    self, KdeOutputOrderV1,
};

/// Highest `kde_output_device_v2` version this backend binds.
const DEVICE_MAX_VERSION: u32 = 11;
/// Highest `kde_output_management_v2` version this backend binds; version 3
/// adds output priorities, version 12 failure reasons.
const MANAGEMENT_MAX_VERSION: u32 = 12;
/// Capability bit for a variable refresh rate (`kde_output_device_v2`).
const CAPABILITY_VRR: u32 = 0x2;

/// A connection to KDE Plasma output management.
#[derive(Debug)]
pub struct KdeBackend {
    queue: EventQueue<State>,
    state: State,
    management: KdeOutputManagementV2,
}

impl KdeBackend {
    /// Connects to the compositor named by `WAYLAND_DISPLAY` and reads every
    /// output.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unavailable`] if there is no Wayland
    /// session or the compositor is not `KWin`.
    pub fn connect() -> Result<Self, BackendError> {
        Self::connect_to(None)
    }

    /// Connects to the named Wayland socket in `XDG_RUNTIME_DIR` (or an
    /// absolute socket path), or to `WAYLAND_DISPLAY` when `None`.
    ///
    /// # Errors
    ///
    /// As [`KdeBackend::connect`].
    pub fn connect_to(display: Option<&str>) -> Result<Self, BackendError> {
        let conn = match display {
            None => Connection::connect_to_env()
                .map_err(|e| BackendError::unavailable(format!("no Wayland session: {e}")))?,
            Some(name) => {
                let path = std::path::Path::new(name);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::var_os("XDG_RUNTIME_DIR")
                        .map(std::path::PathBuf::from)
                        .ok_or_else(|| BackendError::unavailable("XDG_RUNTIME_DIR is not set"))?
                        .join(name)
                };
                let stream = std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
                    BackendError::unavailable(format!("cannot reach `{}`: {e}", path.display()))
                })?;
                Connection::from_socket(stream)
                    .map_err(|e| BackendError::unavailable(format!("no Wayland session: {e}")))?
            }
        };
        let (globals, queue) = registry_queue_init::<State>(&conn)
            .map_err(|e| BackendError::protocol(format!("Wayland registry failed: {e}")))?;
        let qh = queue.handle();
        let management: KdeOutputManagementV2 = globals
            .bind(&qh, 1..=MANAGEMENT_MAX_VERSION, ())
            .map_err(|e| {
            BackendError::unavailable(format!("the compositor is not KDE Plasma: {e}"))
        })?;
        let order: Option<KdeOutputOrderV1> = globals.bind(&qh, 1..=1, ()).ok();

        let mut state = State::default();
        let registry = globals.registry();
        for global in globals.contents().clone_list() {
            if global.interface == KdeOutputDeviceV2::interface().name {
                let version = global.version.min(DEVICE_MAX_VERSION);
                let proxy: KdeOutputDeviceV2 = registry.bind(global.name, version, &qh, ());
                state.devices.push(Device::new(proxy));
            }
        }
        state.ordered = order.is_some();
        let mut backend = Self {
            queue,
            state,
            management,
        };
        backend.roundtrip()?;
        // Mode objects announced in the first round trip send their size
        // and refresh rate right after; a second round trip is cheap and
        // makes sure everything has arrived.
        backend.roundtrip()?;
        if backend.state.devices.is_empty() {
            return Err(BackendError::unavailable("KWin reports no outputs"));
        }
        Ok(backend)
    }

    fn roundtrip(&mut self) -> Result<(), BackendError> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|e| BackendError::protocol(format!("Wayland dispatch failed: {e}")))
    }

    fn layout(&self) -> Layout {
        Layout::new(
            self.state
                .devices
                .iter()
                .map(|d| d.to_output(&self.state.modes, &self.state.order))
                .collect(),
        )
    }

    /// Checks that `desired` can be expressed, and resolves each enabled
    /// output's mode object.
    fn resolve<'a>(&'a self, desired: &Layout) -> Result<Vec<Resolved<'a>>, BackendError> {
        if let Some(mirror) = desired
            .outputs
            .iter()
            .find(|o| o.enabled && o.mirror_of.is_some())
        {
            return Err(BackendError::unsupported(format!(
                "KDE output management cannot mirror outputs (`{}`); use Plasma's display settings",
                mirror.name
            )));
        }
        for output in &desired.outputs {
            if !self.state.devices.iter().any(|d| d.name == output.name) {
                return Err(BackendError::cancelled(format!(
                    "`{}` is no longer connected; read the outputs again",
                    output.name
                )));
            }
        }
        self.state
            .devices
            .iter()
            .map(|device| {
                let current = device.to_output(&self.state.modes, &self.state.order);
                let want = desired.get(&device.name).unwrap_or(&current).clone();
                let mode = if want.enabled {
                    let target = want.mode.or_else(|| want.preferred_mode()).ok_or_else(|| {
                        BackendError::rejected(format!("`{}` has no mode to show", device.name))
                    })?;
                    Some(device.find_mode(&self.state.modes, &target).ok_or_else(|| {
                        BackendError::unsupported(format!(
                            "`{}` does not advertise {target}, and KDE output management cannot add modes",
                            device.name
                        ))
                    })?)
                } else {
                    None
                };
                Ok(Resolved { device, want, mode })
            })
            .collect()
    }

    fn submit(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.roundtrip()?;
        let resolved = self.resolve(desired)?;
        let current = self.layout();
        let qh = self.queue.handle();
        let config = self.management.create_configuration(&qh, ());
        for r in &resolved {
            let proxy = &r.device.proxy;
            config.enable(proxy, i32::from(r.want.enabled));
            let Some(mode) = r.mode else {
                continue;
            };
            config.mode(proxy, mode);
            config.transform(proxy, transform_to_kde(r.want.transform));
            config.position(proxy, r.want.position.x, r.want.position.y);
            config.scale(proxy, r.want.scale.as_f64());
            let old = current.get(&r.device.name).and_then(|o| o.adaptive_sync);
            if let Some(sync) = r
                .want
                .adaptive_sync
                .filter(|s| Some(*s) != old && r.device.has_vrr())
            {
                config.set_vrr_policy(proxy, vrr_to_kde(sync));
            }
        }
        self.set_primary(&config, &resolved, &current);

        config.apply();
        self.state.result = None;
        self.state.failure_reason = None;
        while self.state.result.is_none() {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(|e| BackendError::protocol(format!("Wayland dispatch failed: {e}")))?;
        }
        config.destroy();
        let result = self.state.result.take();
        let reason = self.state.failure_reason.take();
        // Pick up the new state, so a snapshot right after is current.
        self.roundtrip()?;
        match result {
            Some(true) => Ok(()),
            _ => Err(BackendError::rejected(match reason {
                Some(reason) => format!("KWin rejected the configuration: {reason}"),
                None => "KWin rejected the configuration".to_owned(),
            })),
        }
    }

    /// Plasma's primary output is priority 1. Version 3 sets the whole
    /// order; version 2 can only name the primary.
    fn set_primary(
        &self,
        config: &KdeOutputConfigurationV2,
        resolved: &[Resolved<'_>],
        current: &Layout,
    ) {
        let wanted = resolved.iter().find(|r| r.want.enabled && r.want.primary);
        let now = current.outputs.iter().find(|o| o.primary);
        let enabled_changed = resolved.iter().any(|r| {
            current
                .get(&r.device.name)
                .is_some_and(|o| o.enabled != r.want.enabled)
        });
        if wanted.map(|r| &r.device.name) == now.map(|o| &o.name) && !enabled_changed {
            return;
        }
        if self.management.version() >= 3 {
            let enabled: Vec<(&str, bool)> = resolved
                .iter()
                .map(|r| (r.device.name.as_str(), r.want.enabled))
                .collect();
            let primary = wanted.map(|r| r.device.name.as_str());
            for (name, priority) in plan::priorities(&self.state.order, &enabled, primary) {
                if let Some(r) = resolved.iter().find(|r| r.device.name == name) {
                    config.set_priority(&r.device.proxy, priority);
                }
            }
        } else if self.management.version() >= 2
            && let Some(r) = wanted
        {
            config.set_primary_output(&r.device.proxy);
        }
    }
}

/// One output with the settings to send.
struct Resolved<'a> {
    device: &'a Device,
    want: Output,
    mode: Option<&'a KdeOutputDeviceModeV2>,
}

impl Backend for KdeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Kde
    }

    fn capabilities(&self) -> Capabilities {
        let vrr = self.state.devices.iter().any(Device::has_vrr);
        Capabilities {
            mirroring: false,
            adaptive_sync: vrr,
            adaptive_sync_automatic: vrr,
            primary: self.management.version() >= 2,
            custom_modes: false,
            fractional_scale: true,
            test_only: false,
            persistent: true,
        }
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        self.roundtrip()?;
        Ok(self.layout())
    }

    fn test(&mut self, desired: &Layout) -> Result<(), BackendError> {
        // No test request exists; still refuse what cannot be expressed.
        self.roundtrip()?;
        self.resolve(desired)?;
        Err(BackendError::unsupported(
            "KDE output management cannot test a configuration without applying it",
        ))
    }

    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.submit(desired)
    }
}

fn transform_to_kde(transform: Transform) -> i32 {
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

fn transform_from_kde(value: i32) -> Transform {
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

fn vrr_to_kde(sync: AdaptiveSync) -> kde_output_configuration_v2::VrrPolicy {
    match sync {
        AdaptiveSync::Disabled => kde_output_configuration_v2::VrrPolicy::Never,
        AdaptiveSync::Always => kde_output_configuration_v2::VrrPolicy::Always,
        AdaptiveSync::Automatic => kde_output_configuration_v2::VrrPolicy::Automatic,
    }
}

fn vrr_from_kde(policy: WEnum<kde_output_device_v2::VrrPolicy>) -> AdaptiveSync {
    match policy {
        WEnum::Value(kde_output_device_v2::VrrPolicy::Always) => AdaptiveSync::Always,
        WEnum::Value(kde_output_device_v2::VrrPolicy::Automatic) => AdaptiveSync::Automatic,
        _ => AdaptiveSync::Disabled,
    }
}

#[derive(Debug)]
struct ModeEntry {
    proxy: KdeOutputDeviceModeV2,
    mode: Mode,
}

#[derive(Debug)]
struct Device {
    proxy: KdeOutputDeviceV2,
    name: String,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    physical_size_mm: Option<(u32, u32)>,
    modes: Vec<ObjectId>,
    current_mode: Option<ObjectId>,
    enabled: bool,
    position: Position,
    transform: Transform,
    scale: f64,
    capabilities: u32,
    vrr_policy: Option<AdaptiveSync>,
}

impl Device {
    fn new(proxy: KdeOutputDeviceV2) -> Self {
        Self {
            proxy,
            name: String::new(),
            make: None,
            model: None,
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: None,
            enabled: false,
            position: Position::default(),
            transform: Transform::Normal,
            scale: 1.0,
            capabilities: 0,
            vrr_policy: None,
        }
    }

    fn has_vrr(&self) -> bool {
        self.capabilities & CAPABILITY_VRR != 0
    }

    fn find_mode<'a>(
        &self,
        modes: &'a [ModeEntry],
        wanted: &Mode,
    ) -> Option<&'a KdeOutputDeviceModeV2> {
        let own = || modes.iter().filter(|m| self.modes.contains(&m.proxy.id()));
        own()
            .find(|m| m.mode.same_timing(wanted))
            .or_else(|| {
                own()
                    .filter(|m| m.mode.approx_eq(wanted))
                    .min_by_key(|m| m.mode.refresh_mhz.abs_diff(wanted.refresh_mhz))
            })
            .map(|m| &m.proxy)
    }

    fn to_output(&self, modes: &[ModeEntry], order: &[String]) -> Output {
        let lookup = |id: &ObjectId| modes.iter().find(|m| &m.proxy.id() == id).map(|m| m.mode);
        let mut output = Output::new(self.name.clone());
        output.make.clone_from(&self.make);
        output.model.clone_from(&self.model);
        output.serial.clone_from(&self.serial);
        output.description = match (&self.make, &self.model) {
            (Some(maker), Some(product)) => format!("{maker} {product} ({})", self.name),
            _ => self.name.clone(),
        };
        output.physical_size_mm = self.physical_size_mm;
        output.modes = self.modes.iter().filter_map(lookup).collect();
        output.enabled = self.enabled;
        output.mode = if self.enabled {
            self.current_mode.as_ref().and_then(lookup)
        } else {
            None
        };
        output.position = self.position;
        output.transform = self.transform;
        output.scale = Scale::from_f64(self.scale).unwrap_or(Scale::ONE);
        output.adaptive_sync = self.vrr_policy.filter(|_| self.has_vrr());
        output.primary = self.enabled && order.first() == Some(&self.name);
        output
    }
}

#[derive(Debug, Default)]
struct State {
    devices: Vec<Device>,
    modes: Vec<ModeEntry>,
    /// Output names by priority, primary first.
    order: Vec<String>,
    /// The order being received; replaces `order` on `done`.
    pending_order: Vec<String>,
    /// Whether the compositor reports the output order at all.
    ordered: bool,
    result: Option<bool>,
    failure_reason: Option<String>,
}

impl State {
    fn device_mut(&mut self, id: &ObjectId) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| &d.proxy.id() == id)
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() == value.len() {
        (!value.is_empty()).then_some(value)
    } else {
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Outputs plugged in after connecting are not tracked: backends are
        // reopened for every operation.
    }
}

impl Dispatch<KdeOutputDeviceV2, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &KdeOutputDeviceV2,
        event: kde_output_device_v2::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use kde_output_device_v2::Event;

        if let Event::Mode { mode } = event {
            if let Some(device) = state.device_mut(&proxy.id()) {
                device.modes.push(mode.id());
            }
            state.modes.push(ModeEntry {
                proxy: mode,
                mode: Mode::new(0, 0, 0),
            });
            return;
        }
        let Some(device) = state.device_mut(&proxy.id()) else {
            return;
        };
        match event {
            Event::Geometry {
                x,
                y,
                physical_width,
                physical_height,
                make,
                model,
                transform,
                ..
            } => {
                device.position = Position::new(x, y);
                device.physical_size_mm = u32::try_from(physical_width)
                    .ok()
                    .zip(u32::try_from(physical_height).ok())
                    .filter(|(w, h)| *w > 0 && *h > 0);
                device.make = non_empty(make);
                device.model = non_empty(model);
                device.transform = transform_from_kde(transform);
                if device.name.is_empty() {
                    // Version 1 has no name event; the model is the best
                    // stable label it offers.
                    device.name = device.model.clone().unwrap_or_default();
                }
            }
            Event::CurrentMode { mode } => device.current_mode = Some(mode.id()),
            Event::Scale { factor } => device.scale = factor,
            Event::Enabled { enabled } => device.enabled = enabled != 0,
            Event::SerialNumber {
                serialNumber: serial,
            } => device.serial = non_empty(serial),
            Event::Capabilities { flags } => device.capabilities = flags.into(),
            Event::VrrPolicy { vrr_policy } => device.vrr_policy = Some(vrr_from_kde(vrr_policy)),
            Event::Name { name } => device.name = name,
            _ => {}
        }
    }

    event_created_child!(State, KdeOutputDeviceV2, [
        kde_output_device_v2::EVT_MODE_OPCODE => (KdeOutputDeviceModeV2, ()),
    ]);
}

impl Dispatch<KdeOutputDeviceModeV2, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &KdeOutputDeviceModeV2,
        event: kde_output_device_mode_v2::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use kde_output_device_mode_v2::Event;

        let id = proxy.id();
        if let Event::Removed = event {
            state.modes.retain(|m| m.proxy.id() != id);
            for device in &mut state.devices {
                device.modes.retain(|m| m != &id);
                if device.current_mode.as_ref() == Some(&id) {
                    device.current_mode = None;
                }
            }
            return;
        }
        let Some(entry) = state.modes.iter_mut().find(|m| m.proxy.id() == id) else {
            return;
        };
        let to_u32 = |v: i32| u32::try_from(v).unwrap_or(0);
        match event {
            Event::Size { width, height } => {
                entry.mode.width = to_u32(width);
                entry.mode.height = to_u32(height);
            }
            Event::Refresh { refresh } => entry.mode.refresh_mhz = to_u32(refresh),
            Event::Preferred => entry.mode.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<KdeOutputManagementV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &KdeOutputManagementV2,
        _: kde_output_management_v2::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The interface has no events.
    }
}

impl Dispatch<KdeOutputConfigurationV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &KdeOutputConfigurationV2,
        event: kde_output_configuration_v2::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use kde_output_configuration_v2::Event;

        match event {
            Event::Applied => state.result = Some(true),
            Event::Failed => state.result = Some(false),
            Event::FailureReason { reason } => state.failure_reason = non_empty(reason),
            _ => {}
        }
    }
}

impl Dispatch<KdeOutputOrderV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &KdeOutputOrderV1,
        event: kde_output_order_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            kde_output_order_v1::Event::Output { output_name } => {
                state.pending_order.push(output_name);
            }
            kde_output_order_v1::Event::Done => {
                state.order = std::mem::take(&mut state.pending_order);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_round_trip() {
        for transform in Transform::ALL {
            assert_eq!(transform_from_kde(transform_to_kde(transform)), transform);
        }
    }

    #[test]
    fn vrr_policies_round_trip() {
        for sync in [
            AdaptiveSync::Disabled,
            AdaptiveSync::Always,
            AdaptiveSync::Automatic,
        ] {
            let sent = vrr_to_kde(sync) as u32;
            let read = kde_output_device_v2::VrrPolicy::try_from(sent)
                .map_or(WEnum::Unknown(sent), WEnum::Value);
            assert_eq!(vrr_from_kde(read), sync);
        }
    }
}
