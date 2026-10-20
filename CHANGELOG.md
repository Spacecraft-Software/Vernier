# Changelog

All notable changes to Vernier are recorded here. Dates are UTC, ISO 8601.

## Unreleased

### Added

- Core layout model, geometry, validation, TOML profiles, niri and sway export.
- wlr-output-management backend with the COSMIC extension (mirroring,
  `scale_1000`, three-state adaptive sync, Xwayland primary).
- `vnr` CLI conforming to the Spacecraft Software CLI Standard v1.1:
  noun-verb commands (`output list|get|set`, `profile save|apply`,
  `layout export`, `backend get`, `gui`, `schema`, `describe`); table output
  on a terminal and a `{metadata, data}` JSON envelope when piped or under an
  agent or CI; `jsonl`, `yaml`, `csv`, `--fields` and `--print0`; single-line
  JSON errors with runnable hints and documented exit codes 0–5 and 10–16;
  `--dry-run` for every change and file write; argument and path validation.
- `--fields` accepts any field a record declares, even when this answer
  omits it as null.
- `fixture` backend (`VNR_FIXTURE`) for testing without a display server.
- `vnr-theme` crate shared by the CLI and the editor.
- `SKILL.md` describing `vnr` for AI agents.
- iced editor with drag-and-snap, properties panel, problem list, Steelbore
  theme, and confirm-or-revert after apply.
- X11 backend over `RandR` 1.3+ (pure-Rust `x11rb` connection): outputs,
  modes, EDID identity, position, mode, rotation and reflection, mirroring,
  primary output, screen resizing, all applied with the server grabbed.
  Verified on real hardware (ThinkPad T490s, Intel modesetting driver).
- `vnr output get` shows make, model and serial, and lists modes under
  `available` so `mode` always means the current one.
- KDE Plasma backend over `kde_output_device_v2` and
  `kde_output_management_v2`: modes, position, rotation, fractional scale,
  VRR policy, and the primary output through output priorities. Plasma keeps
  applied changes across restarts. Tested against headless `KWin`.
  The Plasma protocol XML is vendored and compiled in (MIT-CMU), so current
  `KWin` versions, which send newer events, work; outputs announced through
  `kde_output_device_registry_v2` are read; mirroring works from Plasma 6.3.
- A missing `libwayland-client.so` is reported by name, with how to fix it.
- Backend detection tries the desktop's own protocol first, then wlr,
  then KWin's.
- CSV output always has the same columns, even when an optional value is
  absent from every row.
- Stub crate for the GNOME backend.
