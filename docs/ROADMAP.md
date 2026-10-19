Status legend: `planned` · `in progress` · `done` · `deferred`

---

# Roadmap

## v1 — Native, fast browsing of local NTFS SSD volumes

The smallest slice that delivers real value: open, navigate, and scroll local folders — including very large ones — faster than Windows 11 Explorer, with per-volume isolation guaranteed from day one so no drive or network location can ever block the UI. "Done" means a folder of 100k+ files opens and scrolls without perceptible stutter, verified against the performance budgets in `docs/ARCHITECTURE.md`.

- [x] Rust core engine crate: `FindFirstFileEx` enumeration streaming `FileEntry` batches (`fx-core`) — `done`
- [x] In-memory listing cache (cache-hit short-circuit; eviction `TODO — define`) — `done`
- [x] Per-volume isolated workers with cancellation tokens — no shared threads or blocking calls across volumes — `done`
- [x] Network/non-local enumeration timeout in the core (default 2 s, configurable). The caller polls `Listing::try_recv` and is not blocked during the wait — `done`
- [ ] "Network" tree node: lazy, collapsed by default, never eagerly enumerated — `planned`
- [ ] FFI/IPC boundary between Rust core and native Win32 shell — `planned`
- [ ] Win32 shell: `SysListView32` in virtual/owner-data mode wired to the core — `planned`
- [ ] `SysTreeView32` folder tree, same virtualized data pattern — `planned`
- [ ] Basic navigation: address bar, back/forward, keyboard nav — `planned`
- [ ] Icon caching by file extension (avoid per-file `SHGetFileInfo` calls) — `planned`
- [x] CI: unit tests for the core engine, `rustfmt`, and `clippy` gate on merge — `done`

## v1.5 — Visual polish and safety nets

Refinements and gaps that follow naturally from v1.

- [ ] Direct2D/DirectWrite owner-draw layer for thumbnails and custom columns — `planned`
- [ ] Async thumbnail generation on a background worker pool — `planned`
- [ ] Minimal native context menu (copy/cut/paste/rename/delete/properties) with shell COM fallback for "More options" — `planned`
- [ ] Shell-extension and thumbnail-generation isolation in a watchdog-supervised worker process — `planned`
- [ ] Crash telemetry and dump collection — `planned`
- [ ] Settings surface for the network timeout value (default 2 s) — `planned`

## v2 — Power-user speed features

The next substantial capability step.

- [ ] NTFS MFT/USN Journal indexing engine: filename/metadata index, built from MFT reads, kept live via the USN Change Journal, NTFS local volumes only by default — `planned`
- [ ] Smart ranked search: fuzzy matching + frecency-based ranking, local-filesystem results only, never blended with web/online sources — `planned`
- [ ] Search-as-you-type across indexed volumes — `planned`
- [ ] Dual-pane browsing and tabs — `planned`

## Future / parking lot

Ideas worth keeping but not yet scheduled. No commitment implied.

- Cloud-sync placeholder file support (Cloud Files API / CfAPI integration).
- Scripting/plugin extensibility system, Directory Opus-style.
- Automated UI regression test suite (e.g., WinAppDriver-based).
- Full-text content search — status not yet decided; see Open design questions in `docs/ARCHITECTURE.md`.

## Notes

`fx-core` enumerates directories and caches finished listings as of 2026-10-19. The UI and FFI are not started. Cache eviction is `TODO — define`. SSD budget numbers are not measured. Confirmed during design discussion: SSD-only target (HDD support explicitly out of scope), active-use memory ceiling raised from an initial 50 MB draft to 512 MB (idle stays at 80 MB as the meaningful tripwire), cold-enumeration budget tightened from 2 s to 1 s given the SSD-only scope, and a hard requirement that no volume — network or otherwise — may ever block the UI, enforced via per-volume worker isolation plus a 2 s configurable timeout. Three items remain open (auto-retry behavior on timeout, whether full-text search is in scope, and whether frecency ranking is the default) — see `docs/ARCHITECTURE.md` Open design questions.
