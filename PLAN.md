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
- [x] P-013 End-to-end test against headless sway
- [ ] P-014 Verify on niri (real hardware)
- [ ] P-015 Verify on COSMIC (real hardware), including mirroring
- [ ] P-016 Live change tracking (re-read state on compositor `done` events)
- [ ] P-036 CLI Standard v1.1 conformance (see TODO.md)

## M2 — X11 backend

- [x] P-017 `RandR` 1.3+ snapshot via `x11rb`: outputs, CRTCs, modes, EDID identity
- [x] P-018 Apply: CRTC config, screen resize, rotation, mirroring, primary
- [x] P-037 Scale via CRTC transform: verified on a ThinkPad T490s (Intel, modesetting), 2026-10-20
- [ ] P-038 Custom modes (`RRCreateMode` from CVT timings)
- [ ] P-019 `ScreenChangeNotify` live tracking
- [ ] P-020 Run the Xorg dummy-driver integration test (`crates/vnr-x11/tests/xorg_dummy.rs`) in CI
- [x] P-039 Verify on real X11 hardware: scale, rotation, revert and EDID identity on a ThinkPad T490s, 2026-10-20

## M3 — KDE Plasma backend

- [x] P-021 `kde_output_device_v2` snapshot: modes, position, rotation, scale, VRR policy, identity
- [x] P-022 `kde_output_management_v2` apply; output priority (`kde_output_order_v1`) for the primary output
- [ ] P-040 Mirroring on Plasma (replication source; not in the protocol versions `wayland-protocols-plasma` 0.3.12 ships)
- [ ] P-041 Run the headless `KWin` test (`crates/vnr-kde/tests/kwin_virtual.rs`) in CI
- [ ] P-042 Verify on real Plasma 6 hardware, including VRR

## M4 — GNOME backend

- [ ] P-023 `org.gnome.Mutter.DisplayConfig` `GetCurrentState` via `zbus`
- [ ] P-024 `ApplyMonitorsConfig` with verify, temporary and persistent methods

## M5 — Release hardening

- [ ] P-025 CI: fmt, clippy `-D warnings`, tests, `cargo audit`, `cargo deny`
- [ ] P-026 Persist through the compositor: write niri `output` blocks to an included file on request
- [ ] P-027 Export to shikane, kanshi and a home-manager snippet (optional)
- [ ] P-028 Live screen previews via `ext-image-copy-capture-v1` (optional)

## M6 — Accessibility and input

- [ ] P-029 Accessible names and roles in the editor (AccessKit, once iced supports it)
- [ ] P-030 Remappable key bindings from `~/.config/vernier/config.toml`
- [ ] P-031 Screen-name overlays via layer-shell, as in wdisplays (optional)

## M7 — Packaging and documentation

- [ ] P-032 Texinfo manual `doc/vernier.texi`
- [ ] P-033 `packaging/guix.scm` and `packaging/PKGBUILD`
- [ ] P-034 Verify `packaging/default.nix` builds on Bravais
- [ ] P-035 Requirement list in `AGENTS.md` (§20, Category C)
