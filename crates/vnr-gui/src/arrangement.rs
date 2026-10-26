// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canvas that shows outputs as rectangles and lets the user drag them.

use std::collections::BTreeMap;

use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Image, Path, Program, Stroke, Text};
use iced::widget::image;
use iced::widget::text::Alignment;
use iced::{Point, Rectangle, Renderer, Size, Theme, alignment, keyboard};
use vnr_core::geometry::snap;
use vnr_core::{Layout, Position, Rect};

use crate::preview::Thumbnail;
use crate::theme::Steelbore;

/// Gap between an output's outline and its screen contents, at least the
/// widest outline. The renderer draws pictures above lines, so the picture
/// must leave the outline (selection, overlap) uncovered.
const PICTURE_INSET: f32 = 3.0;

/// Offsets of the halo drawn behind text over screen contents, which keeps
/// it readable over any picture (text is drawn above pictures).
const HALO: [(f32, f32); 8] = [
    (-1.0, -1.0),
    (0.0, -1.0),
    (1.0, -1.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 1.0),
    (0.0, 1.0),
    (1.0, 1.0),
];

/// A picture of an output's screen, ready to draw.
#[derive(Debug, Clone)]
pub struct Shot {
    handle: image::Handle,
    width: u32,
    height: u32,
}

impl Shot {
    /// Uploads `thumbnail` as an image the renderer can draw.
    pub fn new(thumbnail: Thumbnail) -> Self {
        Self {
            width: thumbnail.width,
            height: thumbnail.height,
            handle: image::Handle::from_rgba(thumbnail.width, thumbnail.height, thumbnail.rgba),
        }
    }
}

/// The largest rectangle with the proportions `width` x `height` centred in
/// `area`. Pictures keep their shape when the edited layout gives an
/// output a different one than the screen has now.
#[expect(
    clippy::cast_precision_loss,
    reason = "thumbnail sizes are far below 2^23"
)]
fn fit(area: Rectangle, width: u32, height: u32) -> Rectangle {
    if width == 0 || height == 0 || area.width <= 0.0 || area.height <= 0.0 {
        return area;
    }
    let zoom = (area.width / width as f32).min(area.height / height as f32);
    let size = Size::new(width as f32 * zoom, height as f32 * zoom);
    Rectangle::new(
        Point::new(
            area.x + (area.width - size.width) / 2.0,
            area.y + (area.height - size.height) / 2.0,
        ),
        size,
    )
}

/// Distance in screen pixels within which a dragged edge snaps.
///
/// Converted to logical pixels at the current zoom, so snapping feels the
/// same however large the layout is.
const SNAP_SCREEN_PX: f32 = 12.0;

/// Fraction of the canvas left empty around the layout.
const MARGIN: f32 = 0.08;

/// Messages the canvas sends to the application.
#[derive(Debug, Clone)]
pub enum CanvasEvent {
    /// The user clicked an output, or empty space.
    Select(Option<usize>),
    /// An output was dragged to a new position.
    Move {
        /// Index into the layout's outputs.
        index: usize,
        /// The new top-left corner.
        position: Position,
    },
    /// A drag finished.
    DragEnded,
}

/// Maps logical coordinates to canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    zoom: f32,
    origin: Point,
    logical_origin: (i32, i32),
}

impl Viewport {
    /// Fits `layout` into a canvas of `size`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "layout sizes are far below 2^23 logical pixels"
    )]
    fn fit(layout: &Layout, size: Size) -> Self {
        let bounds = layout
            .bounds()
            .unwrap_or_else(|| Rect::new(0, 0, 1920, 1080));
        let content_w = bounds.width.max(1) as f32;
        let content_h = bounds.height.max(1) as f32;
        let usable_w = size.width * (1.0 - 2.0 * MARGIN);
        let usable_h = size.height * (1.0 - 2.0 * MARGIN);
        let zoom = (usable_w / content_w)
            .min(usable_h / content_h)
            .max(f32::EPSILON);
        let origin = Point::new(
            (size.width - content_w * zoom) / 2.0,
            (size.height - content_h * zoom) / 2.0,
        );
        Self {
            zoom,
            origin,
            logical_origin: (bounds.x, bounds.y),
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "screen coordinates need no more precision"
    )]
    fn to_screen(self, rect: Rect) -> Rectangle {
        Rectangle {
            x: self.origin.x + (rect.x - self.logical_origin.0) as f32 * self.zoom,
            y: self.origin.y + (rect.y - self.logical_origin.1) as f32 * self.zoom,
            width: rect.width as f32 * self.zoom,
            height: rect.height as f32 * self.zoom,
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounded screen coordinates fit in i32"
    )]
    fn to_logical(self, point: Point) -> (i32, i32) {
        let x = ((point.x - self.origin.x) / self.zoom).round() as i32;
        let y = ((point.y - self.origin.y) / self.zoom).round() as i32;
        (x + self.logical_origin.0, y + self.logical_origin.1)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small positive pixel distance"
    )]
    fn snap_threshold(self) -> u32 {
        (SNAP_SCREEN_PX / self.zoom).round().max(1.0) as u32
    }
}

/// An in-progress drag.
#[derive(Debug, Clone, Copy)]
struct Drag {
    index: usize,
    grab: (i32, i32),
    /// The view is frozen while dragging so the canvas does not rescale
    /// under the pointer.
    view: Viewport,
}

/// Interaction state kept by iced between events.
#[derive(Debug, Default)]
pub struct Interaction {
    drag: Option<Drag>,
    free: bool,
}

/// The arrangement canvas program.
#[derive(Debug)]
pub struct Arrangement<'a> {
    /// The layout being edited.
    pub layout: &'a Layout,
    /// Index of the selected output.
    pub selected: Option<usize>,
    /// Names of outputs involved in an overlap.
    pub overlapping: &'a [String],
    /// Screen contents by output name, when shown.
    pub shots: &'a BTreeMap<String, Shot>,
    /// Theme tokens.
    pub tokens: &'a Steelbore,
}

impl Arrangement<'_> {
    fn hit(&self, view: Viewport, point: Point) -> Option<usize> {
        let (x, y) = view.to_logical(point);
        let placed: Vec<_> = self.layout.placed_rects().collect();
        // The selected output is drawn last, so it is hit first.
        self.selected
            .and_then(|s| placed.iter().find(|(i, r)| *i == s && r.contains(x, y)))
            .or_else(|| placed.iter().rev().find(|(_, r)| r.contains(x, y)))
            .map(|(i, _)| *i)
    }
}

impl Program<CanvasEvent> for Arrangement<'_> {
    type State = Interaction;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<CanvasEvent>> {
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.free = modifiers.shift();
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = cursor.position_in(bounds)?;
                let view = Viewport::fit(self.layout, bounds.size());
                let Some(index) = self.hit(view, point) else {
                    return Some(canvas::Action::publish(CanvasEvent::Select(None)).and_capture());
                };
                let rect = self.layout.outputs[index].logical_rect()?;
                let (x, y) = view.to_logical(point);
                state.drag = Some(Drag {
                    index,
                    grab: (x - rect.x, y - rect.y),
                    view,
                });
                Some(canvas::Action::publish(CanvasEvent::Select(Some(index))).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let drag = state.drag?;
                let point = cursor.position_in(bounds).or_else(|| cursor.position())?;
                let rect = self.layout.outputs.get(drag.index)?.logical_rect()?;
                let (x, y) = drag.view.to_logical(point);
                let moving = Rect::new(x - drag.grab.0, y - drag.grab.1, rect.width, rect.height);
                let position = if state.free {
                    Position::new(moving.x, moving.y)
                } else {
                    let others: Vec<Rect> = self
                        .layout
                        .placed_rects()
                        .filter(|(i, _)| *i != drag.index)
                        .map(|(_, r)| r)
                        .collect();
                    snap(moving, &others, drag.view.snap_threshold())
                };
                if position == self.layout.outputs[drag.index].position {
                    return Some(canvas::Action::capture());
                }
                Some(
                    canvas::Action::publish(CanvasEvent::Move {
                        index: drag.index,
                        position,
                    })
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take()?;
                Some(canvas::Action::publish(CanvasEvent::DragEnded).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let tokens = self.tokens;
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), tokens.surface_alt);

        let view = state
            .drag
            .map_or_else(|| Viewport::fit(self.layout, bounds.size()), |d| d.view);

        let mut order: Vec<_> = self.layout.placed_rects().collect();
        // Draw the selected output last so it sits on top.
        order.sort_by_key(|(i, _)| Some(*i) == self.selected);

        for (index, rect) in order {
            let output = &self.layout.outputs[index];
            let area = view.to_screen(rect);
            let selected = Some(index) == self.selected;
            let overlapping = self.overlapping.contains(&output.name);

            frame.fill_rectangle(area.position(), area.size(), tokens.surface);
            let shot = self.shots.get(&output.name);
            if let Some(shot) = shot {
                let inner = Rectangle::new(
                    Point::new(area.x + PICTURE_INSET, area.y + PICTURE_INSET),
                    Size::new(
                        (area.width - 2.0 * PICTURE_INSET).max(0.0),
                        (area.height - 2.0 * PICTURE_INSET).max(0.0),
                    ),
                );
                frame.draw_image(
                    fit(inner, shot.width, shot.height),
                    Image::new(shot.handle.clone()),
                );
            }
            let (color, width) = match (selected, overlapping) {
                (true, _) => (tokens.focus, 3.0),
                (false, true) => (tokens.warning, 2.0),
                (false, false) => (tokens.border, 1.5),
            };
            frame.stroke(
                &Path::rectangle(area.position(), area.size()),
                Stroke::default().with_color(color).with_width(width),
            );

            let mut lines = vec![output.name.clone()];
            if let Some(mode) = output.mode {
                lines.push(format!("{}x{}", mode.width, mode.height));
            }
            if output.scale != vnr_core::Scale::ONE {
                lines.push(format!("scale {}", output.scale));
            }
            let mirrors: Vec<&str> = self
                .layout
                .outputs
                .iter()
                .filter(|o| o.enabled && o.mirror_of.as_deref() == Some(output.name.as_str()))
                .map(|o| o.name.as_str())
                .collect();
            if !mirrors.is_empty() {
                lines.push(format!("mirrored on {}", mirrors.join(", ")));
            }
            if overlapping {
                // State is never conveyed by color alone (§18).
                lines.push("overlaps".to_owned());
            }
            let size = (area.height / 7.0).clamp(10.0, 18.0);
            let text = Text {
                content: lines.join("\n"),
                position: area.center(),
                max_width: area.width - 8.0,
                color: tokens.foreground,
                size: size.into(),
                align_x: Alignment::Center,
                align_y: alignment::Vertical::Center,
                ..Text::default()
            };
            if shot.is_some() {
                for (dx, dy) in HALO {
                    frame.fill_text(Text {
                        position: Point::new(text.position.x + dx, text.position.y + dy),
                        color: tokens.surface,
                        ..text.clone()
                    });
                }
            }
            frame.fill_text(text);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        let view = Viewport::fit(self.layout, bounds.size());
        match cursor.position_in(bounds) {
            Some(point) if self.hit(view, point).is_some() => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use vnr_core::{Mode, Output};

    use super::*;

    fn layout() -> Layout {
        let mut a = Output::new("A");
        a.mode = Some(Mode::new(1920, 1080, 60_000));
        let mut b = Output::new("B");
        b.mode = Some(Mode::new(1920, 1080, 60_000));
        b.position = Position::new(1920, 0);
        Layout::new(vec![a, b])
    }

    #[test]
    fn viewport_round_trips_coordinates() {
        let view = Viewport::fit(&layout(), Size::new(800.0, 600.0));
        let screen = view.to_screen(Rect::new(1920, 0, 1920, 1080));
        let (x, y) = view.to_logical(Point::new(screen.x, screen.y));
        assert!((x - 1920).abs() <= 2 && y.abs() <= 2, "got {x},{y}");
    }

    #[test]
    fn pictures_keep_their_shape() {
        let area = Rectangle::new(Point::new(10.0, 20.0), Size::new(200.0, 100.0));
        let tall = fit(area, 90, 160);
        assert!((tall.height - 100.0).abs() < 0.01 && (tall.width - 56.25).abs() < 0.01);
        assert!((tall.center_x() - area.center_x()).abs() < 0.01);
        assert_eq!(fit(area, 0, 5), area);
    }

    #[test]
    fn layout_fits_inside_canvas() {
        let size = Size::new(800.0, 600.0);
        let view = Viewport::fit(&layout(), size);
        let all = view.to_screen(layout().bounds().unwrap());
        assert!(all.x >= 0.0 && all.y >= 0.0);
        assert!(all.x + all.width <= size.width + 0.5);
        assert!(all.y + all.height <= size.height + 0.5);
    }
}
