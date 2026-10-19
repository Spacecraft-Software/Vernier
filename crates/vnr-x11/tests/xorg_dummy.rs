// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end tests against a real X server running the `dummy` video
//! driver, which offers sixteen virtual outputs (`DUMMY0` ... `DUMMY15`)
//! with genuine CRTCs. Run with:
//!
//! ```nushell
//! Xorg :5 -noreset -nolisten tcp -config crates/vnr-x11/tests/dummy.conf
//! VNR_X11_TEST_DISPLAY=:5 cargo test -p vnr-x11 --test xorg_dummy -- --ignored
//! ```
//!
//! The test resets the server to a known two-output layout first, so it can
//! be re-run against the same server.

use vnr_core::{Backend, Layout, Mode, Position, Scale, Transform};
use vnr_x11::X11Backend;
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{ConnectionExt as _, Rotation};

const DISPLAY_ENV: &str = "VNR_X11_TEST_DISPLAY";

fn display() -> String {
    std::env::var(DISPLAY_ENV)
        .unwrap_or_else(|_| panic!("set {DISPLAY_ENV} to a dummy-driver X server"))
}

/// Turns on `DUMMY0` and `DUMMY1` side by side at 1920x1080, both offering
/// 1920x1080 and 1280x720, with `DUMMY0` primary and every other CRTC off.
fn reset(display: &str) {
    let (conn, screen) = x11rb::connect(Some(display)).expect("connect");
    let root = conn.setup().roots[screen].root;
    let res = conn
        .randr_get_screen_resources(root)
        .unwrap()
        .reply()
        .unwrap();
    let output = |name: &str| {
        res.outputs
            .iter()
            .copied()
            .find(|o| {
                conn.randr_get_output_info(*o, res.config_timestamp)
                    .unwrap()
                    .reply()
                    .unwrap()
                    .name
                    == name.as_bytes()
            })
            .unwrap_or_else(|| panic!("no output {name}; is this the dummy driver?"))
    };
    let (zero, one) = (output("DUMMY0"), output("DUMMY1"));
    // Each output must list 1920x1080 and 1280x720; the driver may offer
    // several modes of one size, so prefer one the output already has.
    let ensure = |o: u32, w: u16, h: u16| -> u32 {
        let info = conn
            .randr_get_output_info(o, res.config_timestamp)
            .unwrap()
            .reply()
            .unwrap();
        let sized = |m: &&x11rb::protocol::randr::ModeInfo| m.width == w && m.height == h;
        if let Some(m) = res
            .modes
            .iter()
            .filter(sized)
            .find(|m| info.modes.contains(&m.id))
        {
            return m.id;
        }
        let m = res
            .modes
            .iter()
            .find(sized)
            .unwrap_or_else(|| panic!("the server config must define a {w}x{h} modeline"));
        conn.randr_add_output_mode(o, m.id)
            .unwrap()
            .check()
            .unwrap();
        m.id
    };
    let fhd = [ensure(zero, 1920, 1080), ensure(one, 1920, 1080)];
    ensure(zero, 1280, 720);
    ensure(one, 1280, 720);
    let ts = conn
        .randr_get_screen_resources(root)
        .unwrap()
        .reply()
        .unwrap()
        .config_timestamp;
    let normal = Rotation::ROTATE0;
    for &crtc in &res.crtcs {
        let reply = conn
            .randr_set_crtc_config(crtc, 0, ts, 0, 0, 0, normal, &[])
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(reply.status, x11rb::protocol::randr::SetConfig::SUCCESS);
    }
    conn.randr_set_screen_size(root, 3840, 1080, 1016, 286)
        .unwrap()
        .check()
        .unwrap();
    // The driver decides which CRTCs may drive which output; ask it.
    let possible = |o: u32| {
        conn.randr_get_output_info(o, ts)
            .unwrap()
            .reply()
            .unwrap()
            .crtcs
    };
    let first = possible(zero)[0];
    let second = possible(one)
        .into_iter()
        .find(|c| *c != first)
        .expect("a second CRTC");
    for (crtc, x, o, mode) in [(first, 0, zero, fhd[0]), (second, 1920, one, fhd[1])] {
        let reply = conn
            .randr_set_crtc_config(crtc, 0, ts, x, 0, mode, normal, &[o])
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(reply.status, x11rb::protocol::randr::SetConfig::SUCCESS);
    }
    conn.randr_set_output_primary(root, zero)
        .unwrap()
        .check()
        .unwrap();
    conn.flush().unwrap();
}

fn connect(display: &str) -> X11Backend {
    X11Backend::connect_to(Some(display)).expect("connect to the test server")
}

fn apply(backend: &mut X11Backend, edit: impl FnOnce(&mut Layout)) -> Layout {
    let mut layout = backend.snapshot().unwrap();
    edit(&mut layout);
    vnr_core::geometry::normalize(&mut layout);
    backend.apply(&layout).unwrap();
    let after = backend.snapshot().unwrap();
    for wanted in &layout.outputs {
        let got = after.get(&wanted.name).unwrap();
        assert_eq!(got.enabled, wanted.enabled, "{}", wanted.name);
        if wanted.enabled {
            // A mirror's own position is ignored; it sits on its source.
            if wanted.mirror_of.is_none() {
                assert_eq!(got.position, wanted.position, "{}", wanted.name);
            }
            assert_eq!(got.transform, wanted.transform, "{}", wanted.name);
            assert_eq!(got.primary, wanted.primary, "{}", wanted.name);
            assert_eq!(got.mirror_of, wanted.mirror_of, "{}", wanted.name);
        }
    }
    after
}

#[test]
#[ignore = "needs an Xorg dummy-driver server; see the module docs"]
fn randr_round_trips_on_the_dummy_driver() {
    let display = display();
    reset(&display);
    let mut backend = connect(&display);

    // Snapshot.
    let layout = backend.snapshot().unwrap();
    let on: Vec<&str> = layout
        .outputs
        .iter()
        .filter(|o| o.enabled)
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(on, ["DUMMY0", "DUMMY1"]);
    let one = layout.get("DUMMY1").unwrap();
    assert_eq!(one.position, Position::new(1920, 0));
    assert_eq!(
        (one.mode.unwrap().width, one.mode.unwrap().height),
        (1920, 1080)
    );
    assert!(layout.get("DUMMY0").unwrap().primary);
    let caps = backend.capabilities();
    assert!(caps.mirroring && caps.primary && !caps.test_only);
    assert!(
        !caps.fractional_scale,
        "the dummy driver has no CRTC transforms"
    );

    // Applying the same layout changes nothing.
    backend.apply(&layout).unwrap();
    assert_eq!(backend.snapshot().unwrap(), layout);

    // Move left of DUMMY0.
    apply(&mut backend, |l| {
        l.outputs
            .iter_mut()
            .find(|o| o.name == "DUMMY1")
            .unwrap()
            .position = Position::new(-1920, 0);
    });

    // A smaller mode shrinks the screen.
    let after = apply(&mut backend, |l| {
        l.outputs
            .iter_mut()
            .find(|o| o.name == "DUMMY1")
            .unwrap()
            .mode = Some(Mode::new(1280, 720, 60_000));
    });
    assert_eq!(
        after.get("DUMMY1").unwrap().logical_size(),
        Some((1280, 720))
    );

    // Mirror, then stop mirroring.
    apply(&mut backend, |l| {
        let o = l.outputs.iter_mut().find(|o| o.name == "DUMMY1").unwrap();
        o.mode = Some(Mode::new(1920, 1080, 60_000));
        o.mirror_of = Some("DUMMY0".into());
    });
    apply(&mut backend, |l| {
        let o = l.outputs.iter_mut().find(|o| o.name == "DUMMY1").unwrap();
        o.mirror_of = None;
        o.position = Position::new(1920, 0);
    });

    // Primary moves; an output turns off and back on.
    apply(&mut backend, |l| {
        for o in &mut l.outputs {
            o.primary = o.name == "DUMMY1";
        }
    });
    apply(&mut backend, |l| {
        l.outputs
            .iter_mut()
            .find(|o| o.name == "DUMMY0")
            .unwrap()
            .enabled = false;
    });
    apply(&mut backend, |l| {
        let o = l.outputs.iter_mut().find(|o| o.name == "DUMMY0").unwrap();
        o.enabled = true;
        o.mode = Some(Mode::new(1920, 1080, 60_000));
        o.position = Position::new(1920, 0);
    });

    refusals(&mut backend);
}

/// What the dummy driver cannot do is refused before anything is sent.
fn refusals(backend: &mut X11Backend) {
    let before = backend.snapshot().unwrap();
    let mut scaled = before.clone();
    scaled
        .outputs
        .iter_mut()
        .find(|o| o.name == "DUMMY0")
        .unwrap()
        .scale = Scale::from_millis(1500).unwrap();
    assert!(backend.apply(&scaled).unwrap_err().is_unsupported());
    let mut rotated = before.clone();
    rotated
        .outputs
        .iter_mut()
        .find(|o| o.name == "DUMMY0")
        .unwrap()
        .transform = Transform::Rotate90;
    assert!(
        backend.apply(&rotated).unwrap_err().is_unsupported(),
        "the dummy driver only has rotate-0"
    );
    let mut odd = before.clone();
    odd.outputs
        .iter_mut()
        .find(|o| o.name == "DUMMY0")
        .unwrap()
        .mode = Some(Mode::new(1000, 700, 60_000));
    assert!(backend.apply(&odd).unwrap_err().is_unsupported());
    assert!(backend.test(&before).unwrap_err().is_unsupported());
    assert_eq!(backend.snapshot().unwrap(), before);
}
