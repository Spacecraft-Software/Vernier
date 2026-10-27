// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Vernier backend for `wlr-output-management-unstable-v1`.
//!
//! Covers niri, sway, Hyprland, river, labwc, Wayfire and COSMIC. When the
//! compositor also advertises COSMIC's `zcosmic_output_manager_v1`, the
//! backend reports [`BackendKind::Cosmic`] and additionally handles:
//!
//! - output mirroring (`mirror_head`),
//! - scale in thousandths (`scale_1000`) instead of a float,
//! - three-state adaptive sync (disabled, automatic, always),
//! - the Xwayland primary output.
//!
//! Configurations set this way are transient on niri and sway; see
//! [`vnr_core::export`] for making them permanent.
//!
//! # Examples
//!
//! ```no_run
//! use vnr_core::Backend;
//!
//! # fn main() -> Result<(), vnr_core::BackendError> {
//! let mut backend = vnr_wlr::WlrBackend::connect()?;
//! let layout = backend.snapshot()?;
//! for output in &layout.outputs {
//!     println!("{} {:?}", output.name, output.mode);
//! }
//! # Ok(())
//! # }
//! ```

// Rust guideline compliant 2026-05-18

mod protocol;

use vnr_core::{
    AdaptiveSync, Backend, BackendError, BackendKind, Capabilities, Layout, Mode, Output, Position,
    Scale, Transform,
};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use crate::protocol::cosmic_output::{
    zcosmic_output_configuration_head_v1::{self, ZcosmicOutputConfigurationHeadV1},
    zcosmic_output_configuration_v1::{self, ZcosmicOutputConfigurationV1},
    zcosmic_output_head_v1::{self, ZcosmicOutputHeadV1},
    zcosmic_output_manager_v1::{self, ZcosmicOutputManagerV1},
};

/// Highest `zwlr_output_manager_v1` version this backend understands.
/// Version 4 adds adaptive sync; versions 2 and 3 add make/model/serial
/// and the `release` destructors.
const WLR_MAX_VERSION: u32 = 4;

/// Highest `zcosmic_output_manager_v1` version this backend understands.
/// Version 2 adds three-state adaptive sync, version 3 the Xwayland primary.
const COSMIC_MAX_VERSION: u32 = 3;

/// A connection to a compositor implementing wlr-output-management.
#[derive(Debug)]
pub struct WlrBackend {
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrOutputManagerV1,
}

impl WlrBackend {
    /// Connects to the compositor named by `WAYLAND_DISPLAY` and reads the
    /// initial output state.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::is_unavailable`] if there is no Wayland
    /// session or the compositor does not implement wlr-output-management.
    pub fn connect() -> Result<Self, BackendError> {
        Self::connect_to(None)
    }

    /// Connects to the named Wayland socket in `XDG_RUNTIME_DIR` (or an
    /// absolute socket path), or to `WAYLAND_DISPLAY` when `None`.
    ///
    /// # Errors
    ///
    /// As [`WlrBackend::connect`].
    pub fn connect_to(display: Option<&str>) -> Result<Self, BackendError> {
        let conn = match display {
            None => Connection::connect_to_env().map_err(|e| no_session(&e))?,
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
                Connection::from_socket(stream).map_err(|e| no_session(&e))?
            }
        };
        let (globals, queue) = registry_queue_init::<State>(&conn)
            .map_err(|e| BackendError::protocol(format!("Wayland registry failed: {e}")))?;
        let qh = queue.handle();
        let manager: ZwlrOutputManagerV1 =
            globals.bind(&qh, 1..=WLR_MAX_VERSION, ()).map_err(|_e| {
                BackendError::unavailable("the compositor does not implement wlr-output-management")
            })?;
        let cosmic: Option<ZcosmicOutputManagerV1> =
            globals.bind(&qh, 1..=COSMIC_MAX_VERSION, ()).ok();

        let mut backend = Self {
            queue,
            state: State::new(cosmic),
            manager,
        };
        backend.sync()?;
        Ok(backend)
    }

    /// Dispatches until the compositor has sent a complete, current state.
    ///
    /// Heads announced during the first round trip get their COSMIC
    /// extension objects requested from the event handler; the compositor
    /// sends those properties followed by another `done`, which the second
    /// round trip collects.
    fn sync(&mut self) -> Result<(), BackendError> {
        let rounds = if self.state.cosmic.is_some() { 2 } else { 1 };
        for _ in 0..rounds {
            self.roundtrip()?;
        }
        if self.state.finished {
            return Err(BackendError::unavailable(
                "the compositor withdrew wlr-output-management",
            ));
        }
        if self.state.serial.is_none() {
            return Err(BackendError::protocol(
                "the compositor never completed the output state",
            ));
        }
        Ok(())
    }

    fn roundtrip(&mut self) -> Result<(), BackendError> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|e| BackendError::protocol(format!("Wayland dispatch failed: {e}")))
    }

    fn submit(&mut self, desired: &Layout, test_only: bool) -> Result<(), BackendError> {
        self.sync()?;
        let wants_mirror = desired
            .outputs
            .iter()
            .any(|o| o.enabled && o.mirror_of.is_some());
        if wants_mirror && self.state.cosmic.is_none() {
            return Err(BackendError::unsupported(
                "this compositor cannot mirror outputs (needs COSMIC's output management extension)",
            ));
        }
        let serial = self
            .state
            .serial
            .ok_or_else(|| BackendError::protocol("no output state serial"))?;

        let qh = self.queue.handle();
        let config = self.manager.create_configuration(serial, &qh, ());
        let cosmic_config = self
            .state
            .cosmic
            .as_ref()
            .map(|m| m.get_configuration(&config, &qh, ()));

        let mut ext_heads = Vec::new();
        // Mirror sources must be enabled before a head can mirror them, so
        // configure every non-mirroring head first.
        let (mirrors, others): (Vec<&Head>, Vec<&Head>) = self.state.heads.iter().partition(|h| {
            desired
                .get(&h.name)
                .is_some_and(|o| o.enabled && o.mirror_of.is_some())
        });
        for head in others.into_iter().chain(mirrors) {
            let current = head.to_output(&self.state.modes);
            let want = desired.get(&head.name).unwrap_or(&current);
            if !want.enabled {
                config.disable_head(&head.proxy);
                continue;
            }
            let cfg_head = match (&want.mirror_of, &cosmic_config) {
                (Some(source), Some(ext)) => {
                    let source = self
                        .state
                        .heads
                        .iter()
                        .find(|h| &h.name == source)
                        .ok_or_else(|| {
                            BackendError::rejected(format!(
                                "mirror source {source} is not connected"
                            ))
                        })?;
                    ext.mirror_head(&head.proxy, &source.proxy, &qh, ())
                }
                _ => config.enable_head(&head.proxy, &qh, ()),
            };
            self.configure_head(head, want, &current, &cfg_head, &mut ext_heads);
        }

        if test_only {
            config.test();
        } else {
            config.apply();
        }
        self.state.result = None;
        while self.state.result.is_none() {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(|e| BackendError::protocol(format!("Wayland dispatch failed: {e}")))?;
        }
        for ext in ext_heads {
            ext.release();
        }
        if let Some(ext) = cosmic_config {
            ext.release();
        }
        config.destroy();

        match self.state.result.take() {
            Some(ConfigResult::Succeeded) => {
                if !test_only {
                    self.set_primary(desired)?;
                }
                Ok(())
            }
            Some(ConfigResult::Failed) => Err(BackendError::rejected(
                "the compositor rejected the configuration",
            )),
            Some(ConfigResult::Cancelled) | None => Err(BackendError::cancelled(
                "the outputs changed while the configuration was being sent",
            )),
        }
    }

    fn configure_head(
        &self,
        head: &Head,
        want: &Output,
        current: &Output,
        cfg_head: &ZwlrOutputConfigurationHeadV1,
        ext_heads: &mut Vec<ZcosmicOutputConfigurationHeadV1>,
    ) {
        let qh = self.queue.handle();
        if let Some(mode) = want.mode {
            match head.find_mode(&self.state.modes, &mode) {
                Some(proxy) => cfg_head.set_mode(proxy),
                None => cfg_head.set_custom_mode(
                    i32::try_from(mode.width).unwrap_or(i32::MAX),
                    i32::try_from(mode.height).unwrap_or(i32::MAX),
                    i32::try_from(mode.refresh_mhz).unwrap_or(0),
                ),
            }
        }
        cfg_head.set_position(want.position.x, want.position.y);
        cfg_head.set_transform(to_wl_transform(want.transform));

        let ext = self
            .state
            .cosmic
            .as_ref()
            .map(|m| m.get_configuration_head(cfg_head, &qh, ()));
        match &ext {
            Some(ext) => ext.set_scale_1000(i32::try_from(want.scale.millis()).unwrap_or(1000)),
            None => cfg_head.set_scale(want.scale.as_f64()),
        }

        if want.adaptive_sync != current.adaptive_sync
            && let Some(sync) = want.adaptive_sync
        {
            match &ext {
                Some(ext) if ext.version() >= 2 => ext.set_adaptive_sync_ext(match sync {
                    AdaptiveSync::Disabled => {
                        zcosmic_output_head_v1::AdaptiveSyncStateExt::Disabled
                    }
                    AdaptiveSync::Automatic => {
                        zcosmic_output_head_v1::AdaptiveSyncStateExt::Automatic
                    }
                    AdaptiveSync::Always => zcosmic_output_head_v1::AdaptiveSyncStateExt::Always,
                }),
                _ if cfg_head.version() >= 4 => cfg_head.set_adaptive_sync(match sync {
                    AdaptiveSync::Disabled => zwlr_output_head_v1::AdaptiveSyncState::Disabled,
                    AdaptiveSync::Automatic | AdaptiveSync::Always => {
                        zwlr_output_head_v1::AdaptiveSyncState::Enabled
                    }
                }),
                _ => {}
            }
        }
        if let Some(ext) = ext {
            ext_heads.push(ext);
        }
    }

    /// Marks the Xwayland primary output on COSMIC (manager version 3).
    fn set_primary(&mut self, desired: &Layout) -> Result<(), BackendError> {
        let Some(manager) = self.state.cosmic.as_ref().filter(|m| m.version() >= 3) else {
            return Ok(());
        };
        let wanted = desired.outputs.iter().find(|o| o.enabled && o.primary);
        let current = self.state.heads.iter().find(|h| h.xwayland_primary);
        if wanted.map(|o| &o.name) == current.map(|h| &h.name) {
            return Ok(());
        }
        let head = wanted.and_then(|o| self.state.heads.iter().find(|h| h.name == o.name));
        manager.set_xwayland_primary(head.and_then(|h| h.cosmic.as_ref()));
        self.roundtrip()
    }
}

impl Backend for WlrBackend {
    fn kind(&self) -> BackendKind {
        if self.state.cosmic.is_some() {
            BackendKind::Cosmic
        } else {
            BackendKind::Wlr
        }
    }

    fn capabilities(&self) -> Capabilities {
        let cosmic_version = self.state.cosmic.as_ref().map_or(0, Proxy::version);
        Capabilities {
            mirroring: cosmic_version >= 1,
            adaptive_sync: self.manager.version() >= 4 || cosmic_version >= 2,
            adaptive_sync_automatic: cosmic_version >= 2,
            primary: cosmic_version >= 3,
            custom_modes: true,
            fractional_scale: true,
            test_only: true,
            persistent: false,
            save_on_request: false,
        }
    }

    fn snapshot(&mut self) -> Result<Layout, BackendError> {
        self.sync()?;
        Ok(Layout::new(
            self.state
                .heads
                .iter()
                .map(|h| h.to_output(&self.state.modes))
                .collect(),
        ))
    }

    fn test(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.submit(desired, true)
    }

    fn apply(&mut self, desired: &Layout) -> Result<(), BackendError> {
        self.submit(desired, false)
    }
}

fn to_wl_transform(transform: Transform) -> wl_output::Transform {
    match transform {
        Transform::Normal => wl_output::Transform::Normal,
        Transform::Rotate90 => wl_output::Transform::_90,
        Transform::Rotate180 => wl_output::Transform::_180,
        Transform::Rotate270 => wl_output::Transform::_270,
        Transform::Flipped => wl_output::Transform::Flipped,
        Transform::Flipped90 => wl_output::Transform::Flipped90,
        Transform::Flipped180 => wl_output::Transform::Flipped180,
        Transform::Flipped270 => wl_output::Transform::Flipped270,
    }
}

fn from_wl_transform(transform: WEnum<wl_output::Transform>) -> Transform {
    match transform {
        WEnum::Value(wl_output::Transform::_90) => Transform::Rotate90,
        WEnum::Value(wl_output::Transform::_180) => Transform::Rotate180,
        WEnum::Value(wl_output::Transform::_270) => Transform::Rotate270,
        WEnum::Value(wl_output::Transform::Flipped) => Transform::Flipped,
        WEnum::Value(wl_output::Transform::Flipped90) => Transform::Flipped90,
        WEnum::Value(wl_output::Transform::Flipped180) => Transform::Flipped180,
        WEnum::Value(wl_output::Transform::Flipped270) => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigResult {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug)]
struct ModeEntry {
    proxy: ZwlrOutputModeV1,
    mode: Mode,
}

#[derive(Debug)]
struct Head {
    proxy: ZwlrOutputHeadV1,
    cosmic: Option<ZcosmicOutputHeadV1>,
    name: String,
    description: String,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    physical_size_mm: Option<(u32, u32)>,
    modes: Vec<ObjectId>,
    enabled: bool,
    current_mode: Option<ObjectId>,
    position: Position,
    transform: Transform,
    scale: f64,
    scale_1000: Option<u32>,
    adaptive_sync: Option<AdaptiveSync>,
    adaptive_sync_ext: Option<AdaptiveSync>,
    mirroring: Option<String>,
    xwayland_primary: bool,
}

impl Head {
    fn new(proxy: ZwlrOutputHeadV1) -> Self {
        Self {
            proxy,
            cosmic: None,
            name: String::new(),
            description: String::new(),
            make: None,
            model: None,
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            enabled: false,
            current_mode: None,
            position: Position::default(),
            transform: Transform::Normal,
            scale: 1.0,
            scale_1000: None,
            adaptive_sync: None,
            adaptive_sync_ext: None,
            mirroring: None,
            xwayland_primary: false,
        }
    }

    fn find_mode<'a>(&self, modes: &'a [ModeEntry], wanted: &Mode) -> Option<&'a ZwlrOutputModeV1> {
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

    fn to_output(&self, modes: &[ModeEntry]) -> Output {
        let lookup = |id: &ObjectId| modes.iter().find(|m| &m.proxy.id() == id).map(|m| m.mode);
        let mut output = Output::new(self.name.clone());
        output.description.clone_from(&self.description);
        output.make.clone_from(&self.make);
        output.model.clone_from(&self.model);
        output.serial.clone_from(&self.serial);
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
        output.scale = self
            .scale_1000
            .and_then(Scale::from_millis)
            .or_else(|| Scale::from_f64(self.scale))
            .unwrap_or(Scale::ONE);
        output.adaptive_sync = self.adaptive_sync_ext.or(self.adaptive_sync);
        output.mirror_of = self.mirroring.clone().filter(|_| self.enabled);
        output.primary = self.xwayland_primary;
        output
    }
}

#[derive(Debug)]
struct State {
    cosmic: Option<ZcosmicOutputManagerV1>,
    heads: Vec<Head>,
    modes: Vec<ModeEntry>,
    serial: Option<u32>,
    finished: bool,
    result: Option<ConfigResult>,
}

impl State {
    fn new(cosmic: Option<ZcosmicOutputManagerV1>) -> Self {
        Self {
            cosmic,
            heads: Vec::new(),
            modes: Vec::new(),
            serial: None,
            finished: false,
            result: None,
        }
    }

    fn head_mut(&mut self, id: &ObjectId) -> Option<&mut Head> {
        self.heads.iter_mut().find(|h| &h.proxy.id() == id)
    }

    fn mode_mut(&mut self, id: &ObjectId) -> Option<&mut Mode> {
        self.modes
            .iter_mut()
            .find(|m| &m.proxy.id() == id)
            .map(|m| &mut m.mode)
    }
}

fn to_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
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
        // Globals added after startup (a new output manager) are not
        // relevant: backends are reopened for every operation.
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        (): &(),
        _: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                let mut entry = Head::new(head);
                if let Some(manager) = &state.cosmic {
                    entry.cosmic = Some(manager.get_head(&entry.proxy, qhandle, entry.proxy.id()));
                }
                state.heads.push(entry);
            }
            zwlr_output_manager_v1::Event::Done { serial } => state.serial = Some(serial),
            zwlr_output_manager_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_head_v1::Event;

        if let Event::Mode { mode } = event {
            if let Some(head) = state.head_mut(&proxy.id()) {
                head.modes.push(mode.id());
            }
            state.modes.push(ModeEntry {
                proxy: mode,
                mode: Mode::new(0, 0, 0),
            });
            return;
        }
        if let Event::Finished = event {
            if proxy.version() >= 3 {
                proxy.release();
            }
            if let Some(index) = state.heads.iter().position(|h| h.proxy.id() == proxy.id()) {
                let head = state.heads.remove(index);
                if let Some(ext) = head.cosmic {
                    ext.release();
                }
            }
            return;
        }
        let Some(head) = state.head_mut(&proxy.id()) else {
            return;
        };
        match event {
            Event::Name { name } => head.name = name,
            Event::Description { description } => head.description = description,
            Event::PhysicalSize { width, height } => {
                head.physical_size_mm =
                    (width > 0 && height > 0).then(|| (to_u32(width), to_u32(height)));
            }
            Event::Enabled { enabled } => head.enabled = enabled != 0,
            Event::CurrentMode { mode } => head.current_mode = Some(mode.id()),
            Event::Position { x, y } => head.position = Position::new(x, y),
            Event::Transform { transform } => head.transform = from_wl_transform(transform),
            Event::Scale { scale } => head.scale = scale,
            Event::Make { make } => head.make = non_empty(make),
            Event::Model { model } => head.model = non_empty(model),
            Event::SerialNumber { serial_number } => head.serial = non_empty(serial_number),
            Event::AdaptiveSync { state: sync } => {
                head.adaptive_sync = Some(match sync {
                    WEnum::Value(zwlr_output_head_v1::AdaptiveSyncState::Enabled) => {
                        AdaptiveSync::Always
                    }
                    _ => AdaptiveSync::Disabled,
                });
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_mode_v1::Event;

        let id = proxy.id();
        if let Event::Finished = event {
            if proxy.version() >= 3 {
                proxy.release();
            }
            state.modes.retain(|m| m.proxy.id() != id);
            for head in &mut state.heads {
                head.modes.retain(|m| m != &id);
                if head.current_mode.as_ref() == Some(&id) {
                    head.current_mode = None;
                }
            }
            return;
        }
        let Some(mode) = state.mode_mut(&id) else {
            return;
        };
        match event {
            Event::Size { width, height } => {
                mode.width = to_u32(width);
                mode.height = to_u32(height);
            }
            Event::Refresh { refresh } => mode.refresh_mhz = to_u32(refresh),
            Event::Preferred => mode.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.result = match event {
            zwlr_output_configuration_v1::Event::Succeeded => Some(ConfigResult::Succeeded),
            zwlr_output_configuration_v1::Event::Failed => Some(ConfigResult::Failed),
            zwlr_output_configuration_v1::Event::Cancelled => Some(ConfigResult::Cancelled),
            _ => return,
        };
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: zwlr_output_configuration_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The interface has no events.
    }
}

impl Dispatch<ZcosmicOutputManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZcosmicOutputManagerV1,
        _: zcosmic_output_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The interface has no events.
    }
}

impl Dispatch<ZcosmicOutputHeadV1, ObjectId> for State {
    fn event(
        state: &mut Self,
        _: &ZcosmicOutputHeadV1,
        event: zcosmic_output_head_v1::Event,
        data: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zcosmic_output_head_v1::{AdaptiveSyncStateExt, Event};

        // `data` is the id of the wlr head this extension object belongs to.
        let Some(head) = state.head_mut(data) else {
            return;
        };
        match event {
            Event::Scale1000 { scale_1000 } => {
                head.scale_1000 = u32::try_from(scale_1000).ok();
            }
            Event::Mirroring { name } => head.mirroring = name,
            Event::AdaptiveSyncExt { state: sync } => {
                head.adaptive_sync_ext = Some(match sync {
                    WEnum::Value(AdaptiveSyncStateExt::Automatic) => AdaptiveSync::Automatic,
                    WEnum::Value(AdaptiveSyncStateExt::Always) => AdaptiveSync::Always,
                    _ => AdaptiveSync::Disabled,
                });
            }
            Event::XwaylandPrimary { state: primary } => head.xwayland_primary = primary != 0,
            _ => {}
        }
    }
}

impl Dispatch<ZcosmicOutputConfigurationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZcosmicOutputConfigurationV1,
        _: zcosmic_output_configuration_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `finished` only tells us the extension object is inert; the
        // result arrives on the wlr configuration it extends.
    }
}

impl Dispatch<ZcosmicOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZcosmicOutputConfigurationHeadV1,
        _: zcosmic_output_configuration_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The interface has no events.
    }
}

/// Explains a failed Wayland connection. A missing `libwayland-client` is
/// called out by name: the GUI toolkit makes the binary load it at run
/// time, and builds outside a Nix environment cannot find it on NixOS.
fn no_session(error: &wayland_client::ConnectError) -> BackendError {
    match error {
        wayland_client::ConnectError::NoWaylandLib => BackendError::unavailable(
            "libwayland-client.so could not be loaded; install it, or on NixOS use the packaged \
             build or put wayland's lib directory on LD_LIBRARY_PATH",
        ),
        other => BackendError::unavailable(format!("no Wayland session: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_round_trip_through_wl_output() {
        for transform in Transform::ALL {
            assert_eq!(
                from_wl_transform(WEnum::Value(to_wl_transform(transform))),
                transform
            );
        }
    }
}
