# Vernier — TODO

Work package: bring `vnr` into conformance with the Spacecraft Software CLI
Standard v1.1 (plan item P-036). Items are ticked only when implemented and
verified by a test or a recorded run.

- [x] T-001 Global flags: `--json`, `--format`, `--fields`, `--dry-run`, `-v`/`-q`, `--color`/`--no-color`, `--absolute-time`, `-0`/`--print0`, `--yes`/`--force`
- [x] T-002 Output mode cascade (flag, `AI_AGENT`/`AGENT` presence, `CI`, TTY) and color precedence
- [x] T-003 `metadata` + `data` envelope with ISO 8601 UTC timestamps
- [x] T-004 Structured errors with canonical and tool-specific exit codes, single-line JSON on stderr
- [x] T-005 Diagnostics: `[ERROR]`/`[WARN]`/`[OK]`/`[INFO]` ladder, `diagnostic` envelope, severity floor
- [x] T-006 Noun-verb tree: `output list|get|set`, `profile save|apply`, `layout export`, `backend get`, `gui`
- [x] T-007 `vnr schema` (JSON Schema 2020-12, derived from code) and `vnr describe`
- [x] T-008 `--dry-run` action plans for every command that changes state or writes a file
- [x] T-009 `--fields`, `jsonl`, `yaml`, `csv` and `--print0` output
- [x] T-010 Input validation: control characters, length bounds, path allow-list
- [x] T-011 `--help` with two or more examples per command and the project footer
- [x] T-012 Panic hook that reports `INTERNAL_ERROR` with exit code 1
- [x] T-013 Fixture backend so the CLI can be tested without a display server
- [x] T-014 Theme tokens in `vnr-theme`, shared by CLI colors and the editor
- [x] T-015 Compliance integration tests covering the CLI Standard test categories
- [x] T-016 `SKILL.md` at the repository root
- [ ] T-017 `--format explore` terminal table viewer (ratatui), with CUA and Vim keys
- [ ] T-018 Cross-shell round-trip tests through jaq, Nushell and PowerShell in CI
- [ ] T-019 `vnr schema` provider wrappers for Anthropic, OpenAI and Gemini (optional)
- [ ] T-020 MCP server via `vnr mcp` (optional)
