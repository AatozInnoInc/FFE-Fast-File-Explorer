# Windows File Explorer — Coding Agent Handoff

Status as of 2026-10-23. Owner: AatozInnoInc. Phase: `fx-core` enumerates directories, caches finished listings, and exposes the in-process `shell` module. `list_shell` shows a directory in a virtual `SysListView32`, with a `SysTreeView32`, a one-line address bar, Back/Forward, and a collapsed Network node. Choosing a tree folder, activating a directory row, entering a path, or choosing a connectable share lists that directory. Each painted row uses a small icon cached by extension, with one shared directory icon. Size and Date modified are DirectWrite cells; the Name column stays native so that icon remains. The Network node stays empty until the user expands it. Eviction of directory listings is `TODO — define`. The context menu is not started. SSD budgets are not measured. The UI/visual system exists as an HTML prototype. `theme-reference.html` is not in the repository.

---

## 1. Mission

Build a quality-first replacement for the Windows 11 File Explorer that is **solid, stable, and very fast** — in that order of priority. The owner explicitly rejected "fastest path" shortcuts (e.g., Dear ImGui / immediate-mode demos) in favor of durability, accessibility, predictable input, and resilience. The benchmark feeling is "Windows 7, where every directory opened instantly."

### Why Explorer is slow (the problems we exist to solve)
- Hybrid Win32 + WinUI 2/XAML Islands + WinUI 3 rendering layers add latency.
- Shell extensions inject into the process; a bad DLL can stall or crash Explorer.
- Context menus render item-by-item through the full COM chain.
- Thumbnail/indexing work competes with the UI thread.
- The **Network** node queries the network unprompted and stalls everything.
- A slow HDD or network share being enumerated slows unrelated drives and the UI.
- Windows Search is indexer-scoped, mixes in web/Bing results, indexes full content by default, and drifts stale.

### Non-goals
- Optimizing for HDDs (they must be tolerated and isolated, not made fast).
- Web/online results in search — never.
- Faking speed by blocking input or delaying first paint for animation.

---

## 2. Hard requirements (non-negotiable)

1. **Zero UI-thread I/O.** Enumeration, thumbnails, metadata, network calls, and timeouts never run on or block the UI thread. "Async" is not enough if it still shares blocking state with the UI.
2. **Per-volume isolation.** Every drive letter, UNC path, and the Network node has its own independent worker, cancellation token, and timeout budget. No two volumes share a worker or a blocking call. A hang on `\\slowserver\share` can only affect that location.
3. **Lazy Network node.** Collapsed and empty until the user expands it. No startup scan, no background population.
4. **Network timeout:** default **2 s**, user-configurable. On timeout, that location alone shows "Not responding — retry". Timeouts themselves must never block the UI.
5. **Streamed multi-volume results.** Cross-volume operations (e.g., search "This PC") stream per-volume; fast volumes never wait on slow ones.
6. **Smart local search** (see §5). Local filesystem results only.
7. **Accessibility and DPI correctness** are v1 requirements (UI Automation / Narrator, high-DPI, keyboard navigation). Reduced-motion must collapse all animation to instant.
8. **Crash isolation.** Shell extensions and thumbnail providers run out-of-process in a watchdog'd worker so third-party DLLs cannot take down the app. Crash telemetry / dump collection is a v1 feature.

---

## 3. Architecture

### Bounded contexts (DDD)
| Context | Responsibility |
|---|---|
| UI Shell | Windows, panes, tabs, navigation, input, theming, animation timing |
| Enumeration & Indexing | Directory enumeration, caching, MFT/USN index, search ranking |
| Owner-draw Rendering | Direct2D/DirectWrite drawing of cells, thumbnails, custom chrome |
| Shell Integration | Context menus, "Open with", properties, shell-extension fallback (isolated process) |
| Resilience & Telemetry | Watchdogs, crash dumps, timeouts, health signals |

Shared kernel: `FileEntry`, `Path`, `VolumeId`.

### Key decisions (with rationale)
- **Rust core engine, no UI dependencies.** Enumeration, caching, indexing live in a standalone, unit-tested Rust library for memory safety under concurrency. Fuzz anything that parses untrusted input (shortcuts, metadata, archive headers).
- **In-process shell call.** The Win32 shell calls `fx-core` in the same process. There is no IPC hop on the listing path. Owner decision 2026-10-19, for the cache-hit paint budget. Shell extensions and thumbnail providers stay out of process.
- **Native Win32 shell with Direct2D/DirectWrite.** Original decision: `SysListView32` (owner-data/virtual mode) and `SysTreeView32` for inherited accessibility/DPI/keyboard behavior, with Direct2D owner-draw via `NM_CUSTOMDRAW` only where it earns its keep. Later visual work (§6) established that custom-drawn chrome and GPU-friendly effects are acceptable where they serve the product — **but the testable core/UI boundary must remain.** See open question Q1 on how far custom drawing goes.
- **Enumeration:** `FindFirstFileEx` with `FIND_FIRST_EX_LARGE_FETCH` for directory listing (not `IShellFolder`/COM). MFT reads + USN Change Journal for whole-volume indexing.
- **Virtualization:** only viewport rows are materialized; data fetched in batches ("chunky, not chatty"); results stream asynchronously.
- **Icons/thumbnails:** cache icons by type/extension, not per file; thumbnails on a background pool capped at `min(4, cores)`, lazy-loaded for visible rows only. Volume enumeration workers are NOT capped low (capping would recreate blocking).
- **Context menu:** native minimal menu (copy, cut, paste, rename, delete, open with, properties) with the real shell menu as an opt-in "More options" fallback, invoked out-of-process.
- **Rendering:** Direct3D/Direct2D rather than OpenGL (Windows-only product).

### Performance and memory budgets (proposed targets — must be measured, not assumed)
| Metric | Target |
|---|---|
| Cache-hit navigation paint | ≤ 16 ms |
| Cold enumeration first paint | ≤ 250 ms |
| Full listing, 100k items (SSD) | ≤ 1 s |
| Idle RSS (leak tripwire) | ≤ 80 MB |
| Active-load ceiling (large folders, thumbnails, tabs) | ≤ 512 MB |
| Network timeout | 2 s default, configurable |

The split memory budget is deliberate: idle must stay low so leaks are detectable; active use may be generous for aggressive caching. Primary hardware target: SSD systems.

### Navigation control flow
Cache hit is an explicit early exit (paint from cache, revalidate in background). Cache miss goes to that volume's worker, streams first batch for first paint, then fills. `list_shell` is that path. Its tree lists a folder's direct child directories only after that node expands. Tree selection, the address bar, Back, and Forward open a directory the same way. A busy volume leaves the current list in place and retries from the timer. The Network node is a separate browse: it stays collapsed until expanded, streams places through `Shell::browse_network`, and does not use the listing cache. A connectable share then lists through the same navigation. A timeout is not retried automatically. Size and Date modified are painted with DirectWrite for the cells the list view asks to draw. The Name column stays with the control. A failed DirectWrite factory leaves those cells to the list view's own text. There are no thumbnails yet.

---

## 4. Tooling and conventions

- Rust: `rustfmt` + `clippy`, CI-gated.
- C++: `.clang-format` + static analysis in CI (base style not yet confirmed by owner).
- Tests on every change; static analysis before merge; fuzzing of parsers.
- UI-test tooling intentionally deferred (WinAppDriver was mentioned as a candidate).
- Design changes require owner sign-off. **Do not decide architecture, budgets, or UX unilaterally** — the owner has corrected this twice. Present options and tradeoffs; mark unknowns `TODO — define` rather than inventing values.
- Design package follows `AatozInnoInc/principal-engineer-skills` → `skills/autonomous-agentic-design-workflow/` templates: README, ARCHITECTURE, `docs/charts/` (context-map, navigation-flow), ROADMAP, CONTRIBUTING, HANDOFF. Use those templates exactly.
- License: undecided.

### Model selection

Different tasks call for different reasoning effort. Maximum reasoning on mechanical work wastes budget and time. Low reasoning on architecture decisions produces poor results. Select by role and effort. Do not lock this workflow to one vendor.

Claude names in the original guide are effort-class examples only: frontier like `claude-opus` at maximum, mid-level like `claude-sonnet` at high, flash like `claude-haiku` at low. This workflow uses Grok.

| Role | Effort | Current selection | Typical tasks |
|---|---|---|---|
| Frontier — design and review | Maximum reasoning | `grok-4.7-xhigh` | Architecture decisions, DDD modeling, domain-layer design, and review of any proposal that touches bounded contexts or the enumeration contract before work continues |
| Mid-level — implementation | High reasoning | `grok-4.7-high` | Feature coding and tests that follow an already reviewed design. Step up to maximum reasoning only when the coding is hard and the design is unchanged |
| Flash — mechanical | Low reasoning | `grok-4.7-low` | Refactoring, renaming, documentation edits, boilerplate scaffolding |

Rules:

1. Read `docs/HANDOFF.md` and `docs/ARCHITECTURE.md` fully before writing any code.
2. Every handoff updates this file, moves the finished prompt into the Handoff Log with an ISO 8601 sign-off timestamp and the model that did the work, and leaves exactly one **Prompt for Next Agent**.
3. Any change to the domain model, bounded contexts, or the enumeration contract requires frontier review before proceeding. That contract is the shared kernel (`FileEntry`, `Path`, `VolumeId`) plus per-volume workers, cancellation, and the network timeout.
4. Do not lock the workflow to one vendor. The role and the reasoning effort are what matter.

---

## 5. Smart search design (v2)

1. Index filenames + metadata only by default (name, path, size, dates, attributes). No content indexing by default.
2. Build index by direct NTFS MFT reads (Everything/WizTree model).
3. Keep live via USN Change Journal — no periodic rebuilds.
4. NTFS local volumes only by default; network/non-NTFS opt-in, never degrading the local fast path.
5. Never blend web/online results.
6. Rank with fuzzy matching (fzf-style subsequence scoring) plus frecency.

---

## 6. UI/UX and visual system

### Themes
Required built-in themes: Windows Classic Dark, Windows Classic Light, macOS Dark, macOS Light, Synthwave, Modern.

### Owner's taste (apply everywhere)
- macOS-inspired, **no Liquid Glass**. Rounded continuous-curvature (superellipse/squircle) geometry, concentric nested radii (inner = outer − padding), stacked 3–6 layer soft shadows, tasteful opacity/translucency.
- Rich SVGs and visuals used selectively; restrained game-UI influence. Too game-like = cheesy; too macOS = bland.
- Rejects: jagged edges, 1px bevel architecture, icon overload, effects that cost performance.

### Modern theme — decided
- **Halo is the lead Modern direction**: no chrome, segmented-control breadcrumb, vibrancy inspector panel, selection is a halo of tinted light rather than a filled bar. Hierarchy via tint + blur + shadow. No blue.
- **Copper is the default palette.** Switchable palettes: Ember, Jade, Orchid, Copper, Sulfur, Rose, Mono, Moss.
- Halo palettes are driven by 8 tokens only: accent, glow, neutral tint, two base stops, three text steps (~40-line theme file; community themes easy; equal render cost across palettes).
- **Keep for comparison / possible additional themes:** G · Lozenge (tabbed squircle capsules, card panes, 3-layer shadows), D · Deck & Bezel (machine/cassette-futurism bezel), E · Strata (perspective planes, Z-lift on hover/select), I · Vellum (stacked sheets; proposed Modern Light).
- **Dropped:** F · Scheme 9 (too jagged). Earlier A/B/C round rejected as "just colors" (B and C palettes liked).

### Animation spec (approved; all 29 implemented in HTML demo)
Global rules: every duration is a token × one global speed scale; max 220 ms except theme switch; all interruptible; reduced-motion = instant; **nothing animates on the enumeration/first-paint path**.

- Selection/hover: row hover 90 ms in / 120 ms out; selection halo bloom 140 ms; range-select stagger 12 ms/row capped at 8 rows; keyboard focus ring 100 ms; sidebar active crossfade 120 ms; checkbox/toggle 130 ms.
- Navigation: directory crossfade 110 ms (outgoing only; incoming paints immediately); breadcrumb slide 160 ms; back/forward 8 px hint 140 ms; tree expand 150 ms + chevron 90°; pane/inspector 180 ms (content fade at 60 ms delay); tab open/close/reorder 150 ms.
- Data: progress bar eased continuous; index scan shimmer 1.4 s low-amplitude loop; throughput digit roll 200 ms monospace; row insert/remove 120 ms; search reorder 130 ms (visible rows only, FLIP).
- Chrome: window open/close 160 ms scale 0.98→1; focus/blur 150 ms; menus 120 ms 4 px rise; tooltip 90 ms in after 400 ms delay, 60 ms out; dialog 170 ms with backdrop blur; snap/resize none.
- Feedback: denied shake 180 ms, 3 px, twice; toast 150 ms in, 200 ms fade; copy/paste pulse 200 ms; drag ghost follows pointer with no easing, target glow 120 ms, drop settle 140 ms.
- Theme: palette and light/dark switch 250 ms crossfade.
- **Never animated:** scrolling (1:1, no smooth-scroll easing), column resize, caret, window drag, anything on the critical path to first paint.

Implementation mapping: the HTML demo uses a `--sp` scale variable, Web Animations API for one-shots, CSS transitions for state, FLIP for reorders — mirror this as a token-based timing layer in Direct2D.

---

## 7. Roadmap

- **v1 — Foundation:** Rust enumeration core (standalone, tested); per-volume isolated workers + cancellation; 2 s configurable network timeout; lazy Network node; virtualized list/tree shell; icon cache + async thumbnails; minimal native context menu with isolated shell fallback; crash isolation + telemetry; theming engine with tokens.
- **v1.5 / v2:** MFT/USN index; smart ranked search (fuzzy + frecency, local only); UI test automation.
- **Later / undecided:** full-text content search; network volumes in search scope.

---

## 8. Open questions (ask the owner; do not decide)

1. **Window silhouette / custom chrome:** Do themes own the window frame (non-rectangular, opt out of Win11 auto-rounding, own border/shadow/hit-testing), or do tabs live inside a conventional frame? How far does custom drawing replace native controls, given accessibility requirements?
2. Timed-out volumes: auto-retry in background, or stay parked until manual retry?
3. Full-text content search: deferred v3 item or explicit non-goal?
4. Frecency as default ranking vs. pure match quality with recency as a sort option?
5. C++ `clang-format` base style; license.
6. Final decision on Lozenge / Deck / Strata / Vellum: separate themes vs. borrowed interactions (e.g., Strata Z-lift, Halo selection inside Lozenge).
7. **Resolved 2026-10-19.** The shell calls `fx-core` in-process. Enumeration and the listing cache do not cross a process boundary. A C ABI is not designed.
8. **Resolved 2026-10-19.** Model roles are recorded under **Model selection** in §4. Select by role and reasoning effort. This workflow uses Grok.

---

## 9. Artifacts and where they are

- Design package is in this repository (`AatozInnoInc/FFE-Fast-File-Explorer`) as of 2026-10-19. It was drafted in Perplexity sessions on 2026-08-31 and 2026-09-22. Files: `README.md`, `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/HANDOFF.md`, `docs/charts/context-map.md`, `docs/charts/navigation-flow.md`, `docs/theme-motion-spec.md`.
- Enumeration crate: `fx-core/`. Windows directory reads use `FindFirstFileExW`. Portable tests use a scripted source. SSD budget numbers are not measured yet. Run `fx-core/examples/measure_listing.rs` on a Windows SSD when a real directory is available.
- List window: `fx-core/src/bin/list_shell.rs`. A virtual `SysListView32`, a `SysTreeView32` of child folders filled on expand, a one-line address bar, Back/Forward, and a collapsed Network node. Calls `fx_core::shell` only. Windows only. Choosing a folder or a connectable share lists it. Small icons are cached by extension. Columns are Name, Size, and Date modified. Size and Date modified are DirectWrite; Name stays native so the icon remains. The Network node is not browsed until it expands.
- Interactive HTML preview: Halo (8 palettes, Copper default), Lozenge, Deck, Strata, Vellum, with all 29 animations, a demo dock, reduced-motion toggle, and 0.25×–3× speed slider. `theme-reference.html` is still absent from the repository. The written spec is `docs/theme-motion-spec.md`.
- Workflow templates: `AatozInnoInc/principal-engineer-skills/skills/autonomous-agentic-design-workflow/`.

---

## 10. Handoff Log

### 2026-10-19T14:20:27Z — Design package in repository; model roles recorded

**Signed off by:** Grok 4.7, design/review pass.

**Completed prompt (partial).** The previous prompt asked to create a repository, lay out the design package, and then implement `fx-core`. The repository and design package are done. `fx-core` was not started. The reviewed brief for it is the current **Prompt for Next Agent**. Roadmap items for the crate stay `planned`.

**What landed:**

- `README.md`, `CONTRIBUTING.md`
- `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/HANDOFF.md`
- `docs/charts/context-map.md`, `docs/charts/navigation-flow.md`
- `docs/theme-motion-spec.md` (`theme-reference.html` is still absent)

**This pass.** Recorded model roles under §4. Closed open question 8. Refreshed the repository status in this file. Wrote the `fx-core` implementation brief. Did not write the crate. Questions 1–7 remain open.

### 2026-10-19T14:42:32Z — fx-core enumeration

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** Built `fx-core` with `FileEntry`, `Path`, and `VolumeId`. One detached worker and one cancellation token per volume. A second listing for a busy volume returns `VolumeBusy` and does not start another worker. Network volumes use a configurable timeout, default 2 seconds. Local volumes have no deadline. `Listing::try_recv` never blocks, including when a Windows call cannot be preempted. Production reads are `FindFirstFileExW` with `FindExInfoBasic` and `FIND_FIRST_EX_LARGE_FETCH` (`fx-core/src/windows.rs`). Tests use `ScriptedSource` and cover cancellation isolation, timeout isolation, local listings ignoring the network timeout, and a first batch before the source is exhausted. CI runs `rustfmt`, `clippy -D warnings`, and `cargo test` on Ubuntu, and clippy plus tests on Windows. No UI and no FFI. Questions 1–7 were not decided. The listing cache was not added. The ≤250 ms and ≤1 s SSD budgets were not measured; `cargo run --example measure_listing -- <directory>` prints them on Windows and does not assert them.

### 2026-10-19T15:24:03Z — Listing cache

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** Added `ListingCache` in `fx-core`. A hit returns the stored entries without waiting, then re-enumerates on that volume's existing worker and replaces the stored entries only when the finished listing differs. A miss streams batches and stores the listing when it finishes. A timed-out or cancelled revalidation leaves the previous entries in place. A slow volume does not delay a cache hit on another volume. Eviction is `TODO — define`; the cache is unbounded. Tests use `ScriptedSource`. The ≤16 ms cache-hit paint budget was not claimed. No UI and no FFI. Questions 1–7 were not decided.

### 2026-10-19T18:31:28Z — Shell calls fx-core in-process

**Signed off by:** Grok 4.7, design/review pass. **Owner decision.**

**Completed prompt.** Asked whether the shell calls `fx-core` in-process or across a process boundary. The owner chose in-process. Recorded in §3, §8 question 7, `docs/ARCHITECTURE.md`, and `docs/ROADMAP.md`. No IPC hop on the listing path. Shell extensions and thumbnail providers stay out of process. No UI was built. No C ABI was designed. Questions 1–6 stay open. Eviction and the SSD budgets were not invented.

### 2026-10-19T18:46:22Z — In-process shell module

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** Added `fx_core::shell`. `Shell::open` returns cached entries immediately when they exist, plus the live listing. `poll` is `try_recv` and does not block. `cancel` reaches that listing's token. The module does not expose `ScriptedSource`, the volume registry, or Windows handles. Enumeration and cache behavior are unchanged. Tests use the scripted source inside the crate. No IPC, no C ABI, and no UI. Questions 1–6 stay open. Eviction and the SSD budgets were not invented.

### 2026-10-19T19:51:56Z — Win32 virtual list

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** Added `list_shell`, a Win32 window with one `SysListView32` in virtual/owner-data mode for one directory. It fills that list from `Shell::open` and `poll`. A cache hit sets the owner-data count and paints before the first `poll`. A miss appends batches. A revalidation replaces rows only when the finished listing differs; timeout, cancel, and failure keep the rows already shown. `LVN_GETDISPINFO` writes the name of the row the control asks for. The message loop is `GetMessageW`. `poll` runs from a timer and does not wait, so a slow volume does not stall the window. Closing the window cancels the live listing. The UI thread does not enumerate. No tree, address bar, icons, context menu, IPC, or C ABI. Questions 1–6 stay open. Eviction and the SSD budgets were not invented.

### 2026-10-21T05:13:23Z — Folder tree beside the list

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** Added a `SysTreeView32` beside the list in `list_shell`. The open directory is the root and starts collapsed. Expanding a node calls `Shell::open` and `poll` for that directory only. Child items are directories already returned; grandchildren are not inserted until those nodes expand. A cache hit inserts those names before the first `poll`. A miss inserts them as batches arrive. Timeout, cancel, and failure keep the names already shown. The same `Shell` serves the list and the tree, so a finished list is a cache hit for that directory. A busy volume refuses the expand and does not block the message loop. There is no Network neighborhood. Selecting a tree folder does not change the list. No address bar, icons, context menu, IPC, or C ABI. Questions 1–6 stay open. Eviction and the SSD budgets were not invented.

### 2026-10-22T17:13:57Z — Address bar, Back, and Forward

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** `list_shell` now lists the directory the user chooses. Choosing a tree folder, activating a directory row (double-click or Enter), or pressing Enter in the one-line address bar calls `Shell::open` for that path. Back and Forward, including Alt+Left and Alt+Right, revisit directories this window already opened. A cache hit sets the owner-data count and paints before `poll`. A busy volume does not clear the list and does not block the message loop; the timer retries the open. A path outside the tree root replaces that root, collapsed, without enumerating it. A path inside the tree selects the matching node and does not walk unloaded descendants. The UI thread does not enumerate. No icons, context menu, Network neighborhood, IPC, or C ABI. Questions 1–6 stay open. Eviction and the SSD budgets were not invented.

### 2026-10-22T17:21:02Z — One icon per extension

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** The list's small image list stores one icon per file extension and one icon for every directory. `SHGetFileInfoW` is called with `SHGFI_USEFILEATTRIBUTES`, so the lookup does not open that file. The call runs once per key, and only when a painted row asks for the icon. Directories share one folder icon. A name with no extension, a leading dot, or a trailing dot uses the generic file icon. `archive.tar.gz` uses `gz`. Failed lookups are remembered too, so a miss is not repeated per row. No thumbnails, context menu, Network node, IPC, or C ABI. The UI thread does not enumerate. Questions 1–6 stay open. Directory-listing eviction and the SSD budgets were not invented.

### 2026-10-22T17:28:46Z — Lazy Network node

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** `list_shell` shows a Network node beside the opened directory. It is collapsed and empty at startup. Expanding it calls `Shell::browse_network` and inserts places as batches arrive. A container expands through another browse. A connectable share with a UNC lists that path through the existing navigation. The browse uses the existing 2 second timeout. Timeout, cancel, and failure keep the places already shown and are not retried. Results are not stored in the listing cache. The UI thread does not call WNet and does not enumerate. A busy parent refuses the expand. Replacing the folder-tree root leaves a new collapsed Network node and does not browse it. No thumbnails, context menu, IPC, or C ABI. Questions 1–6 stay open. Directory-listing eviction and the SSD budgets were not invented.

### 2026-10-23T14:12:22Z — Owner-draw Size and Date modified

**Signed off by:** Grok 4.7, implementation pass.

**Completed prompt.** The virtual list has Name, Size, and Date modified. `LVN_GETDISPINFO` still supplies the text for every column the control asks for. Name stays a native column so the extension icon remains. Size and Date modified are DirectWrite cells during `NM_CUSTOMDRAW`, and only for the rows the list view paints. A directory's size is blank. Date modified is UTC `YYYY-MM-DD HH:MM`. If the DirectWrite factory cannot be created, or a cell draw fails, that cell falls back to the list view's own text. The UI thread does not enumerate, and a slow volume still does not stall the window. No thumbnails, context menu, IPC, or C ABI. Questions 1–6 stay open. Directory-listing eviction and the SSD budgets were not invented. The window was not launched from this environment.

---

## 11. Prompt for Next Agent

> `list_shell` lists a chosen directory through `fx_core::shell`. Name stays a native column with the extension icon. Size and Date modified are DirectWrite owner-draw for the rows the list view paints. A failed DirectWrite factory leaves those cells to the list view. The Network node is browsed only after the user expands it. Do not add IPC or a C ABI. Do not bypass `shell` with `ListingCache` or `enumerate`.
>
> Add async thumbnail generation on a background worker pool capped at `min(4, logical cores)`. Generate thumbnails for visible rows only. The UI thread never decodes an image and never enumerates. Cancel a thumbnail when the user navigates away from that file. The thumbnail cache stays unbounded; eviction is `TODO — define`. A slow volume still must not stall the window.
>
> Do not build a context menu. Do not decide questions 1–6. Do not invent an eviction size. SSD budgets are still unmeasured; do not invent those numbers. Reduced motion and the theme system are out of this slice.
