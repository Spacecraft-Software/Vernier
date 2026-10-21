---
name: vernier
description: Inspect and change display layout (resolution, refresh rate, position, scale, rotation, mirroring, adaptive sync) on Wayland and X11 with the `vnr` command line; save and apply display profiles; export compositor config for niri, sway, kanshi, shikane and home-manager.
license: GPL-3.0-or-later
---

<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Vernier (`vnr`)

Use `vnr` when asked about connected monitors or to arrange them. It talks to
the running display server directly: wlr-output-management (niri, sway,
Hyprland, river), COSMIC, and later KDE, GNOME and X11.

## Start here

```sh
vnr describe --json          # commands, flags, formats, environment
vnr schema output set --json # parameters + output schema of one command
vnr backend get --json       # which backend and what it can do
vnr output list --json       # every output with modes and settings
```

Under an agent (`AI_AGENT` / `AGENT` set) or with stdout piped, output is a
compact `{"metadata": {...}, "data": ...}` JSON document; nothing prompts.
Trim tokens with `--fields name,mode,x,y,scale` or stream with
`--format jsonl`.

## Changing displays safely

1. Preview first: `vnr output set DP-1 --right-of eDP-1 --scale 1.25 --dry-run --json`.
   `data.changes` lists every setting that moves, `data.issues` any layout
   problems, and `data.accepted_by_backend` whether the compositor accepts it.
2. Apply with `--yes`; without a terminal the change is kept.
3. Before risky changes, save the current layout so it can be restored:
   `vnr profile save ./before.toml` then `vnr profile apply ./before.toml --yes`.
4. Changes over wlr-output-management are lost when the compositor restarts;
   on niri, add `--persist ~/.config/niri/vernier.kdl` to the change (only when
   the user asks for a permanent change). It never writes `config.kdl` and never
   replaces a file Vernier did not generate. For sway, kanshi, shikane or
   home-manager use `vnr layout export --to <format>`.

Running the same `set` or `profile apply` twice is safe: the second run
reports `"outcome": "unchanged"`.

## Errors

Failures exit non-zero and print one JSON line on stderr:
`{"error": {"code", "exit_code", "message", "hint", ...}}`. Run the `hint`;
it is a command. Codes: 2 usage, 3 not found, 4 path outside the allow-list
(widen with `VNR_ALLOWED_PATHS`), 5 file exists (add `--yes`), 10 no display
server reachable, 11 backend lacks the feature, 12 compositor rejected the
configuration, 13 protocol error, 15 the layout is invalid (see
`error.issues`).

## Do not

- Do not run `vnr` or `vnr gui` under an agent; the editor needs a person.
- Do not turn off every output; `vnr` refuses (exit 15), but do not try.
- Do not write compositor config unless the user asked for a permanent change.
