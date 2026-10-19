# Rapid Explorer — Theme & Motion Implementation Spec

**Audience:** the implementing agent / engineer.
**Status:** visual direction approved by the product owner; several engineering decisions are still open (see [Open decisions](#open-decisions--do-not-resolve-unilaterally)). Do not resolve those on your own — surface the trade-off and get sign-off.
**Companion file:** `theme-reference.html` (same folder). Open it in any Chromium browser. It is the **source of truth for look and feel**. Where this document and the reference disagree, this document wins for numbers it states explicitly and the reference wins for everything visual it does not cover. Known discrepancies are called out inline.

---

## 1. What you are building

Five themes for a native Windows file explorer, plus a shared motion system.

| ID | Name | Role | Ship status |
|---|---|---|---|
| **H** | **Halo** | **The "Modern" theme. Lead direction.** 8 palettes, **Copper is the default.** | Must ship |
| G | Lozenge | Alternate Modern: tabbed squircle chrome, card panes | Keep — ship status open |
| I | Vellum | Light theme (stacked sheets) | Keep — ship status open |
| D | Deck & Bezel | Cassette-futurism hardware shell | Keep — ship status open |
| E | Strata | 2.5D spatial / perspective | Keep — ship status open |
| ~~F~~ | ~~Scheme 9~~ | **Dropped. Do not implement.** Rejected as "too jagged". | — |

The product owner's taste, in their words and decisions, which should guide every call this doc doesn't make:

- Likes macOS in general, **without** Liquid Glass. Likes game UIs (dark) but "too game-like is cheesy, too macOS is bland."
- Wants **rounded, continuous-curvature corners and tasteful layered shadows and opacity** — explicitly dislikes jagged / hard-bevel edges (why F was dropped).
- Likes rich SVG visuals and animation, **but fast animations, and only certain ones**.
- Quality and stability over shortcuts. The app must feel like "Windows 7 where every directory opened INSTANTLY."

### Non-negotiable product constraints that themes must not break

These come from the locked architecture and apply to every theme:

- **No UI-thread I/O. Ever.** Theming and animation code must never wait on enumeration, thumbnails, or network.
- **No animation on the first-paint path.** A directory's contents paint on the first frame they're available. An animation may decorate that; it may never delay it.
- Memory budget: **≤ 80 MB idle RSS, ≤ 512 MB under active load.** Theme assets (brushes, geometry caches, blur surfaces) count against this.
- Cold enumeration ≤ 1 s at 100k items. Themes must stay cheap at 100k rows (virtualized; only visible rows are ever drawn).
- Rendering stack already chosen: native Win32 virtual/owner-data `SysListView32` / `SysTreeView32`, **Direct2D + DirectWrite** for owner-drawn cells, Rust core for enumeration/MFT/USN.

---

## 2. Shared layout contract

All five themes render the same information architecture. Theme ≠ layout. A theme may change silhouette, materials, geometry, and motion, but not what is on screen.

```
┌ window chrome (theme-owned: tabs / traffic lights / bezel) ───────────────┐
│ tabs row                                                    [+ new tab]  │
│ [‹][›]  breadcrumb (segmented)            [search]  [actions]  [◧] [···] │
├──────────────┬───────────────────────────────────────────┬───────────────┤
│ Sidebar      │ File list (virtualized)                   │ Inspector     │
│  Pinned      │  icon │ Name │ Size │ Modified │ Kind     │  preview      │
│  Volumes     │                                           │  Selected kv  │
│   C: NVMe    │                                           │  Transfer     │
│   D: NVMe    │                                           │  Enumeration  │
│   E: HDD (dim│                                           │               │
│   Network    │  (parked / lazy, never blocks)            │               │
├──────────────┴───────────────────────────────────────────┴───────────────┤
│ status: N items · N selected · idle rss · first paint · net timeout 2.0 s │
└───────────────────────────────────────────────────────────────────────────┘
```

Reference column widths at 1180 px: sidebar 206–216, inspector 256–268, list = remainder. Row height 31–35 px depending on theme (tokens below). List columns: `30px | 1fr | 92px | 112px | 84px`, numerics right-aligned in monospace.

State the sidebar must represent: HDD volumes and the Network node render **dimmed** (they are reachable but isolated / lazy), never hidden.

---

## 3. Theme engine model

### 3.1 Token-first, no literal colours in components

Halo was rebuilt so that **every colour in the theme derives from 8 tokens**. Generalize that to the engine: components reference semantic tokens only; a theme file supplies values. This is what makes the palette switcher trivial and keeps community themes small.

**Halo token schema (the reference model):**

| Token | Type | Meaning |
|---|---|---|
| `ac` | RGB triplet | Accent: selection, active segment, progress, search text, focus ring |
| `gl` | RGB triplet | Glow tint: ambient radial light in window background, progress start |
| `nt` | RGB triplet | Neutral tint: all panels, fields, hover fills, hairline highlights (used at low alpha) |
| `b1`, `b2` | colour | Window base gradient top → bottom |
| `tx`, `tx2`, `tx3` | colour | Text primary / secondary / tertiary (labels, disabled) |

Components use these at fixed alphas, e.g. hover = `nt @ 0.06`, panel = `nt @ 0.10 → 0.045` gradient, selection = `ac @ 0.17 → 0.07` + `ac @ 0.20` 1 px ring + `ac @ 0.42` bloom. The alphas are part of the **theme**, not the palette — palettes only swap the 8 tokens.

**Proposed theme file shape** (format is an open decision — TOML vs JSON vs other; this is illustrative only):

```toml
[theme]
id = "halo"
name = "Halo"
default_palette = "copper"

[geometry]       # see §4
window_radius = 26
window_k = 3.0
panel_radius = 16
row_radius = 12
control_radius = 11
k = 2.4

[elevation]      # see §5
window = ["0 5 10 0.30", "0 20 44 -14 0.55", "0 46 90 -30 0.62"]

[palette.copper]
ac = [255,160,122]; gl = [230,120,80]; nt = [245,205,185]
b1 = "#16110e"; b2 = "#0b0807"
tx = "#f4ebe5"; tx2 = "#ab9a90"; tx3 = "#7d6f66"
```

### 3.2 Runtime switching

- Palette and theme switches are **live** (no restart), via a 250 ms crossfade (motion #28/#29).
- Persist the chosen theme + palette per user.
- Rebuild only device-dependent resources on switch (brushes, cached geometries); never re-enumerate or re-query the filesystem.

---

## 4. Geometry: continuous-curvature corners

This is the single most important visual rule after the palette, and the main reason F was rejected.

### 4.1 Use superellipse corners, not circular arcs

A circular `border-radius` joins a straight edge to an arc with an abrupt curvature change; Apple's corners are superellipses with continuous curvature, so there's no visible seam ([App Screen Studio](https://www.appscreenstudio.com/en/blog/understanding-ios-squircle-continuous-curvature), [squircle math](https://squircle.js.org/blog/math-behind-squircles)).

The reference uses CSS `corner-shape: superellipse(K)` with `border-radius` as the size ([Smashing Magazine](https://www.smashingmagazine.com/2026/03/beyond-border-radius-css-corner-shape-property-ui/)). In CSS, `K = 1` is round, `K = 2` is the squircle, `K = 0` is bevel, `∞` is square. **Treat the curve exponent as `n = 2^K`** (K=1 → n=2 circle, K=2 → n=4 squircle, K=0 → n=1 straight bevel). Note: one secondary source states the exponent as `2K`; that formula gives the wrong result for bevel, so use `2^K` and tune visually against the reference.

| Element | Radius (px @96 DPI) | K | n ≈ |
|---|---|---|---|
| Halo window | 26 | 3.0 | 8 |
| Vellum outer desk | 22 | 2.8 | 7 |
| Lozenge window body | 20 | 2.6 | 6 |
| Panels / glass / sheets | 14–16 | 2.4–2.6 | 5.3–6 |
| Controls, search, pills | 10–12 | 2.3–2.5 | 5–5.7 |
| List rows, sidebar items | 10–12 | 2.2–2.4 | 4.6–5.3 |
| Progress bars, dots | full pill | 1 (round) | 2 |

**Native implementation (Direct2D):** build each corner as an `ID2D1PathGeometry`. For a corner of radius `r` and exponent `n`, sample the quarter-curve `x = r·(1 − |cos t|^(2/n))`, `y = r·(1 − |sin t|^(2/n))`, `t ∈ [0, π/2]`, then either (a) fit 2–3 cubic Béziers per corner, or (b) emit 12–16 line segments per corner. Cache geometries keyed by `(w, h, r, n, dpi)`; rows share one geometry per row-height. Snap to device pixels at the straight edges only. Never rebuild per frame.

### 4.2 Concentric radii

Nested rounded shapes must share a centre: **inner radius = outer radius − padding**. Equal radii on parent and child cause "corner crowding" ([Timeyii](https://timeyii.substack.com/p/the-architecture-of-the-curve-why), [SwiftUI concentricity](https://www.createwithswift.com/exploring-concentricity-in-swiftui/)). Enforce in the layout system, not by hand: a container exposes its inner radius to children. Clamp to a minimum (≈ 6 px) so deep nesting never goes square.

### 4.3 No hairline borders in Halo

Halo uses **no borders**. Separation is surface tint, a 1 px top inner highlight (`nt @ 0.10–0.15`), and layered shadow. Lozenge and Vellum allow a single hairline (`rgba(255,255,255,.07)` / `rgba(42,38,34,.09)`) between regions only.

---

## 5. Elevation: layered shadows

A single shadow reads as an artificial outline; stack 2–3 (up to 6) with exponentially growing blur and falling opacity ([Shadow Studio](https://rocketdevtools.com/utilities/shadow-studio/)). In dark themes, carry most elevation through **surface lightness**, since shadows are nearly invisible on dark surfaces ([UI/UX Atlas](https://uiuxatlas.com/lessons/visual-design/depth-elevation-and-layering/)).

Values are `offsetY blur spread alpha` on black:

| Level | Use | Layers |
|---|---|---|
| e1 | rows, sub-cards, buttons | `0 1 2 0 .28` |
| e2 | panels, cards, active tab | `0 2 4 0 .22` + `0 6 14 -4 .34` |
| e3 | window (Lozenge) | `0 4 8 0 .24` + `0 14 32 -10 .46` + `0 28 60 -22 .50` |
| e3h | window (Halo) | `0 5 10 0 .30` + `0 20 44 -14 .55` + `0 46 90 -30 .62` |
| glow | selection bloom (Halo) | `0 10 30 -12 ac@.42` |
| light | Vellum sheet | `0 1 2 rgba(60,50,38,.14)` + `0 8 20 -6 …24` + `0 22 44 -18 …24` (warm brown, never black) |

**Native implementation:** Direct2D's Shadow effect is built on the Gaussian blur effect (`CLSID_D2D1GaussianBlur`, blur radius = 3 × standard deviation, in DIPs) ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/direct2d/gaussian-blur)). Do **not** run blur effects per row per frame. Pre-render shadow nine-slices per `(radius, n, level, dpi)` into cached bitmaps and stretch the centre. For the window-level shadow, see §8 (DWM may supply it).

---

## 6. Themes

All colours are sRGB. Fonts: UI = `Segoe UI Variable` (fallback Segoe UI); mono = `Cascadia Mono` (reference uses IBM Plex Mono first — **font choice for shipping is open**, see §11); Deck's condensed display face = `Oxanium` / `Chakra Petch` (needs licensing check / bundling).

### 6.1 H · Halo — the Modern theme (lead)

**Concept:** no chrome at all. Light and material do the work. The breadcrumb is a segmented control, the inspector is a vibrancy panel, and selection is a **soft halo of tinted light** (gradient fill + faint ring + outer bloom), not a filled bar.

**Structure**
- Window: radius 26, K 3. Background = `b1→b2` vertical gradient, plus two ambient radials: `gl @ .17` at top-left (90%×70% at 18%,−8%) and `ac @ .12` at bottom-right (70%×60% at 92%,108%). 1 px top inner highlight `nt @ .13`. Elevation e3h.
- Traffic lights: 11 px circles, `nt @ .14` fill, `nt @ .13` ring (neutral, not red/yellow/green).
- Tabs: pill 12 px radius, `nt @ .06`; active = `ac @ .13` + `ac @ .18` ring + `ac @ .40` glow.
- Breadcrumb: segmented control in a `nt @ .07` track (radius 13, 3 px inset). Active segment = solid accent gradient `ac → ac @ .72`, text `#14100a`, e1 + `ac @ .5` glow.
- Search field: radius 12, `nt @ .07`, typed text in accent.
- Sidebar items: radius 12, padding 9×12. Hover `nt @ .07`. Selected = `ac @ .13` fill + `ac @ .18` ring + `ac @ .38` 26 px bloom; icon stroke turns accent. Disabled (HDD/Network) = `tx3`, opacity .62.
- Rows: 35 px, radius 12, padding 0×14. Hover `nt @ .06`. Selected = `ac @ .17 → .07` vertical gradient + `ac @ .20` ring + e1 + glow; numerics turn accent.
- Inspector glass panels: radius 16, `nt @ .10 → .045`, top highlight `nt @ .15`, e2, 14 px backdrop blur.
- Preview "orb": rounded squircle tile (58 px, radius 17) in accent gradient, gently bobbing (5.5 s ease-in-out, ±5 px, ±3°). Treat as ambient — must stop under reduced motion and when window is occluded/minimized.
- Progress: 6 px pill, `nt @ .14` track, `gl → ac` fill with `ac @ .5` glow.

**Palettes** (Copper default; all eight user-selectable):

| Palette | `ac` | `gl` | `nt` | `b1` | `b2` | `tx` | `tx2` | `tx3` |
|---|---|---|---|---|---|---|---|---|
| **Copper (default)** | 255 160 122 | 230 120 80 | 245 205 185 | #16110e | #0b0807 | #f4ebe5 | #ab9a90 | #7d6f66 |
| Ember | 255 189 108 | 255 170 80 | 240 215 180 | #16130f | #0c0a08 | #f3ede4 | #a89d8c | #7b7264 |
| Jade | 122 240 184 | 90 220 160 | 190 235 215 | #0d1512 | #070c0a | #e8f4ed | #93ae9f | #667d71 |
| Orchid | 226 150 255 | 190 120 255 | 225 200 245 | #140f1a | #0a070e | #efe8f6 | #a898b5 | #7a6c87 |
| Sulfur | 245 224 92 | 220 210 70 | 235 232 180 | #14140d | #0a0a06 | #f2f1e4 | #a5a38c | #767464 |
| Rose | 255 150 180 | 240 110 150 | 250 200 215 | #170f13 | #0b070a | #f6e9ee | #b0949d | #806b73 |
| Mono | 236 236 236 | 200 200 200 | 222 222 222 | #131313 | #080808 | #f0f0f0 | #9c9c9c | #6e6e6e |
| Moss | 255 200 110 | 150 200 110 | 215 230 195 | #12150f | #080a07 | #eef2e7 | #9daa90 | #707c66 |

**The product owner explicitly does not want blue for Halo.** Do not add a blue/cyan palette to Halo.

## 7. Motion system

### 7.1 Global rules

1. **Every duration is a token.** All durations are multiplied by one global scale `motion.speed` (default 1.0). This single knob drives both reduced-motion and debug slow-mo.
2. **Max 220 ms** for any interaction animation. Exceptions: theme/palette crossfade 250 ms (rare, user-initiated) and ambient loops (scan shimmer, orb, reels), which must be low-amplitude.
3. **Everything is interruptible** mid-flight. A new input retargets from the current value; it never queues behind the old animation.
4. **Never on the first-paint path.** The directory-change crossfade animates the *outgoing* content; incoming rows paint at full opacity immediately.
5. **Reduced motion** collapses every animation to instant and stops all loops. Sources, in order: the in-app setting; the Windows "Animation effects" setting via `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)` ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa)). Re-read it on `WM_SETTINGCHANGE`.
6. Default easing **`cubic-bezier(0.4, 0, 0.2, 1)`** (standard/physical). Exceptions are noted per row.
7. Pause all ambient loops when the window is minimized, occluded, or inactive for more than a few seconds.
8. Animations run on a compositor/timer tick that is independent of filesystem work. Dropping animation frames is acceptable; blocking input is not.

### 7.2 The 29 animations

All approved by the product owner. "Ref" = what `theme-reference.html` does (Halo scene; open it and use the dock under the window to play each one in isolation, with a speed slider and a reduced-motion toggle).

| # | Animation | Trigger | Duration | Properties | Notes |
|---|---|---|---|---|---|
| **Selection & hover** |||||
| 1 | Row hover tint | pointer enter/leave row | 90 in / 120 out, ease-out | background alpha 0 → `nt @ .06` | Must not lag the pointer on fast sweeps; retarget, don't restart |
| 2 | Selection halo bloom | click / keyboard select | 140 | scale .985 → 1, opacity .65 → 1, glow on | Ring and glow fade in together |
| 3 | Multi-select range fill | shift-click / shift-arrow | 12 ms stagger per row | per-row #2 | Stagger only the first 8 rows; everything after row 8 appears together at +96 ms. Never stagger 100k rows |
| 4 | Focus ring (keyboard only) | keyboard focus | 100 | 2 px `ac @ .75` ring + soft glow | Never on mouse focus |
| 5 | Sidebar active crossfade | navigate to pinned/volume | 120 | background, icon stroke colour | |
| 6 | Checkbox / toggle | state change | 130 | fill, knob position | |
| **Navigation** |||||
| 7 | Directory change crossfade | open folder | 110 | outgoing content opacity → 0.25 + 8 px slide in travel direction | **Incoming rows paint immediately.** Ref animates the whole list in; native must only decorate, never delay |
| 8 | Breadcrumb segment slide | depth change | 160 | FLIP: existing segments translate from old x; new segments enter 14 px + fade | |
| 9 | Back / forward hint | back/forward | 140 | content translates 10 px in travel direction → 0 | Hitting the end of history triggers #24 instead |
| 10 | Tree expand / collapse | chevron / arrow keys | 150 | height 0 ↔ content, chevron rotate 0 ↔ 90° | Children are enumerated async; expand animates to a placeholder height if not ready |
| 11 | Inspector open / close | toggle | 180 | width, opacity; children fade in 120 ms starting at +60 ms, +20 ms stagger | List reflows; reflow must not re-query data |
| 12 | Tab open / close / reorder | new tab, ✕, drag | 150 | open: scale .9 + translateY −4 → none; close: → scale .9 + fade; drag: follows pointer 1:1, settles 150 ms | Reorder commits when dragged > 60 px |
| **Data & progress** |||||
| 13 | Transfer progress | bytes copied | continuous, eased (ref: 400 ms per update) | width toward target | Ease toward target; never snap backwards |
| 14 | Index scan shimmer | index build active | 1.4 s loop, linear | low-alpha `ac @ .16` band sweeping L→R | Ambient; stops under reduced motion |
| 15 | Throughput digit roll | rate update | 200, cubic ease-out | numeric interpolation | Monospace digits so there's no reflow |
| 16 | Row insert / remove | USN / watcher event | 120 | height 0 ↔ row, opacity, 4 px Y | Batch bursts: if more than ~20 changes per 100 ms, skip animation and just repaint |
| 17 | Search result reorder | each keystroke | 130 | FLIP translateY; new rows fade in | Animate visible rows only; matched substring highlighted in accent bold |
| **Chrome & window** |||||
| 18 | Window open / close | create / close | 160 | scale .98 ↔ 1 + opacity | Likely DirectComposition / DWM; don't use for minimize/restore (system owns that) |
| 19 | Window focus / blur | WM_ACTIVATE | 150, ease-out | chrome desaturate (saturate .6, brightness .86 in ref) | Native: lower accent/glow alpha and tint; filter is a web shortcut |
| 20 | Menu / context menu | open | 120 | opacity 0 → 1, translateY −4 → 0 | Glass material, radius 15 |
| 21 | Tooltip | hover dwell | 400 delay, 90 in, **60 out** | opacity | **Discrepancy:** ref uses 90 ms out. Implement 60 ms per approved list |
| 22 | Dialog / modal | open | 170 | dialog scale .96 + 6 px Y → none; scrim opacity + blur 0 → 7 px | Close: 140 ms scale .97 + fade |
| 23 | Snap / resize | — | **0 (none)** | — | Follows pointer 1:1, no easing, ever |
| **Feedback** |||||
| 24 | Denied-action shake | access denied, end of history | 180 × 2 | translateX 0 → −3 → +3 → 0 | Easing `cubic-bezier(.36,.07,.19,.97)`. Exactly twice |
| 25 | Toast | notification | 150 in / 200 out | in: 18 px X + fade; out: 10 px X + fade | Dwell 2000 ms; stack bottom-right, newest at bottom |
| 26 | Copy / paste confirm pulse | clipboard op | 200, ease-out | affected rows flash `ac @ .55` → transparent | Single flash |
| 27 | Drag & drop | drag a row | ghost: 0 (1:1); target glow 120; drop settle 140 | ghost follows pointer; target `ac @ .2` fill + ring + glow; settle scale 1.03 → 1 | Drag starts after 5 px of movement |
| **Theme** |||||
| 28 | Palette switch crossfade | palette change | 250, ease-in-out | full-window opacity dip to .55 and back while tokens swap | Native: crossfade two rendered frames, not a real opacity dip |
| 29 | Light / dark switch | theme change | 250 | same path as #28 | |

### 7.3 Never animated

Scrolling (1:1 with input, no smooth-scroll easing), column resize, the text caret, window drag, and anything on the path to first paint.

### 7.4 Ambient effects inventory (per theme)

| Theme | Loop | Period | Stops when |
|---|---|---|---|
| Halo | preview orb bob | 5.5 s | reduced motion, occluded, inactive |
| Halo | scan shimmer (#14) | 1.4 s | no index build |
| Deck | tape reels | 3.4 s / 5.6 s | no transfer |
| Deck | rec LED blink (hard steps) | 2.4 s | reduced motion (flashing content) |
| Strata | wireframe cube orbit | 14 s | reduced motion, occluded |

---

## 8. Window chrome: drawing our own frame

Every theme draws its own chrome (no system title bar). Key platform facts:

- Standard approach: handle `WM_NCCALCSIZE` (wParam TRUE) to extend the client area over the caption, keep the left/right/bottom resize borders in the non-client area, handle `WM_NCHITTEST` for caption drag, resize edges, and caption buttons ([Microsoft Learn: Custom Window Frame Using DWM](https://learn.microsoft.com/en-us/windows/win32/dwm/customframe)). Extending the frame by a minimal margin (e.g. `{0,0,1,0}`) is the common way to keep the DWM shadow ([handmade.network write-up](https://handmade.network/forums/articles/t/9073-custom_window_title_bar_and_almost_correctly_drawing_windows_10_borders)).
- Maximized windows: clamp the proposed client rect to the monitor work area and remove the top resize border, or the window covers the taskbar or gets clipped ([mullion notes](https://github.com/Burakuslendera/mullion/blob/main/docs/snap-and-nonclient-region.md)).
- **Snap Layouts flyout:** to keep it on hover, return `HTMAXBUTTON` from `WM_NCHITTEST` over our maximize button. One source reports the flyout only reliably appears with a native caption band — **verify on Windows 11 before committing** ([mullion notes](https://github.com/Burakuslendera/mullion/blob/main/docs/snap-and-nonclient-region.md)).
- **Rounded corners:** Windows 11 auto-rounds windows only when it can infer a standard frame; apps that customize most of the frame lose auto-rounding, and per-pixel-alpha layering for a custom shadow makes the window non-rectangular and disables system rounding entirely. `DWMWA_WINDOW_CORNER_PREFERENCE` is a hint ([Microsoft Learn: apply rounded corners](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-rounded-corners)).

**Consequence by theme:**

| Theme | Silhouette | Can use DWM shadow + rounding? |
|---|---|---|
| Halo | rounded rect (r 26, superellipse) | DWM rounding is circular and ~8 px, so it can't produce Halo's 26 px squircle. Matching the reference requires a per-pixel-alpha window + our own shadow |
| Lozenge | tabs above body (non-rectangular top edge) | No, per-pixel alpha |
| Vellum | rounded desk | Same as Halo |
| Deck | chamfered polygon | No, per-pixel alpha |
| Strata | 14 px rounded rect | Closest to the system; could accept DWM rounding |

This is the open decision from earlier rounds; see §11, item 1.

---

## 9. Accessibility & robustness

- **Contrast:** body text ≥ 4.5:1 and secondary text ≥ 3:1 against its actual surface (including tinted selection). Verify all 8 Halo palettes plus G, I, D, E. Accent-on-accent text (active breadcrumb segment) uses the dark ink `#14100a`.
- **High Contrast mode:** when Windows High Contrast / contrast themes are on, drop custom themes and render with system colours. Non-negotiable.
- **Reduced motion:** §7.1 rule 5. Also stop all flashing (Deck LED).
- **Keyboard:** every interactive element reachable; focus ring is #4 (keyboard-only).
- **DPI:** per-monitor DPI v2. All geometry in DIPs, caches keyed by DPI, recomputed on `WM_DPICHANGED`.
- **GPU loss:** handle `D2DERR_RECREATE_TARGET` / device removed. Rebuild resources and repaint, never crash, never block.
- **Software fallback:** if hardware acceleration is unavailable, disable blur (glass → solid `nt` tint) and ambient loops, and keep everything else.

---

## 10. Acceptance criteria

The implementing agent should show evidence for each:

1. All 8 Halo palettes, plus G, I, D, E, render with values matching §6 (screenshot side-by-side against `theme-reference.html` at 100% and 150% DPI).
2. Copper is the default on a fresh profile; palette and theme switch live with no filesystem re-query (trace or log proof).
3. Each of the 29 animations plays with the stated duration ±1 frame at 60 Hz and is interruptible (record a capture at 0.25× speed scale).
4. Reduced motion (in-app toggle and Windows setting) turns everything instant and stops every loop.
5. First paint of a 100k-item directory is identical with animations on vs. off (timing log).
6. Idle RSS stays ≤ 80 MB with any theme active; switching themes 50× doesn't grow memory (leak check).
7. Scrolling a 100k-row list holds frame rate with hover/selection animations active; no row ever draws a Gaussian blur per frame.
8. High Contrast mode falls back to system colours.
9. Custom frame: drag, resize on all edges, maximize/restore, multi-monitor, and DPI change all work; Snap behaviour documented per §8.

---
