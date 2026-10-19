# Vernier

Vernier (`vnr`) arranges, rotates, scales and mirrors displays. It replaces
[wdisplays](https://github.com/MichaelAquilina/wdisplays) on Wayland and
[ARandR](https://github.com/haad/arandr) on X11 with one memory-safe Rust
program: a graphical editor and a scriptable command line built on the same
backends.

Conforms to The Steelbore Standard v2.13 — Category C, tailored — and the
Spacecraft Software CLI Standard v1.1 (see
[Tailoring register](#tailoring-register)).

## Status

Pre-release (0.1.0). Milestone progress is tracked in [`PLAN.md`](PLAN.md).

| Backend | Compositors / sessions | State |
|---|---|---|
| `wlr` | niri, sway, Hyprland, river, labwc, Wayfire | Working; tested against headless sway |
| `cosmic` | COSMIC (wlr + `zcosmic_output_manager_v1` v3) | Implemented; not yet tested on COSMIC |
| `kde` | KDE Plasma (`kde-output-management-v2`) | Planned (M3) |
| `mutter` | GNOME (`org.gnome.Mutter.DisplayConfig`) | Planned (M4) |
| `x11` | Any X11 session (`RandR` 1.3+) | Working; tested against the Xorg dummy driver. Scaling and custom modes pending |

## What it does better than the originals

- **Confirm or revert.** Every change, from the editor or the command line,
  asks to be kept and reverts itself after 15 seconds. A configuration that
  blanks every screen fixes itself. (wdisplays warns that auto-apply can leave
  you with an unusable setup; ARandR has no undo.)
- **Test before apply.** On wlr and COSMIC the compositor checks a
  configuration before Vernier applies it.
- **No text scraping.** Vernier speaks the display protocols directly, so mode
  names such as `1080p` cannot break it the way they broke ARandR.
- **Profiles you can read.** `vnr profile save` writes TOML; entries match a monitor by
  make, model and serial, so a profile survives the monitor moving to another
  port.
- **Permanent where the compositor forgets.** Changes over wlr-output-management
  are lost when niri or sway restarts. `vnr layout export --to niri` (or `sway`)
  prints the config block that makes them permanent.
- **Mirroring on COSMIC**, through COSMIC's own protocol extension.

## Usage

Commands follow `vnr <noun> <verb>`:

```nushell
vnr                                       # open the editor
vnr gui --demo                            # try the editor on a sample layout
vnr output list                           # outputs, modes and settings
vnr output list --json | from json | get data   # the same, as a Nushell table
vnr output get DP-1 --fields name,mode,scale
vnr backend get                           # which backend, and what it can do
vnr output set DP-1 --mode 2560x1440@144 --right-of eDP-1 --scale 1.25
vnr output set HDMI-A-1 --mirror eDP-1    # COSMIC only
vnr output set eDP-1 --off --dry-run      # plan it and ask the compositor; change nothing
vnr profile save ~/.config/vernier/desk.toml
vnr profile apply ~/.config/vernier/desk.toml --yes
vnr layout export --to niri --write ~/.config/niri/vernier.kdl
```

On a terminal `vnr` prints tables and asks before keeping a change. Piped, or
when `AI_AGENT`, `AGENT` or `CI` is set, it prints one JSON document
`{"metadata": …, "data": …}` on stdout and never prompts; diagnostics and
errors go to stderr, errors as one JSON line with a `hint` holding the command
to run next. `--format jsonl|yaml|csv`, `--fields` and `--print0` shape the
output, `--dry-run` previews any change or file write, and `--yes` keeps a
change or overwrites a file without asking.

`vnr schema` prints every command's JSON Schema (ready for LLM function
calling) and `vnr describe` the capability manifest. Exit codes: 0 success,
2 usage, 3 not found, 4 path not allowed, 5 conflict, 10 backend unavailable,
11 unsupported, 12 rejected by the compositor, 13 protocol error, 14 reverted,
15 invalid layout, 16 revert failed.

`--backend` (or `VNR_BACKEND`) forces `wlr`, `cosmic`, `kde`, `mutter` or
`x11`. File arguments must lie in the working directory or Vernier's XDG
directories; `VNR_ALLOWED_PATHS` adds more.

### Editor keys

| Action | CUA | Vim |
|---|---|---|
| Move the selected output by 1 px (Shift: 10) | Arrow keys | `h` `j` `k` `l` |
| Select the next / previous output | Page Down / Page Up | `]` / `[` |
| Apply | Ctrl+Enter, Ctrl+S | |
| Keep / revert a pending change | Keep / Revert buttons | `y` / `n` |
| Discard edits (or revert a pending change) | Escape | |
| Reload from the display server | F5, Ctrl+R | |

Drag an output to move it; edges snap to neighbors, hold Shift to place it
freely. Tab moves keyboard focus between controls.

## Building

```nushell
cargo build --release
```

Requires Rust 1.88 or later. The binary links only libc; the Wayland
protocol code is pure Rust. At run time the editor loads `libwayland-client`,
`libxkbcommon` and a Vulkan or OpenGL driver if present, and falls back to a
software renderer otherwise. On Steelbore OS Bravais,
[`packaging/default.nix`](packaging/default.nix) builds the package and wraps
the binary with those libraries.

## Design

- **Workspace.** `vnr-core` holds the model, geometry, validation, profiles and
  exporters and has no display-server dependency, so it is fully unit-tested.
  Each backend is its own crate implementing `vnr_core::Backend`;
  `vnr-backends` detects which to use; `vnr-gui` is the iced editor; `vnr` is
  the binary.
- **COSMIC.** COSMIC's display stack extends wlr-output-management rather than
  replacing it, so one backend serves niri and COSMIC. Vernier compiles the
  COSMIC protocol XML (MIT, from `pop-os/cosmic-protocols`) with
  `wayland-scanner`, and uses the extension only when the compositor offers it.
  `cosmic-randr` (MPL-2.0) was studied for its design but no code was copied.
- **Concurrency (§3.2).** The work is a handful of protocol round trips per
  action, which is serial by nature; parallelism would add synchronization and
  gain nothing. Backend calls run on short-lived worker threads so the UI
  thread never blocks, and each action opens a fresh connection so it always
  works from current state.
- **Allocator.** `mimalloc` (M-MIMALLOC-APPS) is deliberately not used: the
  program allocates almost nothing at run time, and the C allocator would add
  unsafe native code for no measurable benefit (Stability outranks
  Performance, §3).
- **Unsafe code** is forbidden workspace-wide (`unsafe_code = "forbid"`).
- **Component system (§13): Material Design**, as iced is custom-drawn, themed
  through the named `Steelbore` theme. Colors are read from the bundled copy of
  the canonical `steelbore.toml`; the theme resolves per §11.6 (`--theme`,
  `SPACECRAFT_THEME`, `steelbore/theme.toml`, default Steelbore Modern) and
  `--verbose` reports the decision.

## Project Posture

Vernier is a personal project maintained at hobby pace, provided as is, with
no warranty and no liability; see [`NOTICE.md`](NOTICE.md). Contributions are
welcome but not guaranteed to be accepted; see
[`CONTRIBUTING.md`](CONTRIBUTING.md). Report vulnerabilities as described in
[`SECURITY.md`](SECURITY.md). Forking is encouraged.

## Tailoring register

Category C keeps its register here (§19.5). Each entry is an obligation not
yet met, with the reason and the milestone that closes it.

| Date | Clause | Gap | Reason | Closes in |
|---|---|---|---|---|
| 2026-10-18 | §18.3 | GUI publishes no accessible names or roles | iced 0.14 has no AccessKit integration; the CLI is the accessible path meanwhile | M6 |
| 2026-10-18 | §10 | Key bindings are not yet user-remappable | Bindings are fixed in code until the config file lands | M6 |
| 2026-10-18 | §8 | No Texinfo manual | Interface still changing before 1.0 | M7 |
| 2026-10-18 | §5.5 | Only the Nix package exists; Guix and PKGBUILD are missing | Pre-release, no tag pushed | M7 |
| 2026-10-19 | CLI §2 Rule 1 | `--format explore` prints the table instead of an interactive viewer | ratatui viewer not built yet (TODO T-017) | M6 |
| 2026-10-19 | CLI testing | No cross-shell round-trip tests (Nushell, PowerShell, jaq) | Needs those shells in CI (TODO T-018) | M7 |
| 2026-10-18 | §20 | Requirements live as `PLAN.md` items, not an `AGENTS.md` requirement list | Scope still being settled | M7 |

## License

GPL-3.0-or-later; see [`LICENSE`](LICENSE). Third-party material is credited in
[`CREDITS.md`](CREDITS.md).

---

Maintainer: Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> ·
https://Vernier.SpacecraftSoftware.org/ · Built by Spacecraft Software
