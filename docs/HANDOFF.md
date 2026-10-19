# Windows File Explorer — Coding Agent Handoff

Status as of 2026-10-19. Owner: AatozInnoInc. Phase: design package is in this repository; the core engine (`fx-core`) has not been started. The UI/visual system exists as an HTML prototype. `theme-reference.html` is not in the repository.

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
Cache hit is an explicit early exit (paint from cache, revalidate in background). Cache miss goes to that volume's worker, streams first batch for first paint, then fills.

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
7. FFI boundary between Rust core and C++ shell (shape not yet designed).
8. **Resolved 2026-10-19.** Model roles are recorded under **Model selection** in §4. Select by role and reasoning effort. This workflow uses Grok.

---

## 9. Artifacts and where they are

- Design package is in this repository (`AatozInnoInc/FFE-Fast-File-Explorer`) as of 2026-10-19. It was drafted in Perplexity sessions on 2026-08-31 and 2026-09-22. Files: `README.md`, `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/HANDOFF.md`, `docs/charts/context-map.md`, `docs/charts/navigation-flow.md`, `docs/theme-motion-spec.md`.
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

---

## 11. Prompt for Next Agent

> This prompt is implementation work for the mid-level tier. The enumeration contract below is already reviewed. Do not redesign it, and do not create a new repository. The design package is already in this repo.
>
> Build a standalone Rust crate `fx-core` with no UI dependencies and no FFI. Open question 7 (the Rust/C++ boundary) stays open — do not design or build it.
>
> Shared-kernel types: `FileEntry`, `Path`, and `VolumeId`.
>
> Enumeration contract:
> - One worker, one cancellation token, and one timeout budget per `VolumeId`. No two volumes share a worker or a blocking call.
> - Network and non-local paths use a configurable timeout, default 2 seconds. Do not invent a local-SSD timeout, auto-retry behavior, or a search-ranking policy.
> - Stream `FileEntry` batches over a channel.
> - Production directory reads on Windows use `FindFirstFileExW` with `FindExInfoBasic` and `FIND_FIRST_EX_LARGE_FETCH`.
> - A small enumerator seam is part of this design: production Windows code calls that API; tests use a scripted source that can delay, hang, and yield batches. The seam exists so isolation tests run on non-Windows CI. It is not a second enumeration design.
>
> Tests must show that (a) cancelling or timing out one volume never delays another, and (b) the first batch arrives before enumeration completes.
>
> The 100k SSD budgets (≤ 1 s full listing, ≤ 250 ms first batch) are measured only on Windows against a real directory. Report those numbers when they are measured. Do not publish pipeline-only or Linux timings as those budgets.
>
> Set up CI with `rustfmt` (no custom `rustfmt.toml`), `clippy` with warnings denied, and the portable tests.
>
> Do not build any UI. Before deciding anything still open in §8 (questions 1–7), stop and ask the owner.
