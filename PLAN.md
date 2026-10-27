# Vernier — Plan

MVP: M0–M7 — Vernier v1.0

## M0 — Core model and workspace

- [x] P-001 Output, mode, transform, scale and mirroring model
- [x] P-002 Geometry: logical rects, edge snapping, normalization, overlap push
- [x] P-003 Validation with errors and warnings
- [x] P-004 TOML profiles matched by make, model and serial
- [x] P-005 niri and sway exporters
- [x] P-006 Backend trait and run-time detection

## M1 — wlr and COSMIC backend, CLI, editor

- [x] P-007 wlr-output-management v4: snapshot, test, apply
- [x] P-008 COSMIC extension: mirroring, scale_1000, adaptive sync, Xwayland primary
- [x] P-009 CLI: list, backend, save, apply, set, export, JSON output
- [x] P-010 Confirm-or-revert on CLI and in the editor
- [x] P-011 iced editor: drag with snapping, properties panel, problems list
- [x] P-012 Steelbore theme with §11.6 resolution and §18.1 accessible toggle
- [x] P-013 End-to-end test against headless sway (`crates/vnr-wlr/tests/sway_headless.rs`, in CI)
- [x] P-014 Verify on niri (real hardware): Steelbore OS Bravais, ThinkPad T490s eDP-1, 2026-10-26; rotation with confirm, Save for niri, screen contents through `wlr-screencopy`
- [ ] P-015 Verify on COSMIC (real hardware), including mirroring
- [ ] P-016 Live change tracking (re-read state on compositor `done` events)
- [ ] P-036 CLI Standard v1.1 conformance (see TODO.md)

## M2 — X11 backend

- [x] P-017 `RandR` 1.3+ snapshot via `x11rb`: outputs, CRTCs, modes, EDID identity
- [x] P-018 Apply: CRTC config, screen resize, rotation, mirroring, primary
- [x] P-037 Scale via CRTC transform: verified on a ThinkPad T490s (Intel, modesetting), 2026-10-20
- [ ] P-038 Custom modes (`RRCreateMode` from CVT timings)
- [ ] P-019 `ScreenChangeNotify` live tracking
- [x] P-020 Run the Xorg dummy-driver integration test (`crates/vnr-x11/tests/xorg_dummy.rs`) in CI
- [x] P-039 Verify on real X11 hardware: scale, rotation, revert and EDID identity on a ThinkPad T490s, 2026-10-20

## M3 — KDE Plasma backend

- [x] P-021 `kde_output_device_v2` snapshot: modes, position, rotation, scale, VRR policy, identity
- [x] P-022 `kde_output_management_v2` apply; output priority (`kde_output_order_v1`) for the primary output
- [ ] P-040 Mirroring on Plasma 6.3+ (`set_replication_source`): implemented, verify on real Plasma
- [x] P-041 Run the headless `KWin` test (`crates/vnr-kde/tests/kwin_virtual.rs`) in CI
- [x] P-042 Verify on real Plasma 6 hardware: snapshot, scale, rotation, revert on the maintainer's ThinkPad T490s, 2026-10-21
- [ ] P-046 Verify VRR policy on Plasma with an adaptive-sync monitor

## M4 — GNOME backend

- [x] P-023 `org.gnome.Mutter.DisplayConfig` `GetCurrentState` via `zbus`
- [x] P-024 `ApplyMonitorsConfig`: verify (as `test`) and temporary; mirroring as shared logical monitors; physical and logical layout modes
- [ ] P-043 Persistent GNOME changes without GNOME Shell's own confirm dialog doubling Vernier's: implemented (`--save`, Save for GNOME; GNOME's dialog replaces Vernier's prompt; checked on headless Mutter), verify GNOME Shell's dialog with P-045
- [x] P-044 Run the headless Mutter test (`crates/vnr-mutter/tests/mutter_headless.rs`) in CI
- [ ] P-045 Verify on real GNOME hardware, including fractional scaling in logical layout mode

## M5 — Release hardening

- [x] P-025 CI: fmt, clippy `-D warnings`, tests, `cargo audit`, `cargo deny` (`deny.toml`)
- [x] P-026 Persist through the compositor: write niri `output` blocks to an included file on request (`--persist`, editor "Save for niri"; checked with niri 26.04 `validate`)
- [x] P-027 Export to shikane, kanshi and a home-manager snippet (optional)
- [x] P-028 Live screen previews via `ext-image-copy-capture-v1`, falling back to `wlr-screencopy` (optional; all eight transforms checked upright through both protocols on headless sway 1.12, rotations through screencopy on sway 1.9)

## M6 — Accessibility and input

- [ ] P-029 Accessible names and roles in the editor (AccessKit, once upstream iced supports it; decided 2026-10-21 to wait rather than move to the pop-os fork)
- [x] P-030 Remappable key bindings from `~/.config/vernier/config.ncl` (Nickel; `vnr config check`)
- [x] P-031 Screen-name overlays via layer-shell, as in wdisplays (optional; shown on headless sway)

## M7 — Packaging and documentation

- [x] P-032 Texinfo manual `doc/vernier.texi` (`make info html pdf`, zero warnings)
- [x] P-033 `packaging/guix.scm` and `packaging/PKGBUILD` (PKGBUILD built and packaged; `guix build -f packaging/guix.scm` built with Guix 1.5.0-unstable, substitutes from ci.guix.gnu.org, all 155 tests passing, `vnr` runs and the Info manual is installed; run in a container daemon with `--disable-chroot`, not on a Guix System)
- [x] P-034 Verify `packaging/default.nix` builds on Bravais (built on Steelbore OS Bravais from `main` at `6225035`; the 155 tests pass in the Nix sandbox, also verified earlier with nixpkgs in a container)
- [x] P-035 Requirement list in `AGENTS.md` (§20, Category C)
