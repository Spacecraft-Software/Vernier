// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Live screen previews, as wdisplays' "Show Screen Contents".
//!
//! While previews are on, every output is captured into a shared-memory
//! buffer, shrunk to a thumbnail no larger than [`MAX_EDGE`] pixels, turned
//! upright, and sent to the editor, which draws it inside the output's
//! rectangle. Capture uses `ext-image-copy-capture-v1` (sway 1.11+, COSMIC)
//! and falls back to `wlr-screencopy-unstable-v1` (niri, Hyprland).
//! Compositors with neither (`KWin`, Mutter) and X11 sessions report
//! [`Event::Unavailable`] once and capture nothing.
//!
//! Cost: a capture at most every [`INTERVAL`] per output, and after the
//! first picture the compositor only answers once the screen has changed,
//! so a still screen costs nothing. The thumbnail reads four samples per thumbnail pixel,
//! never the whole buffer.
//!
//! Concurrency: as the screen labels, the capture runs on its own thread
//! with its own Wayland connection and a calloop event loop. It sends
//! thumbnails through a bounded channel and stops when the editor drops
//! the receiving end; nothing is shared, so nothing is locked.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use iced::futures::channel::mpsc;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_output, delegate_registry, delegate_shm, registry_handlers};
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_shm};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// Longest edge of a thumbnail in pixels. The canvas rarely draws an
/// output larger than this, and four samples per pixel stay cheap.
pub const MAX_EDGE: u32 = 640;

/// The shortest time between two captures of the same output.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// How often the thread checks that the editor still listens, so it stops
/// even while every screen is still and no capture completes.
const LISTENING_CHECK: Duration = Duration::from_millis(500);

/// Thumbnails waiting for the editor; older ones are dropped beyond this.
const QUEUE: usize = 16;

/// What the capture thread tells the editor.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new picture of an output.
    Frame {
        /// The output's connector name.
        output: String,
        /// The picture, upright.
        thumbnail: Thumbnail,
    },
    /// Previews cannot work in this session, and why.
    Unavailable(String),
}

/// A small upright RGBA picture of one output.
#[derive(Clone, PartialEq, Eq)]
pub struct Thumbnail {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Rows of opaque RGBA pixels, top to bottom, no padding.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Thumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Thumbnail({}x{})", self.width, self.height)
    }
}

impl Thumbnail {
    /// The RGBA bytes of the pixel at `(x, y)`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(at..at + 4)?.try_into().ok()
    }
}

/// Which capture protocol to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Method {
    /// `ext-image-copy-capture-v1` where offered, otherwise
    /// `wlr-screencopy-unstable-v1` (niri, Hyprland).
    #[default]
    Best,
    /// Only `wlr-screencopy-unstable-v1`, even where the newer protocol is
    /// offered. For tests.
    Screencopy,
}

/// Starts capturing on the session's compositor (`WAYLAND_DISPLAY`) and
/// returns the stream of events. Dropping the stream stops the thread
/// within [`LISTENING_CHECK`].
#[must_use]
pub fn start() -> mpsc::Receiver<Event> {
    start_on(None, Method::Best)
}

/// As [`start`], on the compositor socket `display` (a name inside
/// `XDG_RUNTIME_DIR`, or an absolute path) when given, with `method`. For
/// tests.
#[must_use]
pub fn start_on(display: Option<String>, method: Method) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(QUEUE);
    let mut report = sender.clone();
    let spawned = thread::Builder::new()
        .name("vernier-previews".into())
        .spawn(move || {
            if let Err(error) = run(display.as_deref(), method, sender) {
                let _ = report.try_send(Event::Unavailable(error));
            }
        });
    if let Err(error) = spawned {
        let (mut sender, receiver) = mpsc::channel(1);
        let _ = sender.try_send(Event::Unavailable(format!(
            "cannot start the capture thread: {error}"
        )));
        return receiver;
    }
    receiver
}

fn connect(display: Option<&str>) -> Result<Connection, String> {
    let Some(name) = display else {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err("screen contents need a Wayland session".to_owned());
        }
        return Connection::connect_to_env().map_err(|e| e.to_string());
    };
    let path = PathBuf::from(name);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is not set")?
            .join(name)
    };
    let stream = UnixStream::connect(&path)
        .map_err(|e| format!("cannot reach `{}`: {e}", path.display()))?;
    Connection::from_socket(stream).map_err(|e| e.to_string())
}

/// The two managers of `ext-image-copy-capture-v1`.
struct ExtManagers {
    sources: ExtOutputImageCaptureSourceManagerV1,
    copier: ExtImageCopyCaptureManagerV1,
}

/// The bound capture protocols. Each output uses ext when bound and
/// screencopy otherwise, or when ext offers it no readable buffer format.
struct Copier {
    ext: Option<ExtManagers>,
    wlr: Option<ZwlrScreencopyManagerV1>,
}

impl Copier {
    fn bind(
        globals: &GlobalList,
        qh: &QueueHandle<Capturer>,
        method: Method,
    ) -> Result<Self, String> {
        let ext = (method == Method::Best)
            .then(|| {
                let sources = globals.bind(qh, 1..=1, ()).ok()?;
                let copier = globals.bind(qh, 1..=1, ()).ok()?;
                Some(ExtManagers { sources, copier })
            })
            .flatten();
        let wlr = globals.bind(qh, 1..=3, ()).ok();
        if ext.is_none() && wlr.is_none() {
            return Err(
                "the compositor offers neither ext-image-copy-capture-v1 nor \
                        wlr-screencopy-unstable-v1"
                    .to_owned(),
            );
        }
        Ok(Self { ext, wlr })
    }
}

fn run(display: Option<&str>, method: Method, sender: mpsc::Sender<Event>) -> Result<(), String> {
    let conn = connect(display)?;
    let (globals, queue) = registry_queue_init::<Capturer>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let copier = Copier::bind(&globals, &qh, method)?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let pool = SlotPool::new(4096, &shm).map_err(|e| e.to_string())?;

    let mut event_loop: EventLoop<Capturer> = EventLoop::try_new().map_err(|e| e.to_string())?;
    WaylandSource::new(conn, queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(Timer::from_duration(LISTENING_CHECK), |_, (), c| {
            if c.sender.is_closed() {
                c.stop.stop();
            }
            TimeoutAction::ToDuration(LISTENING_CHECK)
        })
        .map_err(|e| e.to_string())?;

    let mut capturer = Capturer {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        shm,
        pool,
        copier,
        captures: Vec::new(),
        sender,
        stop: event_loop.get_signal(),
        timers: event_loop.handle(),
        qh,
    };
    event_loop
        .run(None, &mut capturer, |_| {})
        .map_err(|e| e.to_string())
}

/// The shape of the buffer a capture needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Shape {
    size: Option<(u32, u32)>,
    /// Bytes per row; `None` means four per pixel, no padding.
    stride: Option<u32>,
    /// The format exactly as advertised: buffers must use it, not merely
    /// one with the same byte order.
    format: Option<wl_shm::Format>,
}

impl Shape {
    /// Width, height, stride and format, once all are known.
    fn known(self) -> Option<(u32, u32, u32, wl_shm::Format)> {
        let (width, height) = self.size?;
        let stride = self.stride.or_else(|| width.checked_mul(4))?;
        Some((width, height, stride, self.format?))
    }
}

/// The two byte orders every compositor offers for shared memory, and the
/// only two read here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `ARGB8888` and `XRGB8888`: B, G, R, A bytes in memory.
    Bgra,
    /// `ABGR8888` and `XBGR8888`: R, G, B, A bytes in memory.
    Rgba,
}

impl ByteOrder {
    fn of(format: wl_shm::Format) -> Option<Self> {
        match format {
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => Some(Self::Bgra),
            wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => Some(Self::Rgba),
            _ => None,
        }
    }
}

/// A capture in flight.
enum Frame {
    Ext(ExtImageCopyCaptureFrameV1),
    Wlr(ZwlrScreencopyFrameV1),
}

impl Frame {
    fn destroy(&self) {
        match self {
            Self::Ext(frame) => frame.destroy(),
            Self::Wlr(frame) => frame.destroy(),
        }
    }
}

/// One output being captured.
struct Capture {
    output: wl_output::WlOutput,
    name: String,
    /// The ext session; screencopy has none and asks per frame.
    session: Option<ExtImageCopyCaptureSessionV1>,
    /// The shape being announced; it takes effect at `done`.
    incoming: Shape,
    /// The shape of `buffer`.
    current: Shape,
    buffer: Option<Buffer>,
    frame: Option<Frame>,
    transform: wl_output::Transform,
    /// Rows run bottom to top (screencopy only).
    y_invert: bool,
    /// A picture was taken before; later screencopy frames wait for damage.
    taken: bool,
    /// A timer will start the next capture.
    waiting: bool,
}

struct Capturer {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    copier: Copier,
    captures: Vec<Capture>,
    sender: mpsc::Sender<Event>,
    stop: LoopSignal,
    timers: LoopHandle<'static, Capturer>,
    qh: QueueHandle<Capturer>,
}

impl Capturer {
    fn add(&mut self, output: wl_output::WlOutput) {
        let Some(name) = self.outputs.info(&output).and_then(|info| info.name) else {
            return;
        };
        let session = self.copier.ext.as_ref().map(|ext| {
            let source = ext.sources.create_source(&output, &self.qh, ());
            let session =
                ext.copier
                    .create_session(&source, Options::empty(), &self.qh, output.clone());
            // The session keeps what it needs; the source can go.
            source.destroy();
            session
        });
        let starts_now = session.is_none();
        self.captures.push(Capture {
            output,
            name,
            session,
            incoming: Shape::default(),
            current: Shape::default(),
            buffer: None,
            frame: None,
            transform: wl_output::Transform::Normal,
            y_invert: false,
            taken: false,
            waiting: false,
        });
        // An ext session starts at its first `done`; screencopy now.
        if starts_now {
            self.capture(self.captures.len() - 1);
        }
    }

    fn remove(&mut self, output: &wl_output::WlOutput) {
        self.captures.retain(|c| {
            if &c.output == output {
                if let Some(frame) = &c.frame {
                    frame.destroy();
                }
                if let Some(session) = &c.session {
                    session.destroy();
                }
                false
            } else {
                true
            }
        });
    }

    fn index(&self, output: &wl_output::WlOutput) -> Option<usize> {
        self.captures.iter().position(|c| &c.output == output)
    }

    /// Makes sure output `index` has a buffer of its current shape.
    fn buffer(&mut self, index: usize) -> Option<(i32, i32)> {
        let capture = self.captures.get_mut(index)?;
        let (width, height, stride, format) = capture.current.known()?;
        let (w, h, s) = (
            i32::try_from(width).ok()?,
            i32::try_from(height).ok()?,
            i32::try_from(stride).ok()?,
        );
        if capture.buffer.is_none() {
            let (buffer, _) = self.pool.create_buffer(w, h, s, format).ok()?;
            capture.buffer = Some(buffer);
        }
        Some((w, h))
    }

    /// Starts a capture of output `index` unless one is running or due.
    fn capture(&mut self, index: usize) {
        let Some(capture) = self.captures.get_mut(index) else {
            return;
        };
        if capture.frame.is_some() || capture.waiting {
            return;
        }
        capture.y_invert = false;
        if capture.session.is_none() {
            if let Some(manager) = &self.copier.wlr {
                // The frame announces the buffer it needs; see `copy`.
                let frame =
                    manager.capture_output(0, &capture.output, &self.qh, capture.output.clone());
                capture.frame = Some(Frame::Wlr(frame));
            }
            return;
        }
        let Some((w, h)) = self.buffer(index) else {
            return;
        };
        let capture = &mut self.captures[index];
        let (Some(session), Some(buffer)) = (&capture.session, &capture.buffer) else {
            return;
        };
        let frame = session.create_frame(&self.qh, capture.output.clone());
        frame.attach_buffer(buffer.wl_buffer());
        frame.damage_buffer(0, 0, w, h);
        frame.capture();
        capture.frame = Some(Frame::Ext(frame));
    }

    /// Moves output `index` from its ext session to screencopy, when the
    /// session offers no buffer format this module reads. Without
    /// screencopy the output simply shows no picture.
    fn fall_back(&mut self, index: usize) {
        if self.copier.wlr.is_none() {
            return;
        }
        let Some(capture) = self.captures.get_mut(index) else {
            return;
        };
        if let Some(frame) = capture.frame.take() {
            frame.destroy();
        }
        if let Some(session) = capture.session.take() {
            session.destroy();
        }
        capture.current = Shape::default();
        capture.buffer = None;
        self.capture(index);
    }

    /// Copies a screencopy frame once its buffer shape is announced.
    fn copy(&mut self, index: usize) {
        let Some(capture) = self.captures.get_mut(index) else {
            return;
        };
        let next = std::mem::take(&mut capture.incoming);
        if next != capture.current {
            capture.buffer = None;
        }
        capture.current = next;
        if self.buffer(index).is_none() {
            // No readable format: give up on this frame, try again later.
            self.drop_frame(index);
            self.later(index);
            return;
        }
        let capture = &mut self.captures[index];
        let (Some(Frame::Wlr(frame)), Some(buffer)) = (&capture.frame, &capture.buffer) else {
            return;
        };
        if capture.taken && frame.version() >= 2 {
            frame.copy_with_damage(buffer.wl_buffer());
        } else {
            frame.copy(buffer.wl_buffer());
        }
    }

    fn drop_frame(&mut self, index: usize) {
        if let Some(frame) = self.captures.get_mut(index).and_then(|c| c.frame.take()) {
            frame.destroy();
        }
    }

    /// Captures output `index` again after [`INTERVAL`].
    fn later(&mut self, index: usize) {
        let Some(capture) = self.captures.get_mut(index) else {
            return;
        };
        capture.waiting = true;
        let output = capture.output.clone();
        let _ = self
            .timers
            .insert_source(Timer::from_duration(INTERVAL), move |_, (), c| {
                if let Some(i) = c.index(&output) {
                    c.captures[i].waiting = false;
                    c.capture(i);
                }
                TimeoutAction::Drop
            });
    }

    fn ready(&mut self, index: usize) {
        self.drop_frame(index);
        // Screencopy buffers are in the output's own orientation.
        let output_transform = self
            .captures
            .get(index)
            .and_then(|c| self.outputs.info(&c.output))
            .map(|info| info.transform);
        let Some(capture) = self.captures.get_mut(index) else {
            return;
        };
        capture.taken = true;
        if capture.session.is_none() {
            capture.transform = output_transform.unwrap_or(wl_output::Transform::Normal);
        }
        let (Some((width, height, stride, format)), Some(buffer)) =
            (capture.current.known(), capture.buffer.as_ref())
        else {
            return;
        };
        let Some(order) = ByteOrder::of(format) else {
            return;
        };
        let (transform, y_invert, name) =
            (capture.transform, capture.y_invert, capture.name.clone());
        let Some(data) = self.pool.canvas(buffer) else {
            return;
        };
        let source = Source {
            data,
            width,
            height,
            stride: stride as usize,
            order,
            y_invert,
        };
        if let Some(thumbnail) = thumbnail(&source, transform, MAX_EDGE) {
            let sent = self.sender.try_send(Event::Frame {
                output: name,
                thumbnail,
            });
            if sent
                .as_ref()
                .is_err_and(mpsc::TrySendError::is_disconnected)
            {
                self.stop.stop();
                return;
            }
        }
        self.later(index);
    }
}

/// A captured buffer as it lies in shared memory.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    /// The buffer's bytes.
    pub data: &'a [u8],
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// Byte order of each pixel.
    pub order: ByteOrder,
    /// Rows run bottom to top.
    pub y_invert: bool,
}

impl Source<'_> {
    fn rgb(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        let y = if self.y_invert {
            self.height.checked_sub(y + 1)?
        } else {
            y
        };
        let at = y as usize * self.stride + x as usize * 4;
        let p = self.data.get(at..at + 4)?;
        Some(match self.order {
            ByteOrder::Bgra => [p[2], p[1], p[0]],
            ByteOrder::Rgba => [p[0], p[1], p[2]],
        })
    }
}

/// Shrinks `source` to fit in `max_edge` pixels and undoes `transform`, the
/// transform the compositor applied to the contents, so the result shows
/// the output as it appears in the layout.
///
/// Each thumbnail pixel averages a 2x2 grid of samples from its area of the
/// source, which keeps text and edges recognizable without reading every
/// source pixel. `None` when the source is empty or too short for its size.
#[must_use]
pub fn thumbnail(
    source: &Source<'_>,
    transform: wl_output::Transform,
    max_edge: u32,
) -> Option<Thumbnail> {
    let (bw, bh) = (source.width, source.height);
    if bw == 0 || bh == 0 || max_edge == 0 {
        return None;
    }
    let needed = (bh as usize - 1) * source.stride + bw as usize * 4;
    if source.data.len() < needed || source.stride < bw as usize * 4 {
        return None;
    }
    // Size of the upright picture, before shrinking.
    let (lw, lh) = if swaps_axes(transform) {
        (bh, bw)
    } else {
        (bw, bh)
    };
    let longest = lw.max(lh);
    let (tw, th) = if longest <= max_edge {
        (lw, lh)
    } else {
        (
            scale_down(lw, max_edge, longest),
            scale_down(lh, max_edge, longest),
        )
    };
    let mut rgba = Vec::with_capacity(tw as usize * th as usize * 4);
    for ty in 0..th {
        for tx in 0..tw {
            let mut sum = [0_u32; 3];
            let mut count = 0_u32;
            for (fx, fy) in [(1, 1), (3, 1), (1, 3), (3, 3)] {
                // Quarter points of this pixel's area, in upright coordinates.
                let lx = sample(tx, fx, lw, tw);
                let ly = sample(ty, fy, lh, th);
                let (bx, by) = to_buffer(transform, lx, ly, lw, lh);
                if let Some(rgb) = source.rgb(bx, by) {
                    for (s, c) in sum.iter_mut().zip(rgb) {
                        *s += u32::from(c);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            for s in sum {
                rgba.push(u8::try_from(s / count).unwrap_or(u8::MAX));
            }
            rgba.push(u8::MAX);
        }
    }
    Some(Thumbnail {
        width: tw,
        height: th,
        rgba,
    })
}

/// `length * edge / longest`, at least one.
fn scale_down(length: u32, edge: u32, longest: u32) -> u32 {
    let scaled = u64::from(length) * u64::from(edge) / u64::from(longest);
    u32::try_from(scaled).unwrap_or(edge).max(1)
}

/// The source coordinate at `quarter / 4` of thumbnail pixel `t`.
fn sample(t: u32, quarter: u64, source: u32, thumb: u32) -> u32 {
    let at = (u64::from(t) * 4 + quarter) * u64::from(source) / (u64::from(thumb) * 4);
    u32::try_from(at)
        .unwrap_or(u32::MAX)
        .min(source.saturating_sub(1))
}

const fn swaps_axes(transform: wl_output::Transform) -> bool {
    matches!(
        transform,
        wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270
    )
}

/// Where the upright pixel at `col`, `row` of a `width` x `height` picture
/// lies in a buffer whose contents were transformed by `transform`.
///
/// `wl_output` transforms turn counter-clockwise; "flipped" mirrors around
/// the vertical axis first.
const fn to_buffer(
    transform: wl_output::Transform,
    col: u32,
    row: u32,
    width: u32,
    height: u32,
) -> (u32, u32) {
    let (right, bottom) = (width - 1 - col, height - 1 - row);
    match transform {
        wl_output::Transform::_90 => (row, right),
        wl_output::Transform::_180 => (right, bottom),
        wl_output::Transform::_270 => (bottom, col),
        wl_output::Transform::Flipped => (right, row),
        wl_output::Transform::Flipped90 => (row, col),
        wl_output::Transform::Flipped180 => (col, bottom),
        wl_output::Transform::Flipped270 => (bottom, right),
        _ => (col, row),
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, wl_output::WlOutput> for Capturer {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        data: &wl_output::WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event as E;
        let Some(index) = state.index(data) else {
            return;
        };
        let capture = &mut state.captures[index];
        match event {
            E::BufferSize { width, height } => capture.incoming.size = Some((width, height)),
            E::ShmFormat {
                format: WEnum::Value(format),
            } => {
                // Keep the first readable format the compositor lists.
                if capture.incoming.format.is_none() && ByteOrder::of(format).is_some() {
                    capture.incoming.format = Some(format);
                }
            }
            E::Done => {
                let next = std::mem::take(&mut capture.incoming);
                let changed = next != capture.current;
                capture.current = next;
                let readable = next.format.is_some();
                if changed {
                    // A frame in flight was made for the old buffer: drop it
                    // before the buffer, then capture with the new shape.
                    state.drop_frame(index);
                    state.captures[index].buffer = None;
                }
                if readable {
                    state.capture(index);
                } else {
                    state.fall_back(index);
                }
            }
            E::Stopped => state.remove(data),
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, wl_output::WlOutput> for Capturer {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        data: &wl_output::WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event as E;
        let Some(index) = state.index(data) else {
            return;
        };
        match event {
            E::Transform {
                transform: WEnum::Value(transform),
            } => state.captures[index].transform = transform,
            E::Ready => state.ready(index),
            E::Failed { reason } => {
                state.drop_frame(index);
                match reason {
                    // New constraints follow; their `done` starts the next
                    // capture. Had they come first, `done` would already
                    // have dropped this frame and captured again.
                    WEnum::Value(FailureReason::BufferConstraints) => {}
                    WEnum::Value(FailureReason::Stopped) => state.remove(data),
                    _ => state.later(index),
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, wl_output::WlOutput> for Capturer {
    fn event(
        state: &mut Self,
        proxy: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        data: &wl_output::WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event as E;
        let Some(index) = state.index(data) else {
            return;
        };
        let capture = &mut state.captures[index];
        match event {
            E::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                // Keep the first readable shared-memory buffer offered.
                if capture.incoming.format.is_none() && ByteOrder::of(format).is_some() {
                    capture.incoming = Shape {
                        size: Some((width, height)),
                        stride: Some(stride),
                        format: Some(format),
                    };
                }
                // Before version 3 there is no `buffer_done`, and this is
                // the only buffer event.
                if proxy.version() < 3 {
                    state.copy(index);
                }
            }
            E::BufferDone => state.copy(index),
            E::Flags {
                flags: WEnum::Value(flags),
            } => capture.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            E::Ready { .. } => state.ready(index),
            E::Failed => {
                state.drop_frame(index);
                state.later(index);
            }
            _ => {}
        }
    }
}

/// Requests with no events: the managers and the capture source.
macro_rules! no_events {
    ($($proxy:ty),+) => {$(
        impl Dispatch<$proxy, ()> for Capturer {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as Proxy>::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )+};
}

no_events!(
    ExtOutputImageCaptureSourceManagerV1,
    ExtImageCaptureSourceV1,
    ExtImageCopyCaptureManagerV1,
    ZwlrScreencopyManagerV1
);

impl OutputHandler for Capturer {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add(output);
    }

    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Only a rename matters: the session follows mode and transform
        // changes by itself.
        let name = self.outputs.info(&output).and_then(|info| info.name);
        let known = self.index(&output).map(|i| self.captures[i].name.clone());
        if name != known {
            self.remove(&output);
            self.add(output);
        }
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.remove(&output);
    }
}

impl ShmHandler for Capturer {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Capturer {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_output!(Capturer);
delegate_shm!(Capturer);
delegate_registry!(Capturer);

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` x `h` BGRA buffer whose pixel `(x, y)` has red `x`, green `y`.
    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend([0, u8::try_from(y).unwrap(), u8::try_from(x).unwrap(), 0]);
            }
        }
        data
    }

    fn source(data: &[u8], w: u32, h: u32) -> Source<'_> {
        Source {
            data,
            width: w,
            height: h,
            stride: w as usize * 4,
            order: ByteOrder::Bgra,
            y_invert: false,
        }
    }

    #[test]
    fn inverted_rows_are_read_bottom_up() {
        let data = gradient(2, 3);
        let src = Source {
            y_invert: true,
            ..source(&data, 2, 3)
        };
        let t = thumbnail(&src, wl_output::Transform::Normal, 8).unwrap();
        assert_eq!(t.pixel(1, 0), Some([1, 2, 0, 255]));
        assert_eq!(t.pixel(1, 2), Some([1, 0, 0, 255]));
    }

    #[test]
    fn padded_rows_are_skipped() {
        // Two pixels per row, stride of three.
        let data = [
            0, 0, 1, 0, 0, 0, 2, 0, 9, 9, 9, 9, //
            0, 0, 3, 0, 0, 0, 4, 0, 9, 9, 9, 9,
        ];
        let src = Source {
            stride: 12,
            ..source(&data, 2, 2)
        };
        let t = thumbnail(&src, wl_output::Transform::Normal, 8).unwrap();
        let reds: Vec<u8> = t.rgba.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, [1, 2, 3, 4]);
    }

    #[test]
    fn small_buffers_are_copied_upright_and_opaque() {
        let data = gradient(4, 3);
        let t = thumbnail(&source(&data, 4, 3), wl_output::Transform::Normal, 640).unwrap();
        assert_eq!((t.width, t.height), (4, 3));
        assert_eq!(t.pixel(2, 1), Some([2, 1, 0, 255]));
        assert_eq!(t.pixel(4, 0), None);
    }

    #[test]
    fn rgba_buffers_keep_their_channels() {
        let data = [10, 20, 30, 0];
        let src = Source {
            order: ByteOrder::Rgba,
            ..source(&data, 1, 1)
        };
        let t = thumbnail(&src, wl_output::Transform::Normal, 8).unwrap();
        assert_eq!(t.pixel(0, 0), Some([10, 20, 30, 255]));
    }

    #[test]
    fn large_buffers_shrink_to_the_longest_edge() {
        let data = vec![0; 1920 * 1080 * 4];
        let t = thumbnail(
            &source(&data, 1920, 1080),
            wl_output::Transform::Normal,
            640,
        )
        .unwrap();
        assert_eq!((t.width, t.height), (640, 360));
        assert_eq!(t.rgba.len(), 640 * 360 * 4);
    }

    #[test]
    fn rotated_buffers_come_out_upright() {
        // A 3x2 buffer holding a 2x3 upright picture rotated 90 degrees.
        let data = gradient(3, 2);
        let t = thumbnail(&source(&data, 3, 2), wl_output::Transform::_90, 640).unwrap();
        assert_eq!((t.width, t.height), (2, 3));
        // Every upright pixel reads the buffer pixel to_buffer names.
        for y in 0..3 {
            for x in 0..2 {
                let (bx, by) = to_buffer(wl_output::Transform::_90, x, y, 2, 3);
                let want = [u8::try_from(bx).unwrap(), u8::try_from(by).unwrap(), 0, 255];
                assert_eq!(t.pixel(x, y), Some(want), "({x}, {y})");
            }
        }
    }

    #[test]
    fn every_transform_maps_the_picture_onto_the_buffer() {
        use wl_output::Transform as T;
        for transform in [
            T::Normal,
            T::_90,
            T::_180,
            T::_270,
            T::Flipped,
            T::Flipped90,
            T::Flipped180,
            T::Flipped270,
        ] {
            let (w, h) = (3, 2);
            let (bw, bh) = if swaps_axes(transform) {
                (h, w)
            } else {
                (w, h)
            };
            let mut seen = std::collections::BTreeSet::new();
            for y in 0..h {
                for x in 0..w {
                    let (bx, by) = to_buffer(transform, x, y, w, h);
                    assert!(
                        bx < bw && by < bh,
                        "{transform:?} ({x}, {y}) -> ({bx}, {by})"
                    );
                    seen.insert((bx, by));
                }
            }
            assert_eq!(seen.len(), 6, "{transform:?} is a bijection");
        }
    }

    #[test]
    fn short_or_empty_buffers_are_refused() {
        let data = vec![0; 15];
        assert!(thumbnail(&source(&data, 2, 2), wl_output::Transform::Normal, 8).is_none());
        assert!(thumbnail(&source(&[], 0, 0), wl_output::Transform::Normal, 8).is_none());
    }
}
