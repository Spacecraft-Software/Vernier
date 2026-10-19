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
| `crates/vnr-backends` | Run-time detection, `open()`, and the `fixture` backend (`VNR_FIXTURE=<file.json>`). |
| `crates/vnr-theme` | Steelbore tokens and §11.6 theme resolution, shared by CLI colors and the editor. `assets/steelbore.toml` is the canonical palette, never edited by hand. |
| `crates/vnr-gui` | iced editor. |
| `crates/vnr` | The `vnr` binary: CLI Standard v1.1 grammar, output modes, errors, schema. |

## Commands

```nushell
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo run -p vnr -- gui --demo        # editor on a sample layout, no display changes
```

CLI tests (`crates/vnr/tests/cli.rs`) run the real binary against the fixture
backend, so they need no display server. Headless end-to-end testing: run
sway with `WLR_BACKENDS=headless`, add outputs with `swaymsg create_output`,
then point `WAYLAND_DISPLAY` at it.

## CLI invariants (Spacecraft CLI Standard v1.1)

- Grammar is `vnr <noun> <verb>`. A new leaf command needs a `Spec` entry in
  `crates/vnr/src/spec.rs` (two or more examples, one with `--json`); a unit
  test fails otherwise.
- stdout carries data only, through `Out::emit`. Diagnostics go through
  `Out::diag` with a code listed in `spec::DIAGNOSTICS`; errors are
  `AppError` values returned to `main`, never printed by commands.
- Every error hint is a runnable command. New error codes go in
  `error::ErrorCode` with a documented exit code.
- Argument structs derive `JsonSchema`; their doc comments are both the
  `--help` text and the schema descriptions.
- Path arguments go through `validate::Roots::check`; commands that change
  state or write files honor `--dry-run` and need `--yes` to overwrite.
- Agent detection is presence-based (`AI_AGENT=claude-code_…` counts).

## Conventions

- Backends are synchronous and short-lived; callers open one per operation.
  Never cache a wlr serial across operations.
- Every apply path must keep the previous `Layout` and offer revert.
- Colors only through `theme::Steelbore` tokens; no hex literals in UI code.
- Library crates use canonical error structs (`BackendError`, `ProfileError`);
  only `vnr-gui` uses `anyhow`; `vnr` uses `AppError`.
- Commits are signed by the maintainer and pushed over HTTPS (Steelbore
  Standard §6.3–§6.4). Agents never handle signing keys.
