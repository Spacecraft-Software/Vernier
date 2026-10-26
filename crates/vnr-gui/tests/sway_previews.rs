// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end test of live previews against a headless sway 1.11 or newer
//! (the first with `ext-image-copy-capture-v1`):
//!
//! ```nushell
//! with-env { WLR_BACKENDS: headless, WLR_RENDERER: pixman, WLR_LIBINPUT_NO_DEVICES: 1 } { sway -c /dev/null }
//! VNR_PREVIEW_TEST_DISPLAY=wayland-1 cargo test -p vnr-gui --test sway_previews -- --ignored
//! ```
//!
//! A red square anchored to the top-left corner of the first output marks
//! which way is up. For each of the eight transforms, captured through
//! `ext-image-copy-capture-v1` and again through `wlr-screencopy` (the
//! protocol niri offers), the thumbnail must have the transformed output's
//! shape and show the square in its top-left corner and nowhere else.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::TryRecvError;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    registry_handlers,
};
use vnr_core::{Backend, Position, Transform};
use vnr_gui::preview::{self, Event, Method, Thumbnail};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

const DISPLAY_ENV: &str = "VNR_PREVIEW_TEST_DISPLAY";
/// Logical size of the marker square.
const MARK: u32 = 96;
const TRANSFORMS: [Transform; 8] = [
    Transform::Normal,
    Transform::Rotate90,
    Transform::Rotate180,
    Transform::Rotate270,
    Transform::Flipped,
    Transform::Flipped90,
    Transform::Flipped180,
    Transform::Flipped270,
];

fn display() -> String {
    std::env::var(DISPLAY_ENV)
        .unwrap_or_else(|_| panic!("set {DISPLAY_ENV} to a headless sway's socket"))
}

fn connect(display: &str) -> Connection {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR is set");
    let stream = std::os::unix::net::UnixStream::connect(std::path::Path::new(&dir).join(display))
        .expect("reach the compositor");
    Connection::from_socket(stream).expect("a Wayland connection")
}

/// Shows the marker until the returned flag is set.
fn show_marker(display: &str) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&done);
    let conn = connect(display);
    thread::spawn(move || {
        let (globals, mut queue) = registry_queue_init::<Marker>(&conn).unwrap();
        let qh = queue.handle();
        let compositor = CompositorState::bind(&globals, &qh).unwrap();
        let layers = LayerShell::bind(&globals, &qh).unwrap();
        let shm = Shm::bind(&globals, &qh).unwrap();
        let pool = SlotPool::new((MARK * MARK * 4) as usize, &shm).unwrap();
        let surface = layers.create_layer_surface(
            &qh,
            compositor.create_surface(&qh),
            Layer::Overlay,
            Some("vernier-test-marker"),
            None,
        );
        surface.set_anchor(Anchor::TOP | Anchor::LEFT);
        surface.set_exclusive_zone(-1);
        surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.set_size(MARK, MARK);
        surface.commit();
        let mut marker = Marker {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            shm,
            pool,
            surface,
        };
        while !stop.load(Ordering::Relaxed) {
            queue.blocking_dispatch(&mut marker).unwrap();
        }
    });
    done
}

struct Marker {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    surface: LayerSurface,
}

impl LayerShellHandler for Marker {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {}

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        let side = i32::try_from(MARK).unwrap();
        let (buffer, canvas) = self
            .pool
            .create_buffer(side, side, side * 4, wl_shm::Format::Xrgb8888)
            .unwrap();
        for pixel in canvas.as_chunks_mut::<4>().0 {
            *pixel = [0, 0, 0xFF, 0xFF];
        }
        let surface = self.surface.wl_surface();
        surface.damage_buffer(0, 0, side, side);
        buffer.attach_to(surface).unwrap();
        self.surface.commit();
    }
}

impl CompositorHandler for Marker {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Marker {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Marker {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Marker {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Marker);
delegate_output!(Marker);
delegate_shm!(Marker);
delegate_layer!(Marker);
delegate_registry!(Marker);

/// Whether the compositor draws clients on flipped outputs. sway 1.9 with
/// the pixman renderer leaves them blank (grim sees the same), so flips are
/// checked only on sway newer than 1.9 (1.12 is tested), or when the
/// version cannot be read.
fn draws_flipped_outputs() -> bool {
    let Ok(out) = std::process::Command::new("swaymsg")
        .args(["-t", "get_version", "--raw"])
        .output()
    else {
        return true;
    };
    let Ok(version) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return true;
    };
    let (major, minor) = (version["major"].as_u64(), version["minor"].as_u64());
    !matches!((major, minor), (Some(1), Some(m)) if m < 10)
}

fn is_red(pixel: Option<[u8; 4]>) -> bool {
    matches!(pixel, Some([r, g, b, _]) if r > 200 && g < 60 && b < 60)
}

/// The first thumbnail of `name` that shows the marker, within 10 s.
fn marked_frame(display: &str, name: &str, method: Method) -> Thumbnail {
    let mut events = preview::start_on(Some(display.to_owned()), method);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(Event::Frame { output, thumbnail }) if output == name => {
                let any_red = (0..thumbnail.height)
                    .any(|y| (0..thumbnail.width).any(|x| is_red(thumbnail.pixel(x, y))));
                if any_red {
                    return thumbnail;
                }
            }
            Ok(Event::Unavailable(why)) => panic!("previews unavailable: {why}"),
            Ok(Event::Frame { .. }) => {}
            Err(TryRecvError::Closed) => panic!("the capture thread stopped"),
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(50)),
        }
    }
    panic!("no frame of {name} showed the marker within 10 s");
}

#[test]
#[ignore = "needs a headless sway 1.11+; see the module documentation"]
fn previews_show_each_output_upright() {
    let display = display();
    let mut backend = vnr_wlr::WlrBackend::connect_to(Some(&display)).unwrap();
    let layout = backend.snapshot().unwrap();
    let first = layout.outputs.first().expect("an output").clone();
    let mode = first.mode.expect("a current mode");
    let only_first = |transform| {
        let mut want = layout.clone();
        for output in &mut want.outputs {
            output.enabled = output.name == first.name;
            output.position = Position::new(0, 0);
            output.scale = vnr_core::Scale::ONE;
            output.transform = transform;
        }
        want
    };
    // The marker goes on the focused output, so show it once that is the
    // only one; a layer surface closes when its output is turned off.
    backend.apply(&only_first(Transform::Normal)).unwrap();
    thread::sleep(Duration::from_millis(300));
    let marker = show_marker(&display);
    let flips = draws_flipped_outputs();

    for (method, transform) in [Method::Best, Method::Screencopy]
        .into_iter()
        .flat_map(|m| TRANSFORMS.map(|t| (m, t)))
        .filter(|(_, t)| flips || !t.as_str().starts_with("flipped"))
    {
        backend.apply(&only_first(transform)).unwrap();
        // Let the compositor lay the marker out on the new orientation.
        thread::sleep(Duration::from_millis(300));

        let t = marked_frame(&display, &first.name, method);
        let (w, h) = if transform.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        assert_eq!(
            u64::from(t.width) * u64::from(h) / u64::from(w),
            u64::from(t.height),
            "{method:?} {transform:?}: a {}x{} thumbnail of a {w}x{h} output",
            t.width,
            t.height
        );
        let (x1, y1) = (t.width - 3, t.height - 3);
        assert!(
            is_red(t.pixel(2, 2)),
            "{method:?} {transform:?}: marker top left"
        );
        assert!(
            !is_red(t.pixel(x1, 2)),
            "{method:?} {transform:?}: top right is clear"
        );
        assert!(
            !is_red(t.pixel(2, y1)),
            "{method:?} {transform:?}: bottom left is clear"
        );
        assert!(
            !is_red(t.pixel(x1, y1)),
            "{method:?} {transform:?}: bottom right is clear"
        );
    }

    marker.store(true, Ordering::Relaxed);
    backend.apply(&layout).unwrap();
}
