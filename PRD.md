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

- A hotplug daemon, Weston, Mir, and Wayland compositors with no output-management protocol.
- Custom modelines and EDID overrides.

## Requirements

The requirement set moved to [`AGENTS.md`](AGENTS.md#requirements-steelbore-standard-20-category-c) on 2026-10-22 (Steelbore Standard §20.1, Category C), where coding agents load it. The old `R-NNN` identifiers map one to one: R-020 is now VNR-SRS-020. The needs those requirements trace to are listed in [`README.md`](README.md#needs) as VNR-NEED-001 to 005; the goals above are their source.

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
- **The traceability matrix** is planned: it will map every VNR-SRS identifier to its tests, generated in CI.

**Tailoring register**

| Date | Clause | Gap | Closes in |
| --- | --- | --- | --- |
| 2026-10-18 | §18.3 | No accessible names or roles in the editor (iced has no AccessKit yet) | When upstream iced ships AccessKit |
| 2026-10-18 | §5.5 | Packages build from the checkout; release tarball and checksums come with the first tag | First release |

## Open questions

- [x] **Short code.** `VNR`, matching the binary; requirement identifiers are `VNR-SRS-NNN` (decided 2026-10-22).
- [x] **Configuration language.** Nickel for settings (`config.ncl`), TOML for profiles (decided 2026-10-21).
- [x] **Persistence on niri.** Only on request: `--persist FILE` on `output set` and `profile apply`, and Save for niri in the editor (decided 2026-10-21).
- [ ] **Budgets.** Confirm or change the proposed ceilings in VNR-SRS-110 to 113. The release binary measured 28.8 MB on 2026-10-22 (Nickel and the screen-label text stack), over the 16 MB draft in VNR-SRS-112.
- [x] **Accessibility path.** Wait for AccessKit in upstream iced; the CLI stays the accessible path until then (decided 2026-10-21).
- [ ] **Backend order.** X11 (M2) before KDE (M3) and GNOME (M4), or the other way round?
- [ ] **Upstream.** Propose the COSMIC mirroring semantics for wlr-output-management upstream, or stay a consumer?
