// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Output-name labels on the physical screens, as in wdisplays.
//!
//! While the editor is open, every output shows a small label in its top
//! left corner with its connector name, make and model, so it is obvious
//! which rectangle on the canvas is which screen. The label of the output
//! selected in the editor is outlined in the focus color.
//!
//! Labels are `zwlr_layer_shell_v1` surfaces on the overlay layer with an
//! empty input region, so they never take a click or a key. Compositors
//! without layer-shell (GNOME) and X11 sessions simply get no labels.
//!
//! Concurrency: the labels run on their own thread with their own Wayland
//! connection and a calloop event loop. The editor talks to it only through
//! a channel (the selected output's name); dropping [`Overlays`] closes the
//! channel and the thread exits. Nothing is shared, so nothing is locked.

use std::thread;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Event, Sender};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopSignal};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
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
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use vnr_theme::{Rgb, Steelbore};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

/// Logical pixels between a label and the screen corner.
const MARGIN: i32 = 24;
/// Logical padding inside a label.
const PADDING: f32 = 16.0;
/// Heading size for the connector name.
const NAME_SIZE: f32 = 30.0;
/// Size of the make and model line.
const DETAIL_SIZE: f32 = 15.0;
/// Heading font (Steelbore Standard §12), falling back to any monospace.
const HEADING_FONT: &str = "Share Tech Mono";

/// The running labels. Dropping this removes them.
pub struct Overlays {
    sender: Sender<Option<String>>,
}

impl std::fmt::Debug for Overlays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Overlays")
    }
}

impl Overlays {
    /// Starts the labels if the session offers layer-shell; `None` when it
    /// does not or the thread cannot start. Never blocks the caller.
    pub fn start(tokens: Steelbore) -> Option<Self> {
        std::env::var_os("WAYLAND_DISPLAY")?;
        let (sender, receiver) = channel::channel();
        thread::Builder::new()
            .name("vernier-overlays".into())
            .spawn(move || {
                if let Err(error) = run(&tokens, receiver) {
                    eprintln!("[INFO] screen labels unavailable: {error}");
                }
            })
            .ok()?;
        Some(Self { sender })
    }

    /// Outlines the label of the output called `name`, or none.
    pub fn select(&self, name: Option<&str>) {
        // A closed channel means the thread already stopped; nothing to do.
        let _ = self.sender.send(name.map(str::to_owned));
    }
}

fn run(tokens: &Steelbore, receiver: channel::Channel<Option<String>>) -> Result<(), String> {
    let painter = Painter::new(tokens.clone()).ok_or("no fonts are installed")?;
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, queue) = registry_queue_init::<Labels>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|e| format!("no layer-shell support: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let pool = SlotPool::new(256 * 64 * 4, &shm).map_err(|e| e.to_string())?;

    let mut event_loop: EventLoop<Labels> = EventLoop::try_new().map_err(|e| e.to_string())?;
    WaylandSource::new(conn, queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(receiver, |event, (), labels: &mut Labels| match event {
            Event::Msg(selected) => labels.select(selected),
            Event::Closed => labels.stop.stop(),
        })
        .map_err(|e| e.to_string())?;

    let mut labels = Labels {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        painter,
        shown: Vec::new(),
        selected: None,
        stop: event_loop.get_signal(),
    };
    event_loop
        .run(None, &mut labels, |_| {})
        .map_err(|e| e.to_string())
}

/// One output's label.
struct Label {
    output: wl_output::WlOutput,
    surface: LayerSurface,
    name: String,
    detail: String,
    scale: i32,
    /// Logical size, fixed when the label is created.
    size: (u32, u32),
    configured: bool,
}

struct Labels {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    painter: Painter,
    shown: Vec<Label>,
    selected: Option<String>,
    stop: LoopSignal,
}

impl Labels {
    fn select(&mut self, selected: Option<String>) {
        if self.selected != selected {
            self.selected = selected;
            for i in 0..self.shown.len() {
                self.draw(i);
            }
        }
    }

    fn add(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some(info) = self.outputs.info(&output) else {
            return;
        };
        let (name, detail) = texts(&info);
        let size = self.painter.measure(&name, &detail);
        let surface = self.compositor.create_surface(qh);
        // An empty input region lets every click and touch through.
        if let Ok(region) = Region::new(&self.compositor) {
            surface.set_input_region(Some(region.wl_region()));
        }
        let surface = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("vernier-label"),
            Some(&output),
        );
        surface.set_anchor(Anchor::TOP | Anchor::LEFT);
        surface.set_margin(MARGIN, 0, 0, MARGIN);
        surface.set_exclusive_zone(-1);
        surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.set_size(size.0, size.1);
        surface.commit();
        self.shown.push(Label {
            output,
            surface,
            name,
            detail,
            scale: info.scale_factor.max(1),
            size,
            configured: false,
        });
    }

    fn draw(&mut self, index: usize) {
        let Some(label) = self.shown.get(index) else {
            return;
        };
        if !label.configured {
            return;
        }
        let scale = label.scale;
        let (width, height) = (
            label.size.0 * scale.unsigned_abs(),
            label.size.1 * scale.unsigned_abs(),
        );
        let selected = self.selected.as_deref() == Some(label.name.as_str());
        let (name, detail) = (label.name.clone(), label.detail.clone());
        let Some(pixmap) = self
            .painter
            .render(width, height, scale, selected, &name, &detail)
        else {
            return;
        };
        let (Ok(w), Ok(h)) = (i32::try_from(width), i32::try_from(height)) else {
            return;
        };
        let Ok((buffer, canvas)) = self
            .pool
            .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
        else {
            return;
        };
        // tiny-skia is premultiplied RGBA; wl_shm ARGB8888 is premultiplied
        // BGRA in memory on little-endian machines.
        for (dst, src) in canvas
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(pixmap.data().as_chunks::<4>().0)
        {
            *dst = [src[2], src[1], src[0], src[3]];
        }
        let Some(label) = self.shown.get(index) else {
            return;
        };
        let surface = label.surface.wl_surface();
        let _ = label.surface.set_buffer_scale(scale.unsigned_abs());
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            label.surface.commit();
        }
    }

    fn index_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.shown
            .iter()
            .position(|l| l.surface.wl_surface() == surface)
    }
}

/// Draws labels; independent of Wayland so it can be tested.
struct Painter {
    fonts: FontSystem,
    glyphs: SwashCache,
    tokens: Steelbore,
}

impl Painter {
    /// A painter over the system's fonts; `None` when none are installed,
    /// because shaping text without any font panics inside cosmic-text,
    /// and a panic here would abort the whole editor.
    fn new(tokens: Steelbore) -> Option<Self> {
        Self::with_fonts(FontSystem::new(), tokens)
    }

    fn with_fonts(fonts: FontSystem, tokens: Steelbore) -> Option<Self> {
        (!fonts.db().is_empty()).then(|| Self {
            fonts,
            glyphs: SwashCache::new(),
            tokens,
        })
    }

    /// The label's logical size for these two lines.
    fn measure(&mut self, name: &str, detail: &str) -> (u32, u32) {
        let mut width: f32 = 0.0;
        let mut height = PADDING * 2.0;
        for (text, size) in [(name, NAME_SIZE), (detail, DETAIL_SIZE)] {
            if text.is_empty() {
                continue;
            }
            let buffer = self.shape(text, size, 1.0);
            for run in buffer.layout_runs() {
                width = width.max(run.line_w);
            }
            height += size * 1.25;
        }
        (to_px(width + PADDING * 2.0), to_px(height))
    }

    fn shape(&mut self, text: &str, size: f32, scale: f32) -> Buffer {
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(size * scale, size * 1.25 * scale),
        );
        let attrs = if size >= NAME_SIZE {
            Attrs::new()
                .family(Family::Name(HEADING_FONT))
                .weight(Weight::BOLD)
        } else {
            Attrs::new().family(Family::Monospace)
        };
        buffer.set_size(&mut self.fonts, None, None);
        buffer.set_text(&mut self.fonts, text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    fn render(
        &mut self,
        width: u32,
        height: u32,
        scale: i32,
        selected: bool,
        name: &str,
        detail: &str,
    ) -> Option<Pixmap> {
        #[expect(clippy::cast_precision_loss, reason = "scales are small integers")]
        let s = scale as f32;
        let mut pixmap = Pixmap::new(width, height)?;
        let border = if selected { 4.0 * s } else { 2.0 * s };
        let edge = if selected {
            self.tokens.focus
        } else {
            self.tokens.structure
        };
        #[expect(clippy::cast_precision_loss, reason = "label sizes are far below 2^24")]
        let rect = Rect::from_xywh(
            border / 2.0,
            border / 2.0,
            width as f32 - border,
            height as f32 - border,
        )?;
        let path = PathBuilder::from_rect(rect);
        pixmap.fill_path(
            &path,
            &paint(self.tokens.surface),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        let stroke = Stroke {
            width: border,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint(edge), &stroke, Transform::identity(), None);

        let mut y = PADDING * s;
        for (text, size, rgb) in [
            (name, NAME_SIZE, self.tokens.foreground),
            (detail, DETAIL_SIZE, self.tokens.structure),
        ] {
            if text.is_empty() {
                continue;
            }
            let buffer = self.shape(text, size, s);
            let color = Color::rgb(rgb.r, rgb.g, rgb.b);
            let (x0, y0) = (PADDING * s, y);
            buffer.draw(&mut self.fonts, &mut self.glyphs, color, |x, y, w, h, c| {
                #[expect(clippy::cast_precision_loss, reason = "glyph coordinates are small")]
                let Some(r) = Rect::from_xywh(x0 + x as f32, y0 + y as f32, w as f32, h as f32)
                else {
                    return;
                };
                let mut paint = Paint::default();
                paint.set_color_rgba8(c.r(), c.g(), c.b(), c.a());
                paint.anti_alias = false;
                pixmap.fill_rect(r, &paint, Transform::identity(), None);
            });
            y += size * 1.25 * s;
        }
        Some(pixmap)
    }
}

/// The two lines of a label: connector name, then make and model.
fn texts(info: &OutputInfo) -> (String, String) {
    let name = info
        .name
        .clone()
        .unwrap_or_else(|| format!("output {}", info.id));
    let detail = [info.make.as_str(), info.model.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty() && *s != "Unknown")
        .collect::<Vec<_>>()
        .join(" ");
    (name, detail)
}

fn paint(rgb: Rgb) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb.r, rgb.g, rgb.b, 0xFF);
    paint
}

/// Rounds a logical length up to whole pixels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "label lengths are small and positive"
)]
fn to_px(length: f32) -> u32 {
    length.ceil().max(1.0) as u32
}

impl CompositorHandler for Labels {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(i) = self.index_of(surface) {
            self.shown[i].scale = new_factor.max(1);
            self.draw(i);
        }
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

impl OutputHandler for Labels {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add(qh, output);
    }

    fn update_output(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // A rename or rescale: rebuild that output's label.
        self.shown.retain(|l| l.output != output);
        self.add(qh, output);
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.shown.retain(|l| l.output != output);
    }
}

impl LayerShellHandler for Labels {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.shown.retain(|l| &l.surface != layer);
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(i) = self.shown.iter().position(|l| &l.surface == layer) {
            self.shown[i].configured = true;
            self.draw(i);
        }
    }
}

impl ShmHandler for Labels {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Labels {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Labels);
delegate_output!(Labels);
delegate_shm!(Labels);
delegate_layer!(Labels);
delegate_registry!(Labels);

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixmap: &Pixmap, x: u32, y: u32) -> Rgb {
        let p = pixmap.pixel(x, y).expect("inside the pixmap").demultiply();
        Rgb::new(p.red(), p.green(), p.blue())
    }

    /// A painter with only the bundled test font, so the result does not
    /// depend on the fonts of the machine running the tests (build
    /// sandboxes have none).
    fn painter(tokens: Steelbore) -> Painter {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(include_bytes!("../tests/fonts/Inconsolata-Regular.ttf").to_vec());
        let fonts = FontSystem::new_with_locale_and_db("en-US".to_owned(), db);
        Painter::with_fonts(fonts, tokens).expect("the test font loads")
    }

    fn tokens() -> Steelbore {
        vnr_theme::resolve(&vnr_theme::Request::default()).tokens
    }

    #[test]
    fn a_label_has_a_border_a_fill_and_text() {
        let tokens = tokens();
        let mut painter = painter(tokens.clone());
        let (w, h) = painter.measure("DP-1", "Dell U2723QE");
        assert!(w > 2 * to_px(PADDING) && h > to_px(NAME_SIZE), "{w}x{h}");

        for (selected, edge) in [(false, tokens.structure), (true, tokens.focus)] {
            let pixmap = painter
                .render(w, h, 1, selected, "DP-1", "Dell U2723QE")
                .expect("pixmap");
            assert_eq!(pixel(&pixmap, w / 2, 1), edge, "selected: {selected}");
            assert_eq!(pixel(&pixmap, w - 8, h - 8), tokens.surface);
            let text = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| pixel(&pixmap, x, y) == tokens.foreground)
                .count();
            assert!(text > 20, "too few text pixels: {text}");
        }
    }

    #[test]
    fn doubled_scale_doubles_the_pixels() {
        let mut painter = painter(tokens());
        let (w, h) = painter.measure("eDP-1", "");
        let pixmap = painter
            .render(w * 2, h * 2, 2, false, "eDP-1", "")
            .expect("pixmap");
        assert_eq!((pixmap.width(), pixmap.height()), (w * 2, h * 2));
    }

    #[test]
    fn no_fonts_means_no_labels_instead_of_a_panic() {
        let empty = FontSystem::new_with_locale_and_db(
            "en-US".to_owned(),
            cosmic_text::fontdb::Database::new(),
        );
        assert!(Painter::with_fonts(empty, tokens()).is_none());
    }

    #[test]
    fn to_px_rounds_up() {
        assert_eq!(to_px(10.2), 11);
        assert_eq!(to_px(0.0), 1);
    }
}
