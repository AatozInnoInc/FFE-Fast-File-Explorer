Status legend: `planned` · `in progress` · `done` · `deferred`

---

# Roadmap

## v1 — Native, fast browsing of local NTFS SSD volumes

The smallest slice that delivers real value: open, navigate, and scroll local folders — including very large ones — faster than Windows 11 Explorer, with per-volume isolation guaranteed from day one so no drive or network location can ever block the UI. "Done" means a folder of 100k+ files opens and scrolls without perceptible stutter, verified against the performance budgets in `docs/ARCHITECTURE.md`.

- [x] Rust core engine crate: `FindFirstFileEx` enumeration streaming `FileEntry` batches (`fx-core`) — `done`
- [x] In-memory listing cache (cache-hit short-circuit; eviction `TODO — define`) — `done`
- [x] Per-volume isolated workers with cancellation tokens — no shared threads or blocking calls across volumes — `done`
- [x] Network/non-local enumeration timeout in the core (default 2 s, configurable). The caller polls `Listing::try_recv` and is not blocked during the wait — `done`
- [x] "Network" tree node: lazy, collapsed by default, never eagerly enumerated (`list_shell`; `Shell::browse_network` on expand) — `done`
- [x] In-process call surface the Win32 shell uses to reach `fx-core` (`shell`: open, poll, cancel; no IPC) — `done`
- [x] Win32 shell: `SysListView32` in virtual/owner-data mode wired to the core (`list_shell`, one directory) — `done`
- [x] `SysTreeView32` folder tree, same virtualized data pattern (`list_shell`; children on expand; selection lists that folder) — `done`
- [x] Basic navigation: address bar, back/forward, keyboard nav (`list_shell`) — `done`
- [x] Icon caching by file extension (avoid per-file `SHGetFileInfo` calls; `list_shell` small image list) — `done`
- [x] CI: unit tests for the core engine, `rustfmt`, and `clippy` gate on merge — `done`

## v1.5 — Visual polish and safety nets

Refinements and gaps that follow naturally from v1.

- [x] Direct2D/DirectWrite owner-draw for custom columns (Size and Date modified; Name stays native) — `done`
- [x] Async thumbnail generation on a background worker pool (`min(4, logical cores)`; visible image rows; unbounded cache, eviction `TODO — define`) — `done`
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

`fx-core` enumerates directories, caches finished listings, and exposes an in-process `shell` module (open, poll, cancel) as of 2026-10-19. `list_shell` shows a directory in a virtual `SysListView32` and a `SysTreeView32` of child folders as of 2026-10-21. As of 2026-10-23, choosing a tree folder, the address bar, Back, and Forward list that directory, painted rows use a small icon cached by extension, Size and Date modified are DirectWrite cells, a visible image row requests a thumbnail on a background pool, and a Network node stays collapsed until the user expands it. The context menu is not started. Cache eviction is `TODO — define`. SSD budget numbers are not measured. Confirmed during design discussion: SSD-only target (HDD support explicitly out of scope), active-use memory ceiling raised from an initial 50 MB draft to 512 MB (idle stays at 80 MB as the meaningful tripwire), cold-enumeration budget tightened from 2 s to 1 s given the SSD-only scope, and a hard requirement that no volume — network or otherwise — may ever block the UI, enforced via per-volume worker isolation plus a 2 s configurable timeout. Three items remain open (auto-retry behavior on timeout, whether full-text search is in scope, and whether frecency ranking is the default) — see `docs/ARCHITECTURE.md` Open design questions.
