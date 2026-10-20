// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end test against a headless Mutter with two virtual monitors, on
//! a private session bus:
//!
//! ```nushell
//! let bus = (dbus-daemon --session --fork --print-address=1 | lines | first)
//! with-env { DBUS_SESSION_BUS_ADDRESS: $bus } { mutter --headless --wayland --no-x11 --wayland-display mutter-test --virtual-monitor 1920x1080 --virtual-monitor 1920x1080 }
//! VNR_MUTTER_TEST_BUS=$bus cargo test -p vnr-mutter --test mutter_headless -- --ignored
//! ```
//!
//! The test first applies a known layout, so it can run repeatedly against
//! the same Mutter.

use vnr_core::{Backend, Layout, Mode, Position, Scale, Transform};
use vnr_mutter::MutterBackend;

const BUS_ENV: &str = "VNR_MUTTER_TEST_BUS";

fn connect() -> MutterBackend {
    let bus = std::env::var(BUS_ENV)
        .unwrap_or_else(|_| panic!("set {BUS_ENV} to the bus address Mutter runs on"));
    MutterBackend::connect_to(Some(&bus)).expect("connect to Mutter")
}

/// Every monitor on at its preferred mode, side by side, scale 1, normal,
/// the first one primary.
fn baseline(layout: &Layout) -> Layout {
    let mut layout = layout.clone();
    let mut x = 0;
    for (i, output) in layout.outputs.iter_mut().enumerate() {
        output.enabled = true;
        output.mode = output.preferred_mode();
        output.position = Position::new(x, 0);
        output.scale = Scale::ONE;
        output.transform = Transform::Normal;
        output.mirror_of = None;
        output.primary = i == 0;
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
fn apply(backend: &mut MutterBackend, layout: &Layout) -> Layout {
    backend.test(layout).expect("Mutter verifies the layout");
    backend.apply(layout).expect("Mutter applies the layout");
    let after = backend.snapshot().unwrap();
    for want in &layout.outputs {
        let got = after.get(&want.name).unwrap();
        assert_eq!(got.enabled, want.enabled, "{}", want.name);
        if want.enabled {
            assert_eq!(got.mirror_of, want.mirror_of, "{} mirror", want.name);
            if want.mirror_of.is_none() {
                assert_eq!(got.position, want.position, "{} position", want.name);
                assert_eq!(got.primary, want.primary, "{} primary", want.name);
            }
            assert_eq!(got.scale, want.scale, "{} scale", want.name);
            assert_eq!(got.transform, want.transform, "{} transform", want.name);
        }
    }
    after
}

#[test]
#[ignore = "needs a headless Mutter; see the module docs"]
fn display_config_round_trips() {
    let mut backend = connect();
    let start = backend.snapshot().unwrap();
    assert!(
        start.outputs.len() >= 2,
        "start Mutter with two --virtual-monitor"
    );
    let caps = backend.capabilities();
    assert!(caps.test_only && caps.primary && caps.mirroring && !caps.persistent);

    let base = apply(&mut backend, &baseline(&start));
    let (a, b) = (base.outputs[0].name.clone(), base.outputs[1].name.clone());

    // Swap sides.
    let swapped = edit(&base, &b, |o| o.position = Position::new(0, 0));
    let swapped = edit(&swapped, &a, |o| o.position = Position::new(1920, 0));
    apply(&mut backend, &swapped);

    // Integer scale: in physical layout mode Mutter places the neighbour at
    // 1920 while Vernier sees it at 960, and both must agree after a read.
    let scaled = edit(&base, &a, |o| o.scale = Scale::from_millis(2000).unwrap());
    let scaled = edit(&scaled, &b, |o| o.position = Position::new(960, 0));
    apply(&mut backend, &scaled);

    // Rotation.
    let turned = edit(&base, &b, |o| o.transform = Transform::Rotate90);
    apply(&mut backend, &turned);

    // Mirror, then back.
    let mirrored = edit(&base, &b, |o| o.mirror_of = Some(a.clone()));
    apply(&mut backend, &mirrored);
    apply(&mut backend, &base);

    // Primary moves; a monitor turns off and on again.
    let primary = edit(&edit(&base, &a, |o| o.primary = false), &b, |o| {
        o.primary = true;
    });
    apply(&mut backend, &primary);
    let off = edit(&edit(&base, &a, |o| o.enabled = false), &b, |o| {
        o.position = Position::new(0, 0);
        o.primary = true;
    });
    apply(&mut backend, &off);
    apply(&mut backend, &base);
    apply(&mut backend, &base);

    // Mutter's own checks reach the caller as the right error kinds.
    let apart = edit(&base, &b, |o| o.position = Position::new(4000, 0));
    assert!(
        backend.test(&apart).unwrap_err().is_rejected(),
        "monitors must touch"
    );
    let odd = edit(&base, &a, |o| o.mode = Some(Mode::new(1000, 700, 60_000)));
    assert!(backend.apply(&odd).unwrap_err().is_unsupported());
    if !caps.fractional_scale {
        let fractional = edit(&base, &a, |o| o.scale = Scale::from_millis(1500).unwrap());
        assert!(backend.apply(&fractional).unwrap_err().is_unsupported());
    }
    assert_eq!(backend.snapshot().unwrap(), base);
}
