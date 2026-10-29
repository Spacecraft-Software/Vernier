// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Logical-space geometry: rectangles, edge snapping and normalization.

use serde::{Deserialize, Serialize};

use crate::model::{Layout, Output, Position};

/// How close, in logical pixels, an edge must come to another output's edge
/// before a drag snaps to it.
///
/// Large enough to catch a mouse drag on a zoomed-out canvas, small enough
/// that two outputs can still be placed with a deliberate gap. The GUI
/// scales this by its zoom so the feel stays constant on screen.
pub const DEFAULT_SNAP_THRESHOLD: u32 = 48;

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Converts a size to a coordinate, saturating at `i32::MAX`.
///
/// Display sizes are far below `i32::MAX`; saturation only guards against
/// corrupt backend data.
pub(crate) fn to_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl Rect {
    /// Creates a rectangle.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The exclusive right edge.
    #[must_use]
    pub fn right(&self) -> i32 {
        self.x.saturating_add(to_i32(self.width))
    }

    /// The exclusive bottom edge.
    #[must_use]
    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(to_i32(self.height))
    }

    /// Whether the two rectangles share a positive area.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether the rectangles overlap or share an edge segment of positive
    /// length, so the pointer can move from one to the other.
    #[must_use]
    pub fn touches(&self, other: &Self) -> bool {
        let x_overlap = self.x < other.right() && other.x < self.right();
        let y_overlap = self.y < other.bottom() && other.y < self.bottom();
        let x_adjacent = self.right() == other.x || other.right() == self.x;
        let y_adjacent = self.bottom() == other.y || other.bottom() == self.y;
        (y_overlap && (x_overlap || x_adjacent)) || (x_overlap && y_adjacent)
    }

    /// The smallest rectangle containing both.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right.abs_diff(x), bottom.abs_diff(y))
    }

    /// Whether the point lies inside the rectangle.
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Snaps a rectangle being dragged to the edges of `others`.
///
/// Each axis snaps independently: the left or right edge of `moving` snaps
/// to the left or right edge of any other rectangle within `threshold`, and
/// likewise for top and bottom. The nearest candidate wins. Returns the
/// adjusted top-left corner.
#[must_use]
pub fn snap(moving: Rect, others: &[Rect], threshold: u32) -> Position {
    let width = to_i32(moving.width);
    let height = to_i32(moving.height);
    let x = nearest(
        moving.x,
        others
            .iter()
            .flat_map(|o| [o.x, o.right(), o.x - width, o.right() - width]),
        threshold,
    );
    let y = nearest(
        moving.y,
        others
            .iter()
            .flat_map(|o| [o.y, o.bottom(), o.y - height, o.bottom() - height]),
        threshold,
    );
    Position::new(x, y)
}

fn nearest(value: i32, candidates: impl Iterator<Item = i32>, threshold: u32) -> i32 {
    candidates
        .filter(|c| c.abs_diff(value) <= threshold)
        .min_by_key(|c| c.abs_diff(value))
        .unwrap_or(value)
}

/// Translates every output so the placed ones start at `(0, 0)`.
///
/// Compositors accept negative coordinates, but a layout anchored at the
/// origin is what users expect to read in a profile or config file.
/// Mirrored and disabled outputs move too, so relative intent is kept.
pub fn normalize(layout: &mut Layout) {
    let Some(bounds) = layout.bounds() else {
        return;
    };
    if bounds.x == 0 && bounds.y == 0 {
        return;
    }
    for output in &mut layout.outputs {
        output.position.x = output.position.x.saturating_sub(bounds.x);
        output.position.y = output.position.y.saturating_sub(bounds.y);
    }
}

/// Keeps neighbours attached after `layout.outputs[index]` changed size.
///
/// `old` is the output's rectangle before the change. An output that sat to
/// the right of it and shared rows with it moves by the change in width; one
/// that sat below it and shared columns with it moves by the change in
/// height. Edges that touched still touch, and outputs that never touched it
/// stay where they are, so a valid arrangement cannot gain an overlap.
/// Mutter, for one, rejects a layout with a gap.
pub fn reflow(layout: &mut Layout, index: usize, old: Rect) {
    let Some(new) = layout.outputs.get(index).and_then(Output::logical_rect) else {
        return;
    };
    let dx = to_i32(new.width).saturating_sub(to_i32(old.width));
    let dy = to_i32(new.height).saturating_sub(to_i32(old.height));
    let shares = |lo: i32, hi: i32, olo: i32, ohi: i32| lo < ohi && olo < hi;
    for (i, other) in layout.outputs.iter_mut().enumerate() {
        if i == index || !other.enabled || other.mirror_of.is_some() {
            continue;
        }
        let Some(rect) = other.logical_rect() else {
            continue;
        };
        if rect.x >= old.right() && shares(rect.y, rect.bottom(), old.y, old.bottom()) {
            other.position.x = other.position.x.saturating_add(dx);
        }
        if rect.y >= old.bottom() && shares(rect.x, rect.right(), old.x, old.right()) {
            other.position.y = other.position.y.saturating_add(dy);
        }
    }
}

/// Pushes a placed output out of any overlap by moving it right of the
/// rectangle it overlaps, repeating until it is clear.
///
/// Used after a mode, scale or transform change grows an output into its
/// neighbor, which a drag would otherwise have prevented.
pub fn resolve_overlap(layout: &mut Layout, index: usize) {
    // Every iteration moves the output strictly right of one obstacle, so
    // it can collide with each other output at most once.
    for _ in 0..layout.outputs.len() {
        let Some(rect) = layout.outputs.get(index).and_then(Output::logical_rect) else {
            return;
        };
        let obstacle = layout
            .placed_rects()
            .filter(|(i, _)| *i != index)
            .map(|(_, r)| r)
            .find(|r| r.intersects(&rect));
        match obstacle {
            Some(obstacle) => layout.outputs[index].position.x = obstacle.right(),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Scale};

    fn output(name: &str, x: i32, y: i32, width: u32, height: u32) -> Output {
        let mut output = Output::new(name);
        output.mode = Some(Mode::new(width, height, 60_000));
        output.position = Position::new(x, y);
        output
    }

    #[test]
    fn adjacent_rects_touch_but_do_not_intersect() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(100, 50, 100, 100);
        assert!(a.touches(&b));
        assert!(!a.intersects(&b));
        let corner = Rect::new(100, 100, 10, 10);
        assert!(!a.touches(&corner), "corner contact leaves no shared edge");
        let far = Rect::new(150, 0, 10, 10);
        assert!(!a.touches(&far));
    }

    #[test]
    fn snap_pulls_edges_together() {
        let fixed = [Rect::new(0, 0, 1920, 1080)];
        let moving = Rect::new(1930, 20, 1280, 1024);
        assert_eq!(snap(moving, &fixed, 48), Position::new(1920, 0));
        let far = Rect::new(2100, 300, 1280, 1024);
        assert_eq!(snap(far, &fixed, 48), Position::new(2100, 300));
    }

    #[test]
    fn snap_aligns_bottom_edges() {
        let fixed = [Rect::new(0, 0, 1920, 1080)];
        let moving = Rect::new(1920, 70, 1280, 1024);
        assert_eq!(snap(moving, &fixed, 48).y, 56);
    }

    #[test]
    fn normalize_moves_layout_to_origin() {
        let mut layout = Layout::new(vec![
            output("A", -1920, 100, 1920, 1080),
            output("B", 0, 100, 1920, 1080),
        ]);
        normalize(&mut layout);
        assert_eq!(layout.outputs[0].position, Position::new(0, 0));
        assert_eq!(layout.outputs[1].position, Position::new(1920, 0));
    }

    #[test]
    fn reflow_keeps_the_right_neighbour_touching() {
        let mut layout = Layout::new(vec![
            output("A", 0, 0, 1920, 1080),
            output("B", 1920, 0, 1920, 1080),
        ]);
        let old = layout.outputs[0].logical_rect().unwrap();
        layout.outputs[0].scale = Scale::from_f64(1.25).unwrap();
        reflow(&mut layout, 0, old);
        assert_eq!(layout.outputs[1].position, Position::new(1536, 0));
    }

    #[test]
    fn reflow_does_not_overlap_a_corner_neighbour() {
        let mut layout = Layout::new(vec![
            output("A", 0, 0, 1920, 1080),
            output("B", 1920, 0, 1920, 1080),
            output("C", 1920, 1080, 1920, 1080),
        ]);
        let old = layout.outputs[0].logical_rect().unwrap();
        layout.outputs[0].scale = Scale::from_f64(1.25).unwrap();
        reflow(&mut layout, 0, old);
        assert_eq!(layout.outputs[1].position, Position::new(1536, 0));
        assert_eq!(layout.outputs[2].position, Position::new(1920, 1080));
    }

    #[test]
    fn resolve_overlap_pushes_right() {
        let mut layout = Layout::new(vec![
            output("A", 0, 0, 1920, 1080),
            output("B", 1000, 0, 1920, 1080),
            output("C", 2920, 0, 100, 100),
        ]);
        resolve_overlap(&mut layout, 1);
        assert_eq!(layout.outputs[1].position.x, 3020);
        let rects: Vec<_> = layout.placed_rects().collect();
        for (index, rect) in &rects {
            for (other_index, other) in &rects {
                assert!(index == other_index || !rect.intersects(other));
            }
        }
    }
}
