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
- `fixture` backend (`VNR_FIXTURE`) for testing without a display server.
- `vnr-theme` crate shared by the CLI and the editor.
- `SKILL.md` describing `vnr` for AI agents.
- iced editor with drag-and-snap, properties panel, problem list, Steelbore
  theme, and confirm-or-revert after apply.
- Stub crates for the X11, KDE and GNOME backends.
