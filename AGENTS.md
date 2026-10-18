# Vernier — agent context

Vernier (`vnr`) is a Rust display configurator replacing wdisplays (Wayland)
and ARandR (X11). Spacecraft Software project; maintainer Mohamed Hammad.

- **Assurance category:** C (Steelbore Standard §19). A bad configuration is
  visible and recoverable: every apply is followed by confirm-or-revert.
- **Plan and progress:** `PLAN.md` (§17.5 format). Tick an item only when it
  is implemented and verified.
- **License:** GPL-3.0-or-later. Every source file carries the two-line SPDX
  header. `unsafe` is forbidden.

## Layout

| Crate | Role |
|---|---|
| `crates/vnr-core` | Model, geometry, validation, profiles, exporters, `Backend` trait. No I/O. |
| `crates/vnr-wlr` | wlr-output-management v4 + COSMIC extension (vendored XML in `protocols/`). |
| `crates/vnr-kde`, `vnr-mutter`, `vnr-x11` | Backend stubs; see PLAN.md M2–M4. |
| `crates/vnr-backends` | Run-time detection, `open()`. |
| `crates/vnr-gui` | iced editor; `assets/steelbore.toml` is the canonical palette, never edited by hand. |
| `crates/vnr` | The `vnr` binary (clap CLI, launches the editor). |

## Commands

```nushell
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo run -p vnr -- gui --demo        # editor on a sample layout, no display changes
```

Headless end-to-end testing: run sway with `WLR_BACKENDS=headless`, add
outputs with `swaymsg create_output`, then point `WAYLAND_DISPLAY` at it.

## Conventions

- Backends are synchronous and short-lived; callers open one per operation.
  Never cache a wlr serial across operations.
- Every apply path must keep the previous `Layout` and offer revert.
- Colors only through `theme::Steelbore` tokens; no hex literals in UI code.
- Library crates use canonical error structs (`BackendError`, `ProfileError`);
  only `vnr` and `vnr-gui` use `anyhow`.
