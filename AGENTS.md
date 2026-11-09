# Agent instructions

Every agent working in this repository, whatever the tool, follows the project rules in `.cursor/rules/`. They are binding, and they apply before any prompt in `docs/HANDOFF.md`.

| Rule file | Applies to |
| --- | --- |
| `.cursor/rules/agentic-instruction-following.mdc` | All work: small slices, transparency, no failing tests |
| `.cursor/rules/code-design-qa.mdc` | All code: SOLID, DRY, logging, null guards, no single-line `if` |
| `.cursor/rules/documentation-rules.mdc` | All docs: no emojis, no em dashes |
| `.cursor/rules/ui-ux-standards.mdc` | UI work: reuse existing components and tokens, keep accessibility |

Before you push:

1. `cd fx-core && cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings` (CI runs this on Linux and Windows, so a module used only from `window.rs` needs `#![cfg_attr(not(windows), allow(dead_code))]`)
3. `cargo test`

Then follow the handoff protocol in `CONTRIBUTING.md`.
