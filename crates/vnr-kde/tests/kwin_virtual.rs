// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end test against a headless `KWin` with two virtual outputs:
//!
//! ```nushell
//! with-env { QT_QPA_PLATFORM: offscreen } { kwin_wayland --virtual --width 1920 --height 1080 --output-count 2 --socket kwin-test }
//! VNR_KDE_TEST_DISPLAY=kwin-test cargo test -p vnr-kde --test kwin_virtual -- --ignored
//! ```
//!
//! The test first applies a known layout, so it can run repeatedly against
//! the same compositor.

use vnr_core::{AdaptiveSync, Backend, Layout, Mode, Position, Scale, Transform};
use vnr_kde::KdeBackend;

const DISPLAY_ENV: &str = "VNR_KDE_TEST_DISPLAY";

fn connect() -> KdeBackend {
    let display =
        std::env::var(DISPLAY_ENV).unwrap_or_else(|_| panic!("set {DISPLAY_ENV} to a KWin socket"));
    KdeBackend::connect_to(Some(&display)).expect("connect to KWin")
}

/// Every output on, at its preferred mode, side by side, scale 1, normal,
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
        output.primary = i == 0;
        x += i32::try_from(output.mode.map_or(0, |m| m.width)).unwrap();
    }
    layout
}

fn apply(backend: &mut KdeBackend, layout: &Layout) -> Layout {
    backend.apply(layout).expect("KWin applies the layout");
    let after = backend.snapshot().unwrap();
    for want in &layout.outputs {
        let got = after.get(&want.name).unwrap();
        assert_eq!(got.enabled, want.enabled, "{}", want.name);
        if want.enabled {
            assert_eq!(got.position, want.position, "{} position", want.name);
            assert_eq!(got.scale, want.scale, "{} scale", want.name);
            assert_eq!(got.transform, want.transform, "{} transform", want.name);
            assert_eq!(got.primary, want.primary, "{} primary", want.name);
        }
    }
    after
}

#[test]
#[ignore = "needs a headless KWin; see the module docs"]
fn kde_output_management_round_trips() {
    let mut backend = connect();
    let start = backend.snapshot().unwrap();
    assert!(start.outputs.len() >= 2, "start KWin with --output-count 2");
    let caps = backend.capabilities();
    assert!(caps.persistent && caps.fractional_scale && caps.primary);
    assert!(!caps.mirroring && !caps.test_only && !caps.custom_modes);

    let base = apply(&mut backend, &baseline(&start));
    let (a, b) = (base.outputs[0].name.clone(), base.outputs[1].name.clone());
    let width =
        |l: &Layout, n: &str| i32::try_from(l.get(n).unwrap().logical_size().unwrap().0).unwrap();

    // Swap sides.
    let mut swapped = base.clone();
    swapped
        .outputs
        .iter_mut()
        .find(|o| o.name == b)
        .unwrap()
        .position = Position::new(0, 0);
    swapped
        .outputs
        .iter_mut()
        .find(|o| o.name == a)
        .unwrap()
        .position = Position::new(width(&base, &b), 0);
    apply(&mut backend, &swapped);

    // Fractional scale and rotation.
    let mut turned = base.clone();
    let o = turned.outputs.iter_mut().find(|o| o.name == b).unwrap();
    o.scale = Scale::from_millis(1500).unwrap();
    o.transform = Transform::Rotate90;
    let after = apply(&mut backend, &turned);
    assert_eq!(after.get(&b).unwrap().logical_size(), Some((720, 1280)));

    // Primary moves; an output turns off and on again.
    let mut primary = base.clone();
    for o in &mut primary.outputs {
        o.primary = o.name == b;
    }
    apply(&mut backend, &primary);
    let mut off = base.clone();
    off.outputs
        .iter_mut()
        .find(|o| o.name == a)
        .unwrap()
        .enabled = false;
    off.outputs
        .iter_mut()
        .find(|o| o.name == b)
        .unwrap()
        .primary = true;
    apply(&mut backend, &off);
    apply(&mut backend, &base);

    // Applying the same layout twice is harmless.
    apply(&mut backend, &base);

    // What KDE output management cannot express is refused up front.
    let mut mirror = base.clone();
    mirror
        .outputs
        .iter_mut()
        .find(|o| o.name == b)
        .unwrap()
        .mirror_of = Some(a.clone());
    assert!(backend.apply(&mirror).unwrap_err().is_unsupported());
    let mut odd = base.clone();
    odd.outputs[0].mode = Some(Mode::new(1000, 700, 60_000));
    assert!(backend.apply(&odd).unwrap_err().is_unsupported());
    assert!(backend.test(&base).unwrap_err().is_unsupported());
    if !caps.adaptive_sync {
        // Virtual outputs have no VRR; the policy is simply not sent.
        let mut vrr = base.clone();
        vrr.outputs[0].adaptive_sync = Some(AdaptiveSync::Always);
        backend.apply(&vrr).unwrap();
        assert_eq!(backend.snapshot().unwrap().outputs[0].adaptive_sync, None);
    }
    assert_eq!(backend.snapshot().unwrap(), base);
}
