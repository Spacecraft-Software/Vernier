# Vernier — Product Requirements

Oct 18, 2026 · @Mohamed Hammad

## Overview

Vernier (`vnr`) is one memory-safe Rust program that arranges, rotates, scales and mirrors displays on every major Linux session: wlroots-family Wayland compositors, COSMIC, KDE Plasma, GNOME and X11. It replaces wdisplays (C, GTK 3, wlr only) and ARandR (Python, PyGTK, X11 only) with a graphical editor and a scriptable command line that share one model and one set of backends.

**Problems with what exists today**

- wdisplays warns that its auto-apply can leave the user with an unusable setup, and it has no revert.
- wdisplays does not save anything; its own FAQ sends users to kanshi.
- ARandR scrapes `xrandr` text output. Mode names that are not `WIDTHxHEIGHT` broke it for years (Debian bug #507521).
- ARandR does not notice changes made while it runs, and it saves layouts as shell scripts.
- Each desktop (GNOME, KDE, COSMIC) ships its own settings panel; nothing works across all of them, and none work on niri or sway.

**Goals**

1. One tool for every Linux session the maintainer uses, starting with niri and COSMIC on Steelbore OS Bravais.
2. No configuration change can leave the user without a working screen: every apply is tested first where the server allows it, and reverted unless confirmed.
3. Speak each display protocol directly. Never parse another program's text output.
4. Profiles that survive a monitor moving to another port, and an export path that makes transient changes permanent.
5. A command line that is as capable as the editor and pipes cleanly into Nushell.

**Non-goals**

- An automatic hotplug daemon. shikane and kanshi already do this; Vernier exports to their formats instead.
- Color management, ICC profiles, gamma, brightness or HDR.
- Changing the console (TTY) resolution or bootloader framebuffer.
- Windows or macOS.

## Users and use cases

The primary user is the maintainer on Steelbore OS Bravais (niri, Nushell) and on COSMIC; the secondary user is anyone on a tiling compositor who today juggles wdisplays, wlr-randr and hand-written config.

| Who | Session | What they need |
| --- | --- | --- |
| Laptop user docking at a desk | niri, COSMIC, sway | Plug in a monitor, drag it into place, keep the layout next time |
| Presenter | Any | Mirror the laptop panel to a projector in two clicks, undo afterwards |
| Kiosk or signage builder | wlr compositors, X11 | Exact positions, rotation, scripted from a profile at boot |
| Distro maintainer (Bravais) | niri | Generate the niri `output` blocks declaratively from a profile |
| Scripter | Any | `vnr list --json` and `vnr set` from Nushell, with no prompts in scripts |
| X11 holdout | i3, Openbox, Xfce | An ARandR that understands every mode name and live changes |

**Key scenarios**

1. **Dock.** User connects a 4K monitor. Opens `vnr`, drags it right of the laptop, sets scale 1.5, presses Apply, presses Keep. Runs `vnr save desk.toml`.
2. **Recover.** User picks a mode the monitor cannot display. Every screen goes dark. After 15 seconds Vernier restores the previous layout without any input.
3. **Present.** User sets HDMI-A-1 to mirror eDP-1 (COSMIC, KDE, GNOME, X11), presents, then turns mirroring off.
4. **Persist on niri.** User adds `--persist ~/.config/niri/vernier.kdl` to a change (or presses Save for niri in the editor); once kept, the layout is written to a file niri's config includes, so it survives a niri restart.
5. **Script.** A Nushell script applies `desk.toml` with `--yes` when a dock is detected.

## Scope

Version 1.0 ships five backends through four implementations: COSMIC extends wlr-output-management rather than replacing it, so one backend serves niri, sway, Hyprland and COSMIC. niri and COSMIC are the first test targets.

| Backend | Sessions | Protocol | Mirroring | Adaptive sync | Primary | Test before apply | Saves changes itself |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `wlr` | niri, sway, Hyprland, river, labwc, Wayfire | wlr-output-management v4 | No | On/off (v4) | No | Yes | No |
| `cosmic` | COSMIC | wlr v4 + `zcosmic_output_manager_v1` v3 | Yes | Off / automatic / always | Xwayland primary | Yes | No |
| `kde` | KDE Plasma 6 | `kde_output_device_v2` + `kde_output_management_v2` | Yes (replication) | Yes | Output order | No | Yes (KScreen) |
| `mutter` | GNOME 46+ | `org.gnome.Mutter.DisplayConfig` D-Bus | Yes (shared logical monitor) | Yes | Yes | Yes (verify) | Yes (`monitors.xml`) |
| `x11` | Any X11 session | `RandR` 1.5 over the wire | Yes (shared CRTC position) | No | Yes | No | No |

KDE and GNOME capabilities are planned from their protocol definitions and will be confirmed when each backend lands.

**Out of scope for 1.0**

- Live screen previews inside the canvas (wdisplays' "Show Screen Contents"). Planned as optional work after 1.0.
- Screen-name overlays on each physical display. Optional in M6.
- A hotplug daemon, Weston, Mir, and Wayland compositors with no output-management protocol.
- Custom modelines and EDID overrides.

## Functional requirements

Each requirement keeps its identifier for life; a withdrawn one is struck through, never renumbered. Priority is mandatory, expected or optional; status is verified only when its test passes in CI or its demonstration is recorded.

**Model and safety**

| ID | Requirement | Priority | Verified by | Status |
| --- | --- | --- | --- | --- |
| R-001 | Vernier shall represent every output with its name, identity (make, model, serial), modes, enabled state, mode, position, transform, scale in thousandths, adaptive sync, mirror source and primary flag. | mandatory | Test | Verified |
| R-002 | Vernier shall refuse to apply a layout in which no output is enabled. | mandatory | Test | Verified |
| R-003 | Vernier shall warn, without blocking, when placed outputs overlap or an output shares no edge with any other. | expected | Test | Verified |
| R-004 | Vernier shall ask the server to test a configuration before applying it, on every backend that supports testing. | mandatory | Test | Verified (wlr) |
| R-005 | After every interactive apply, Vernier shall restore the previous configuration unless the user confirms within 15 seconds. | mandatory | Test + demonstration | Verified (wlr) |
| R-006 | A non-interactive apply (no terminal, or `--yes`) shall keep the change without prompting. | mandatory | Test | Verified |
| R-007 | Vernier shall match a requested refresh rate to an advertised mode within 0.5 Hz. | expected | Test | Verified |

**Backends**

| ID | Requirement | Priority | Verified by | Status |
| --- | --- | --- | --- | --- |
| R-010 | Vernier shall read and apply configurations through wlr-output-management versions 1 to 4. | mandatory | Test (headless sway) | Verified |
| R-011 | When `zcosmic_output_manager_v1` is advertised, Vernier shall support mirroring, scale in thousandths, three-state adaptive sync and the Xwayland primary output. | mandatory | Demonstration on COSMIC | Implemented, unverified |
| R-012 | Vernier shall configure X11 outputs through RandR 1.5 without running `xrandr`. | mandatory | Test (Xvfb) | Planned |
| R-013 | Vernier shall configure KDE Plasma 6 through kde-output-management-v2. | mandatory | Demonstration | Planned |
| R-014 | Vernier shall configure GNOME through `org.gnome.Mutter.DisplayConfig`, using the verify method for tests. | mandatory | Demonstration | Planned |
| R-015 | Vernier shall choose a backend from the session, and accept `--backend` or `VNR_BACKEND` to override it. | mandatory | Test | Verified |
| R-016 | Each backend shall report its capabilities, and the editor shall hide controls the backend cannot apply. | mandatory | Test | Verified |
| R-017 | The editor shall refresh its view when the server reports an output change made elsewhere. | expected | Demonstration | Planned |

**Profiles and export**

| ID | Requirement | Priority | Verified by | Status |
| --- | --- | --- | --- | --- |
| R-020 | `vnr save` shall write the current configuration as a TOML profile; `vnr apply` shall restore it. | mandatory | Test | Verified |
| R-021 | A profile entry naming make, model and serial shall match that display on any connector. | mandatory | Test | Verified |
| R-022 | A profile with an unknown key shall be rejected with the key named. | expected | Test | Verified |
| R-023 | `vnr export` shall print niri and sway configuration equivalent to a layout. | mandatory | Test | Verified |
| R-024 | On request, Vernier shall write niri `output` blocks to a separate included file, never editing the main config. | expected | Test | Verified |
| R-025 | `vnr export` shall also produce shikane, kanshi and home-manager formats. | optional | Test | Verified |

**Interfaces**

| ID | Requirement | Priority | Verified by | Status |
| --- | --- | --- | --- | --- |
| R-030 | The editor shall let the user drag outputs, snapping edges within 12 screen pixels; Shift shall disable snapping. | mandatory | Demonstration | Verified |
| R-031 | The editor shall set enabled, mode, scale, rotation, position, mirror, adaptive sync and primary for the selected output. | mandatory | Test + demonstration | Verified |
| R-032 | Every editor action shall be reachable from the keyboard, with CUA and Vim bindings. | mandatory | Test | Verified |
| R-033 | Key bindings shall be remappable from the configuration file. | mandatory | Test | Verified |
| R-034 | The CLI shall offer `list`, `backend`, `save`, `apply`, `set`, `export` and `gui`; `list` and `backend` shall print JSON on request. | mandatory | Test | Verified |
| R-035 | `vnr gui --demo` shall run the editor on a sample layout without contacting any display server. | expected | Test | Verified |

## Non-functional requirements

Stability comes first, then performance, then security (Steelbore Standard §3). The budgets below are proposed ceilings for the maintainer to confirm; the binary size is measured from the 0.1.0 release build.

| ID | Requirement | Priority | Verified by |
| --- | --- | --- | --- |
| R-100 | The workspace shall contain no `unsafe` code (`unsafe_code = "forbid"`). | mandatory | Inspection (compiler lint) |
| R-101 | No library crate shall panic on malformed profiles, protocol messages or EDID data. | mandatory | Test, fuzzing of the profile parser |
| R-102 | A backend failure shall leave the previous configuration in place and report the error. | mandatory | Test |
| R-103 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` shall pass. | mandatory | CI |
| R-110 | `vnr list` shall finish within 100 ms on the reference machine (ThinkPad T490s, two outputs). | expected | Benchmark in CI |
| R-111 | The editor shall show its first frame within 500 ms of launch on the reference machine. | expected | Benchmark |
| R-112 | The release binary shall stay under 16 MB (12.6 MB measured at 0.1.0). | expected | CI size check |
| R-113 | The editor shall use under 1 % CPU while idle with no confirmation pending. | expected | Measurement |
| R-120 | Vernier shall open no network connection and send no telemetry. | mandatory | Inspection + `cargo deny` |
| R-121 | `cargo audit` and `cargo deny` shall pass before each dependency addition and in CI. | mandatory | CI |
| R-122 | The binary shall link only libc; Wayland, xkbcommon and GPU libraries shall be loaded at run time. | expected | Inspection (`ldd`) |
| R-130 | Colours shall come only from the named Steelbore theme, resolved per Standard §11.6; Steelbore Modern is the default. | mandatory | Test |
| R-131 | All 21 registered themes shall resolve to complete token sets. | mandatory | Test |
| R-132 | Accessible mode shall follow §18.1 precedence and switch to the high-contrast theme. | mandatory | Test |
| R-133 | Overlap and other problems shall be stated in words, never by colour alone. | mandatory | Inspection |
| R-134 | The editor shall publish accessible names and roles for every control to screen readers. | mandatory | Demonstration with Orca |
| R-135 | Timestamps written by Vernier shall be ISO 8601 UTC with a `Z` suffix. | mandatory | Test |

R-134 depends on AccessKit support in iced; until then the CLI is the accessible path, recorded in the tailoring register.

## User experience

The editor is a canvas of output rectangles on the left and a properties panel on the right; Apply turns the footer into a Keep/Revert prompt with a countdown, so the canvas never moves under the pointer.

**Editor**

- **Canvas.** Each placed output is a rectangle at its logical size, labelled with its name, resolution, scale, and any mirrors. The selected output has a focus border. Drag to move; edges snap within 12 screen pixels; Shift places freely. The view fits the layout and freezes during a drag.
- **Panel.** Output list (on, off, or "mirrors X"), then the selected output's identity, physical size, Enabled toggle, Mode, Scale, Rotation, Position (x, y), Mirror, Adaptive sync and Primary. Controls the backend cannot apply are hidden. A Problems list states every error and warning in words.
- **Footer.** Backend name, status, Reload, Reset and Apply. Apply is disabled while there are errors or nothing changed.
- **Theme.** Steelbore Modern by default; Share Tech Mono for headings and Inconsolata for body text, falling back to the system fonts.

**Key bindings (CUA and Vim)**

| Action | CUA | Vim |
| --- | --- | --- |
| Move selected output 1 px (Shift: 10 px) | Arrow keys | h j k l |
| Next / previous output | Page Down / Page Up | \] / \[ |
| Apply | Ctrl+Enter, Ctrl+S |  |
| Keep / revert pending change | Keep / Revert buttons | y / n |
| Discard edits, or revert a pending change | Escape |  |
| Reload from the server | F5, Ctrl+R |  |

Tab is left to move keyboard focus between controls. Plain-letter bindings apply only when no text field has focus.

Every binding can be changed in `~/.config/vernier/config.ncl` (Nickel), by action name: `apply`, `reload`, `discard`, `keep`, `revert`, `next`, `previous`, `left`, `right`, `up`, `down`. `vnr config check` validates the file.

**Command line**

```
vnr [--backend wlr|cosmic|kde|mutter|x11] [--verbose] <command>
  gui     [--theme SLUG] [--accessible | --no-accessible] [--demo]
  list    [--json]
  backend [--json]
  save    PATH|- [--force]
  apply   PATH [--test] [--yes] [--timeout SECONDS]
  set     OUTPUT [--on|--off] [--mode WxH[@Hz] | --preferred] [--pos X,Y
          | --left-of|--right-of|--above|--below OUTPUT] [--scale F]
          [--transform T] [--adaptive-sync S] [--mirror OUTPUT | --no-mirror]
          [--primary] [--test] [--yes] [--timeout SECONDS]
  export  --format niri|sway|kanshi|shikane|home-manager [--profile PATH]
```

With no command, `vnr` opens the editor. Changes go to standard error; data goes to standard output; exit status is non-zero on any refusal.

**Profile format**

```toml
[[output]]
name = "DP-3"
make = "Dell Inc."
model = "U2720Q"
serial = "ABC123"
mode = "3840x2160@60Hz"
position = { x = 1920, y = 0 }
scale = 1.5
transform = "normal"
adaptive-sync = "automatic"

[[output]]
name = "eDP-1"
mirror = "DP-3"
```

Fields left out keep the output's current value.

## Architecture

Vernier is a Cargo workspace of eight crates; everything that does not need a display server lives in `vnr-core`, and each protocol is one backend crate behind a single `Backend` trait.

&#91;embedded content: crate layout · 8 crates, 4 backends\]

The binary and the editor never talk to a server directly: they ask `vnr-backends` for a `Box<dyn Backend>` and work on `vnr_core::Layout` values.

**Decisions**

- **Backend trait.** `kind`, `capabilities`, `snapshot`, `test` and `apply`. Backends are synchronous and short-lived: each action opens a fresh connection, so every apply is built from current state, as wlr-output-management's serials require.
- **COSMIC reuse.** COSMIC's display stack is an extension of wlr-output-management. Vernier vendors the protocol XML (MIT) from `pop-os/cosmic-protocols` and compiles it with `wayland-scanner`, binding it only when the compositor offers it. `cosmic-randr` (MPL-2.0) was studied for design only; no code was copied.
- **Concurrency.** Each action is a few protocol round trips, serial by nature; parallelism would add synchronization for no gain. Backend calls run on short-lived worker threads so the editor's UI thread never blocks.
- **Errors.** Library crates expose canonical error structs (`BackendError`, `ProfileError`, `ParseError`); only `vnr` and `vnr-gui` use `anyhow`.
- **Dependencies.** iced 0.14, wayland-client 0.31, wayland-protocols-wlr 0.3, clap 4, serde, toml. Planned: x11rb, zbus, wayland-protocols-plasma, all pure Rust. `mimalloc` is not used: Vernier barely allocates, and it would add native code.
- **Build.** Release builds use LTO, one codegen unit and `panic = "abort"`; `target-cpu=native` and PGO are off so distro packages run everywhere.

## Milestones

Version 1.0 needs all eight milestones, because accessibility (M6) and packaging (M7) carry mandatory requirements; `PLAN.md` holds the task-level breakdown.

&#91;embedded content: milestones M0 to M7 · one release gate\]

M1 still needs verification on niri and COSMIC hardware and live change tracking. No dates are set; the project runs at hobby pace.

## Assurance and verification

Vernier is Category C under the Steelbore Standard: a bad configuration is visible and recoverable, because every interactive apply reverts unless confirmed. The claim in the README is "Conforms to The Steelbore Standard v2.12 — Category C, tailored".

**How each requirement is verified**

- **Unit tests** cover the core model, geometry, validation, profiles, exporters, theme resolution, CLI parsing and editor state (50 tests at 0.1.0).
- **Headless integration tests** run each backend against a real server: sway with `WLR_BACKENDS=headless` for wlr, Xvfb for X11. CI runs both.
- **Demonstrations** cover what CI cannot reach: COSMIC, KDE, GNOME and screen-reader use. Each is recorded with date, versions and hardware.
- **The traceability matrix** maps every R- identifier to its tests and is generated in CI from test names.

**Tailoring register**

| Date | Clause | Gap | Closes in |
| --- | --- | --- | --- |
| 2026-10-18 | §18.3 | No accessible names or roles in the editor (iced has no AccessKit yet) | M6 |
| 2026-10-18 | §8 | No Texinfo manual | M7 |
| 2026-10-18 | §5.5 | Guix and PKGBUILD packages missing; Nix package not yet built on Bravais | M7 |
| 2026-10-18 | §20 | Requirements kept in this PRD, not yet mirrored in `AGENTS.md` | M7 |

## Open questions

- [ ] **Short code.** Is `VNR` the registry short code, matching the binary name?
- [x] **Configuration language.** Nickel for settings (`config.ncl`), TOML for profiles (decided 2026-10-21).
- [x] **Persistence on niri.** Only on request: `--persist FILE` on `output set` and `profile apply`, and Save for niri in the editor (decided 2026-10-21).
- [ ] **Budgets.** Confirm or change the proposed ceilings in R-110 to R-113.
- [x] **Accessibility path.** Wait for AccessKit in upstream iced; the CLI stays the accessible path until then (decided 2026-10-21).
- [ ] **Backend order.** X11 (M2) before KDE (M3) and GNOME (M4), or the other way round?
- [ ] **Upstream.** Propose the COSMIC mirroring semantics for wlr-output-management upstream, or stay a consumer?
