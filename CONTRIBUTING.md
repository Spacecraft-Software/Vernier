# Contributing to Vernier

Contributions are welcome, but whether and when a change is merged is at the
maintainer's discretion. Open an issue before large changes.

- **Scope.** Display configuration: backends, the editor, the CLI, profiles
  and exporters. New backends are especially welcome.
- **Code.** Rust, following the Microsoft Pragmatic Rust Guidelines and the
  Steelbore Standard. No `unsafe` (it is forbidden workspace-wide). Before a
  pull request, run:

  ```nushell
  cargo fmt --all
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  cargo test --workspace --locked
  ```

- **Headers.** Every new source file starts with the two-line SPDX header
  (`SPDX-FileCopyrightText` and `SPDX-License-Identifier: GPL-3.0-or-later`).
- **Sign-off.** Add `Signed-off-by:` to each commit (Developer Certificate of
  Origin), and sign commits cryptographically.
- **License.** Contributions are accepted under GPL-3.0-or-later.
- **Security issues** go through [`SECURITY.md`](SECURITY.md), never a public
  issue.
