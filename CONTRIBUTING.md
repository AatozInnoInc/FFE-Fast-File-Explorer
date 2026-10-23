# Contributing

## Before you write code

Read, in order: `docs/HANDOFF.md`, `docs/ARCHITECTURE.md`, and this file. The active work is always the single **Prompt for Next Agent** at the bottom of `HANDOFF.md`.

## The handoff protocol

Every contributor — human or agent — follows the same cycle:

1. Read the docs above.
2. Do the work in the current **Prompt for Next Agent**.
3. Update every document and chart the work affected, and adjust `docs/ROADMAP.md` status.
4. Move the completed prompt into the Handoff Log with an ISO 8601 sign-off timestamp.
5. Write the next **Prompt for Next Agent**.

## Design changes

Design changes require sign-off from the frontier tier (see **Model selection** in `docs/HANDOFF.md`). Do not quietly change architecture inside an implementation task — raise it, get sign-off, then record it in `ARCHITECTURE.md`. In particular: the decision to use native Win32 controls plus targeted Direct2D owner-draw (instead of a fully custom renderer) is a load-bearing architectural choice made for stability reasons — do not revert to a custom-render approach without an explicit design review.

## Documentation discipline

- No fabricated numbers. Unset constraints are written as `TODO — define`, never as fake measured values. The performance/resource budgets in `ARCHITECTURE.md` are proposed *targets*, explicitly not yet measured — replace them with real benchmark results once available, and update this note when that happens.
- Describe control flow accurately, including short-circuits (e.g., the cache-hit path in navigation must be shown as an early exit, not folded into the cold-enumeration path).
- Keep charts and prose in sync.
- "Domain" means a DDD bounded context unless stated otherwise.

## Style and tooling

- **Core engine (Rust):** `rustfmt` with no custom configuration (no `rustfmt.toml` overrides). `clippy` run on CI with warnings denied. Use modern Rust idioms; where FFI/C conventions and Rust conventions compete, follow Rust naming conventions (e.g., CamelCase wrapper types even over snake_case C originals).
- **UI shell (C++, Win32/Direct2D):** a checked-in `.clang-format` file based on the LLVM or Google style, with `UseTabs: Never` set explicitly to avoid mass re-indentation on first format. Static analysis via MSVC `/analyze` or Clang-Tidy as a CI gate, mirroring the clippy gate on the Rust side. The list, folder tree, address bar, Back/Forward, extension icons, DirectWrite Size and Date modified columns, background image thumbnails, native context menu, and lazy Network node are the Rust binary `fx-core/src/bin/list_shell.rs`. More options is shown and is not an in-process shell extension. It calls `fx_core::shell` in-process. A C ABI is not part of that path. The C++ formatting note remains for later native UI code; the base style is still an open question.
- **Testing:** unit tests are required for all core-engine logic before it is considered done (see the definition-of-done pattern in `HANDOFF.md`'s prompts). UI-level testing conventions (e.g., WinAppDriver automation) are deferred to the v1.5/v2 roadmap items and will be defined when that work starts.
