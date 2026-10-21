// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end test against a headless sway with two outputs:
//!
//! ```nushell
//! with-env { WLR_BACKENDS: headless, WLR_RENDERER: pixman, WLR_LIBINPUT_NO_DEVICES: 1 } { sway -c /dev/null }
//! swaymsg create_output ; swaymsg create_output
//! VNR_WLR_TEST_DISPLAY=wayland-1 cargo test -p vnr-wlr --test sway_headless -- --ignored
//! ```
//!
//! The test first applies a known layout, so it can run repeatedly against
//! the same compositor.

use vnr_core::{Backend, Layout, Position, Scale, Transform};
use vnr_wlr::WlrBackend;

const DISPLAY_ENV: &str = "VNR_WLR_TEST_DISPLAY";

fn connect() -> WlrBackend {
    let display = std::env::var(DISPLAY_ENV)
        .unwrap_or_else(|_| panic!("set {DISPLAY_ENV} to a wlr compositor's socket"));
    WlrBackend::connect_to(Some(&display)).expect("connect to the compositor")
}

/// Every output on, at its current or preferred mode, side by side, scale
/// 1, normal.
fn baseline(layout: &Layout) -> Layout {
    let mut layout = layout.clone();
    let mut x = 0;
    for output in &mut layout.outputs {
        output.enabled = true;
        output.mode = output.mode.or_else(|| output.preferred_mode());
        output.position = Position::new(x, 0);
        output.scale = Scale::ONE;
        output.transform = Transform::Normal;
        x += i32::try_from(output.mode.map_or(0, |m| m.width)).unwrap();
    }
    layout
}

fn edit(layout: &Layout, name: &str, change: impl FnOnce(&mut vnr_core::Output)) -> Layout {
    let mut layout = layout.clone();
    change(layout.outputs.iter_mut().find(|o| o.name == name).unwrap());
    layout
}

/// Tests, applies and reads back `layout`.
fn apply(backend: &mut WlrBackend, layout: &Layout) -> Layout {
    backend
        .test(layout)
        .expect("the compositor accepts the layout in a test");
    backend
        .apply(layout)
        .expect("the compositor applies the layout");
    let after = backend.snapshot().unwrap();
    for want in &layout.outputs {
        let got = after.get(&want.name).unwrap();
        assert_eq!(got.enabled, want.enabled, "{}", want.name);
        if want.enabled {
            assert_eq!(got.position, want.position, "{} position", want.name);
            assert_eq!(got.scale, want.scale, "{} scale", want.name);
            assert_eq!(got.transform, want.transform, "{} transform", want.name);
        }
    }
    after
}

#[test]
#[ignore = "needs a headless wlr compositor; see the module docs"]
fn wlr_output_management_round_trips() {
    let mut backend = connect();
    let start = backend.snapshot().unwrap();
    assert!(start.outputs.len() >= 2, "create two outputs first");
    let caps = backend.capabilities();
    assert!(caps.test_only && caps.fractional_scale && !caps.persistent);

    let base = apply(&mut backend, &baseline(&start));
    let (a, b) = (base.outputs[0].name.clone(), base.outputs[1].name.clone());
    let width =
        |l: &Layout, n: &str| i32::try_from(l.get(n).unwrap().logical_size().unwrap().0).unwrap();

    // Swap sides.
    let swapped = edit(&base, &b, |o| o.position = Position::new(0, 0));
    let swapped = edit(&swapped, &a, |o| {
        o.position = Position::new(width(&base, &b), 0);
    });
    apply(&mut backend, &swapped);

    // Fractional scale and rotation.
    let turned = edit(&base, &b, |o| {
        o.scale = Scale::from_millis(1500).unwrap();
        o.transform = Transform::Rotate90;
    });
    apply(&mut backend, &turned);

    // Off and on again.
    let off = edit(&base, &a, |o| o.enabled = false);
    let off = edit(&off, &b, |o| o.position = Position::new(0, 0));
    apply(&mut backend, &off);
    apply(&mut backend, &base);
    apply(&mut backend, &base);

    // Without COSMIC's extension, mirroring is refused before sending.
    if !caps.mirroring {
        let mirror = edit(&base, &b, |o| o.mirror_of = Some(a.clone()));
        assert!(backend.apply(&mirror).unwrap_err().is_unsupported());
    }
    let after = backend.snapshot().unwrap();
    for (want, got) in base.outputs.iter().zip(&after.outputs) {
        assert_eq!(
            (got.position, got.scale, got.transform),
            (want.position, want.scale, want.transform)
        );
    }
}
