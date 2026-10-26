// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The shared libraries the editor loads at run time.
//!
//! The binary links only the C runtime (VNR-SRS-122); the windowing stack
//! opens the Wayland or X11 client libraries and xkbcommon when the window is
//! created. If one is missing, that happens deep inside the toolkit and
//! surfaces as an opaque failure. Probing the same libraries first, through
//! the same loaders, lets `vnr gui` name what is missing instead.
//!
//! Each probe opens its library once and keeps it open, as the toolkit
//! would anyway, so probing costs nothing extra on a working system.

// Rust guideline compliant 2026-05-18

use std::env;

/// The libraries the window needs for the current session that cannot be
/// loaded, by file name. Empty when everything is present, or when there is
/// no graphical session to open a window on (the toolkit reports that).
#[must_use]
pub fn missing_libraries() -> Vec<&'static str> {
    let session = Session::current();
    needed(session)
        .iter()
        .filter(|lib| !lib.loads())
        .map(|lib| lib.file_name())
        .collect()
}

/// Which window system the toolkit will use, judged as winit does: Wayland
/// when its socket is named, else X11 when a display is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    Wayland,
    X11,
    None,
}

impl Session {
    fn current() -> Self {
        let set = |name| env::var_os(name).is_some_and(|v| !v.is_empty());
        if set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET") {
            Self::Wayland
        } else if set("DISPLAY") {
            Self::X11
        } else {
            Self::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Library {
    WaylandClient,
    Xlib,
    Xcursor,
    XlibXcb,
    XInput2,
    XkbCommon,
}

impl Library {
    const fn file_name(self) -> &'static str {
        match self {
            Self::WaylandClient => "libwayland-client.so.0",
            Self::Xlib => "libX11.so.6",
            Self::Xcursor => "libXcursor.so.1",
            Self::XlibXcb => "libX11-xcb.so.1",
            Self::XInput2 => "libXi.so.6",
            Self::XkbCommon => "libxkbcommon.so.0",
        }
    }

    fn loads(self) -> bool {
        match self {
            Self::WaylandClient => wayland_sys::client::is_lib_available(),
            Self::Xlib => x11_dl::xlib::Xlib::open().is_ok(),
            Self::Xcursor => x11_dl::xcursor::Xcursor::open().is_ok(),
            Self::XlibXcb => x11_dl::xlib_xcb::Xlib_xcb::open().is_ok(),
            Self::XInput2 => x11_dl::xinput2::XInput2::open().is_ok(),
            Self::XkbCommon => xkbcommon_dl::xkbcommon_option().is_some(),
        }
    }
}

const fn needed(session: Session) -> &'static [Library] {
    match session {
        Session::Wayland => &[Library::WaylandClient, Library::XkbCommon],
        // winit's `XConnection::new` opens these four before anything else.
        Session::X11 => &[
            Library::Xlib,
            Library::Xcursor,
            Library::XlibXcb,
            Library::XInput2,
            Library::XkbCommon,
        ],
        Session::None => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_session_needs_keyboard_handling_and_its_client_library() {
        assert_eq!(
            needed(Session::Wayland),
            &[Library::WaylandClient, Library::XkbCommon]
        );
        assert_eq!(
            needed(Session::X11).first(),
            Some(&Library::Xlib),
            "the client library is named first"
        );
        assert_eq!(needed(Session::X11).last(), Some(&Library::XkbCommon));
        assert_eq!(needed(Session::X11).len(), 5);
        assert_eq!(needed(Session::None), &[] as &[Library]);
    }

    #[test]
    fn file_names_are_the_sonames_the_loaders_open() {
        for lib in needed(Session::Wayland).iter().chain(needed(Session::X11)) {
            assert!(lib.file_name().starts_with("lib"));
            assert!(lib.file_name().contains(".so."));
        }
    }
}
