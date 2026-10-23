# Architecture

## Overview

Rapid Explorer is a native Windows file manager built around one core idea: keep the UI on retained-mode, OS-native controls (which already carry decades of accessibility, DPI, and stability hardening) and remove Explorer's actual bottlenecks — the hybrid Win32/WinUI/XAML Islands rendering stack, COM-based shell enumeration, and unbounded per-volume blocking — rather than replacing the UI paradigm entirely with a bespoke GPU renderer. Optimized for local NTFS SSD volumes; HDDs are not a target use case.

## Bounded contexts (DDD)

| Bounded context | Owns | Depends on |
| --- | --- | --- |
| UI Shell | Window chrome, `SysListView32`/`SysTreeView32` virtual-mode controls, navigation state, address bar | Enumeration and Indexing, Owner-draw Rendering, Shell Integration |
| Enumeration and Indexing | Directory enumeration (`FindFirstFileEx`), per-volume isolated workers, in-memory listing cache, NTFS MFT/USN Journal indexing for whole-volume search | Shared Kernel |
| Owner-draw Rendering | Direct2D/DirectWrite painting for thumbnails, custom columns, icon overlays via `NM_CUSTOMDRAW` | Shared Kernel |
| Shell Integration | Context menu (minimal native menu + shell COM fallback), "Open with", properties, third-party shell extension hosting | Resilience and Telemetry |
| Resilience and Telemetry | Watchdog process for isolating shell extensions and thumbnail generation, crash dump collection | — |

**Shared Kernel:** `FileEntry`, `Path`, `VolumeId` — the core value types every context reads, owned by the Enumeration and Indexing context and treated as immutable data contracts by consumers.

Embed the context map here: `docs/charts/context-map.md`.

## Key decisions

- **Decision:** Native Win32 controls (`SysListView32`/`SysTreeView32`, virtual/owner-data mode) instead of a custom-rendered UI.
  - **Alternatives:** Dear ImGui + Direct3D 11 custom renderer; Qt Widgets.
  - **Why:** Native controls inherit Windows' existing accessibility (Narrator/UI Automation), DPI scaling, and theming for free, and match the architecture used by the two file managers with the longest reliability track records (Total Commander, Directory Opus). ImGui redraws continuously and requires rebuilding accessibility/layout from scratch — a bad trade when quality and stability outrank raw build speed.

- **Decision:** Direct2D/DirectWrite for owner-draw cells only (thumbnails, custom columns), layered onto native controls via `NM_CUSTOMDRAW`.
  - **Alternatives:** GDI; a fully custom D3D11 render pipeline for the whole UI.
  - **Why:** Confines GPU-accelerated code to a narrow, well-documented integration point instead of owning the entire render loop, limiting the failure surface.

- **Decision:** Rust for the enumeration/indexing/caching core, in a UI-free crate the Win32 shell calls in-process.
  - **Alternatives:** C++ core. A separate process with IPC between the shell and the core.
  - **Why:** Memory safety for concurrent filesystem code, and a core that is independently unit-testable without any UI dependency. The owner chose in-process on 2026-10-19 so a cache hit does not pay a process hop before paint. The shell's only door is `fx_core::shell`: open, poll, and cancel. Shell extensions and thumbnail generation stay in their own watchdog process; that isolation is for untrusted code, not for `fx-core`.

- **Decision:** `FindFirstFileEx` with `FIND_FIRST_EX_LARGE_FETCH` for standard folder browsing; direct NTFS MFT/USN Journal reads for whole-volume search and size calculations.
  - **Alternatives:** `IShellFolder`/COM enumeration (what Explorer uses today).
  - **Why:** The COM shell-enumeration layer is a documented contributor to Explorer's perceived slowness; bypassing it for routine browsing removes that cost entirely.

- **Decision:** Every volume (drive letter, UNC path, or the "Network" tree node) gets its own independent, cancellable enumeration worker. No two volumes ever share a worker, a thread, or a blocking call.
  - **Alternatives:** A shared enumeration pool or single-threaded dispatcher (Explorer's effective behavior today, and the exact bug Explorer++ had to fix by moving enumeration off the UI thread).
  - **Why:** A slow or hung network share or a non-SSD drive must never be able to stall the UI thread, another volume's browsing, or another tab/pane. This is a hard requirement, not an optimization — isolation must hold even in the timeout/degraded case, not just the happy path.

- **Decision:** Network and non-local enumeration gets a hard timeout, default **2 seconds, user-configurable**. On timeout, the affected volume/location is marked "Not responding" in its own UI element only — the UI thread itself is never blocked for any part of the 2-second wait, since the wait happens entirely on that volume's isolated worker while the UI stays responsive throughout.
  - **Alternatives:** No timeout (Explorer's current behavior — can hang indefinitely on a bad SMB share); a longer default timeout.
  - **Why:** Real, current SMB/indexer regressions can turn a network directory listing into a multi-minute silent crawl; a short default timeout with a visible per-location status is the only way to guarantee the rest of the app stays instant regardless of what one bad network path is doing.
  - **Open question:** should a timed-out volume auto-retry in the background, or stay parked until the user manually retries? Not yet decided.

- **Decision:** The "Network" tree node is never eagerly enumerated — it stays collapsed and empty until the user explicitly expands it.
  - **Alternatives:** Eager background population on startup (a known cause of Explorer slowness when a stale network location, e.g. an old Dropbox/mapped-drive entry, is present).
  - **Why:** Removes an entire class of "why is my file manager slow on a fresh install" complaints at the source.

- **Decision:** Shell extensions and thumbnail generation run in a separate, watchdog-supervised worker process.
  - **Alternatives:** In-process shell-extension loading (Explorer's current model).
  - **Why:** A misbehaving third-party shell extension is a known cause of Explorer crashes and hangs today; isolating it means it can only take down its own worker, not the whole app.

- **Decision:** A minimal native context menu for common actions (copy, cut, paste, rename, delete, properties), with the full shell COM context menu available as an explicit "More options" fallback.
  - **Alternatives:** Always invoking the full shell context-menu COM chain, as Explorer does.
  - **Why:** Avoids the per-item COM menu construction that makes Explorer's right-click menu visibly render piece by piece, while preserving compatibility with third-party shell extensions for users who need them.

- **Decision:** Search indexes filenames and metadata only (name, path, size, dates, attributes), built via direct NTFS MFT reads and kept live via the USN Change Journal — not periodic re-scans. NTFS local volumes only by default; non-NTFS or network locations are opt-in with a much lower refresh cadence, never on the fast path.
  - **Alternatives:** Windows Search's model — indexer-scoped (only fast where explicitly configured), blends local results with web/online suggestions, indexes full file content by default.
  - **Why:** Windows Search's badness is a design problem, not a bug: scope-limited indexing means most real-world searches fall back to a slow live crawl, and blending local files with online suggestions actively buries the answer the user wanted. The Everything/WizTree model (MFT-based index, USN-journal-live, filename-first) is a proven, much faster, much more predictable alternative.
  - **Open question:** is full-text content search in scope for a future version, or an explicit non-goal? Not yet decided.

- **Decision:** Search results are ranked using fuzzy matching combined with frecency (frequency + recency of access), not plain substring filtering, and never blended with any web/online source.
  - **Alternatives:** Plain substring/prefix filtering (fast but dumb); pure match-quality ranking with no usage signal.
  - **Why:** Substring filtering fails on typos and partial recall; frecency surfaces the file the user actually meant even when several files match equally well on text alone.
  - **Open question:** should frecency-weighted ranking be the default, or should ranking be pure match-quality with recency available as a manual sort? Not yet decided.

## Constraints and budgets

These are proposed v1 targets, not yet measured. They must be validated against a real benchmark suite once the core engine exists, and revised if wrong.

- Cache-hit navigation → first paint: **≤ 16 ms** (one frame at 60 Hz).
- Cold enumeration → first visible rows painted, local NTFS SSD, directories up to 100k items: **≤ 250 ms**.
- Cold enumeration → full listing complete, 100k items, local NTFS SSD: **≤ 1 s** (tightened from an earlier 2 s draft now that HDD support is explicitly out of scope).
- Idle RSS: **≤ 80 MB** — this is the number that should alarm us if exceeded, since no content is loaded to justify it.
- Active-use RSS ceiling (large folders, thumbnails, multiple tabs cached): **≤ 512 MB**.
- Worker process/thread ceiling under load: 1 supervised shell-extension host process, plus a thumbnail-generation pool capped at `min(4, logical core count)` threads. Per-volume enumeration workers are separate from this cap and scale with the number of volumes actively being browsed or searched — they must not be artificially throttled low, since that would recreate the blocking problem this architecture exists to prevent.
- Network/non-local enumeration timeout: **2 seconds by default, user-configurable.** This timeout applies only to the affected volume's own worker; it must never be implemented in a way that blocks or delays the UI thread even during the countdown.

## Control flow

Folder navigation short-circuits on cache freshness: if a listing for the target folder is already cached, the UI renders immediately from cache and only then kicks off an async background re-enumeration to reconcile any changes, patching visible rows in place if anything differs. Only on a cache miss does the flow dispatch a fresh async enumeration and populate the virtual list as batches arrive. In both paths, the UI list only ever requests data for the currently visible row range — off-screen rows never trigger enumeration, painting, or thumbnail work. Every volume's enumeration runs on its own isolated worker; a timeout or hang on one volume changes only that volume's own status, and is never visible as a delay anywhere else in the UI. See `docs/charts/navigation-flow.md` for the full branching diagram, including both exits.

`fx-core`'s `ListingCache` is that early exit for directory entries. A hit returns the stored listing and re-enumerates on the same volume's worker, replacing the stored entries only when the finished listing differs. A miss stores the listing when enumeration finishes. Eviction is `TODO — define`; the cache is unbounded in this slice. The ≤16 ms cache-hit figure is a UI paint target and is not measured in the crate.

`list_shell` (`fx-core/src/bin/list_shell.rs`) is the first window. It shows one directory in a virtual `SysListView32` and calls `fx_core::shell` only. A cache hit sets the owner-data count and paints before the first `poll`. The control asks for a row's text when it paints that row. A `SysTreeView32` shows that directory as its root. Expanding a node lists only that directory's child folders, through the same `Shell`, and inserts a grandchild only when that child expands. A cache hit inserts those names before the first `poll`. A busy volume refuses the expand. Choosing a tree folder, activating a directory row, or entering a path in the address bar lists that directory. Back and Forward revisit directories this window already opened. A busy volume leaves the current rows in place and the timer retries the open. A path outside the current tree root replaces that root and leaves it collapsed. A timer drains `poll` without waiting, so a slow volume cannot stall the message loop. Closing the window cancels live listings. A painted row also gets a small icon from an image list cached by extension: one directory icon, and one icon per extension, loaded with file attributes so the file itself is not opened. A Network node sits beside that root. It is not enumerated at startup. Expanding it calls `Shell::browse_network`, which does not use the listing cache, and inserts only places already returned. A container expands the same way. A connectable share lists its UNC through the same navigation. The 2 second timeout is not retried automatically. Size and Date modified are DirectWrite cells for the rows the list view paints. The Name column stays native so the extension icon remains. A failed DirectWrite factory leaves those cells to the list view. A visible image row requests a thumbnail from a pool capped at `min(4, logical cores)`. The decode runs on that pool. The UI thread only polls and, once a buffer is ready, draws it over the icon. Navigation cancels the tickets for the directory being left. The thumbnail cache is unbounded; eviction is `TODO — define`. A context menu offers Copy, Cut, Paste, Rename, Delete, and Properties. Paste, delete, and rename run on a detached thread, and the list refreshes on success if the directory is unchanged. More options asks `menu_host`, a watchdog-supervised process. The helper returns no verbs and does not load a shell extension. The watchdog stops it after the existing 2 second network timeout of silence. An empty reply, a missing helper, or a stopped helper leaves the native commands in place. The property sheet is a modal dialog on the UI thread. Thumbnail pixels stay on the in-process pool. There is no C ABI on the listing path.

## Non-goals (what we will NOT do)

- HDD-optimized enumeration or seek-time workarounds — SSD-only is an explicit target.
- Cloud-sync placeholder files (Cloud Files API) in v1 — this is a large, separate integration surface.
- A scripting/plugin extensibility system (Directory Opus-style) before the core is proven stable.
- Cross-platform support — this is a Windows-only, native-API project by design.
- A fully custom GPU rendering pipeline for the whole UI — rejected in favor of native controls plus targeted owner-draw.
- Blending local search results with web/online suggestions, ever.

## Edge cases considered

- Network drives / UNC paths that hang or time out during enumeration — handled by per-volume isolation and the 2 s configurable timeout.
- Symbolic links and NTFS junctions (avoid infinite traversal loops).
- Paths exceeding legacy `MAX_PATH` (260 characters).
- Permission-denied folders (enumeration must fail gracefully per-item, not abort the whole listing).
- Removable media inserted or removed mid-browse.
- Files changing on disk during enumeration or between USN Journal reads (must not show stale or duplicate entries).
- A slow or non-NTFS drive attached alongside fast NTFS SSDs (must not degrade the fast drives' experience at all).

## Open design questions

- Should a timed-out network volume auto-retry in the background, or stay parked until manually retried?
- Is full-text content search an explicit non-goal, or a deferred future version?
- Should frecency-weighted ranking be the default for search, or should ranking be pure match-quality with recency as a manual sort option?

## Handoff protocol

> **Handoff protocol — every agent, every cycle:**
> 1. Read `docs/HANDOFF.md`, `docs/ARCHITECTURE.md`, and `CONTRIBUTING.md` (if present) before touching code.
> 2. Do the work described in the current **Prompt for Next Agent**.
> 3. Update every document and chart the work affected, and adjust `ROADMAP.md` status.
> 4. Move the completed prompt into the **Handoff Log** with an ISO 8601 **sign-off timestamp** and the model/agent that did it.
> 5. Write the next **Prompt for Next Agent**, leaving it as the only active prompt.

This protocol is the single source of coordination. It is stated here and in `docs/HANDOFF.md`; agents follow it without it being repeated inside each prompt.
