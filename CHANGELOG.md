# Changelog

All notable changes to Vernier are recorded here. Dates are UTC, ISO 8601.

## Unreleased

### Added

- CI (GitHub Actions): fmt, clippy and tests; a minimum-Rust (1.90) build;
  and each backend's end-to-end test against a real headless server (sway,
  the Xorg dummy driver, KWin, Mutter) via `scripts/test-servers.sh`.
- Supply-chain checks in CI: `cargo deny check` (licenses, sources, bans,
  advisories; policy in `deny.toml`) and `cargo audit`.
- `--persist FILE` on `output set` and `profile apply`: once the change is
  kept, write niri `output` blocks to a file niri's config includes. Refuses
  niri's `config.kdl` and any file Vernier did not generate, before anything
  on screen changes. The editor gains Save for niri on wlr sessions.
- Remappable editor key bindings from `~/.config/vernier/config.ncl`
  (Nickel, checked against a contract so errors name the field and line),
  and `vnr config check` to validate it. A broken file never stops the
  editor: it opens with the defaults and says why. New crate `vnr-config`.
- Screen labels: while the editor is open, each screen shows its output
  name through layer-shell, the selected one outlined; clicks pass through.
  `overlays = false` in `config.ncl` turns them off.
- Texinfo manual `doc/vernier.texi` (CC-BY-SA-4.0), built by
  `make info html pdf`; every package installs it as an Info file.
- Guix (`packaging/guix.scm`, crate list generated from `Cargo.lock` by
  `scripts/guix-crates.py`) and Arch (`packaging/PKGBUILD`) packages; all
  three build the checkout until the first release.
- Requirements moved from `PRD.md` to `AGENTS.md` as `VNR-SRS-NNN`, with
  rationale, source, method and status; needs listed in `README.md`.
- CI builds the manual and checks the Guix crate list and package scripts.
- Minimum Rust is now 1.90, which Nickel and its `malachite` dependency require.
- `layout export --to kanshi|shikane|home-manager`, and `--write` may target
  `~/.config/kanshi`, `~/.config/shikane` and `~/.config/home-manager`.

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
- The command line's keep-or-revert prompt counts down in place, as the
  editor's already did; it stays static in accessible mode and on dumb
  terminals, which now get the prompt instead of skipping it.
- Backend detection tries the desktop's own protocol first, then wlr,
  then KWin's.
- CSV output always has the same columns, even when an optional value is
  absent from every row.
- GNOME backend over `org.gnome.Mutter.DisplayConfig` (`zbus`): monitors,
  modes and supported scales, position, rotation, scale, primary output,
  mirroring (monitors sharing a logical monitor), and a real test-only apply
  (Mutter's verify method). Positions are translated in Mutter's physical
  layout mode, where a scaled monitor still covers its full pixel size.
  Changes are temporary, lasting until logout, so GNOME Shell's own
  confirmation dialog does not duplicate Vernier's. Tested against headless
  Mutter 46.
- Backend detection also falls back to KWin's and Mutter's protocols on
  desktops it does not recognize, and reports the first backend's reason
  when none answers.
