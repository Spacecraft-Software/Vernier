# Credits

- **wdisplays** (Michael Aquilina, artizirk and contributors) and **ARandR**
  (chrysn and contributors) — the programs Vernier replaces. No code was
  copied; their feature sets and known problems shaped the design.
- **COSMIC output management protocol** —
  `crates/vnr-wlr/protocols/cosmic-output-management-unstable-v1.xml`,
  Copyright (C) 2024 Victoria Brekenfeld, MIT license (`LICENSES/MIT.txt`),
  from [pop-os/cosmic-protocols](https://github.com/pop-os/cosmic-protocols).
  Vendored verbatim.
- **Plasma output protocols** — `crates/vnr-kde/protocols/kde-output-*.xml`,
  by Kristian Høgsberg, Intel, Collabora, Sebastian Kügler, Méven Car,
  Xaver Hugl and other KDE contributors (copyright lines in each file),
  MIT-CMU license (`LICENSES/MIT-CMU.txt`), from
  [KDE/plasma-wayland-protocols](https://invent.kde.org/libraries/plasma-wayland-protocols)
  at commit `1db0744`. Vendored verbatim.
- **cosmic-randr** (System76, MPL-2.0) — studied as a reference for the
  COSMIC extension; no code was copied.
- **kanshi** and **shikane** — prior art for profile matching by display
  identity.
- **wayland-rs**, **x11rb**, **zbus**, **iced**, **clap**, **serde** and **toml** — the Rust crates
  Vernier is built on, under their respective licenses.
