# Rapid Explorer (working title)

A fast, lightweight, native Windows file explorer built to replace Windows 11's File Explorer, which suffers from render latency caused by its hybrid Win32/WinUI/XAML Islands architecture.

## What it does

Windows 11 File Explorer is slow to render even on a fresh install because of layered rendering frameworks, shell-extension loading, and COM-based folder enumeration. Rapid Explorer solves this by using native Win32 list/tree controls in virtual (owner-data) mode for the UI, a memory-safe Rust engine for filesystem enumeration and NTFS-level indexing, and process isolation for anything (shell extensions, thumbnail generation) that has historically made Explorer unstable. The goal is Explorer-level familiarity with materially better speed and no regression in reliability.

## Status

`fx-core` enumerates directories with one worker per volume, caches a finished listing, and exposes `shell` (open, poll, cancel, and a network browse that does not use that cache). `list_shell` is a Win32 `SysListView32` in virtual mode, with a `SysTreeView32` of child folders, a one-line address bar, Back/Forward, and a collapsed Network node. Expanding a folder lists its children through `shell`. Choosing a folder, entering a path, going back, or choosing a connectable share lists that directory. Painted rows show a small icon cached by extension. Size and Date modified are DirectWrite columns; Name stays native. Visible image rows request thumbnails on a background pool. The list has a native context menu for Copy, Cut, Paste, Rename, Delete, and Properties. More options asks `menu_host`, a watchdog-supervised process that returns no verbs and does not load a shell extension. See `docs/ROADMAP.md`.

## Getting started

The enumeration core is the `fx-core` crate. From that directory:

```
cargo test
```

On Windows, open one directory in the list window:

```
cargo run --bin list_shell -- C:\some\directory
```

A cache hit paints stored names before the live listing finishes. The tree starts collapsed. Expand a folder to list its child directories. The first expand of the open directory waits until the list's listing of that same volume finishes, then paints from the cache. Choose a tree folder, double-click a directory in the list, or press Enter in the address bar to list that path. Back and Forward (and Alt+Left / Alt+Right) revisit directories this window already opened. If that volume is busy, the current list stays and the window retries without freezing. Icons are looked up once per extension, not once per file. Size and Date modified are drawn for the visible rows; a directory's size is blank, and the date is UTC. Image thumbnails are decoded off the UI thread and drawn over the icon when ready. Leaving the folder cancels those requests. The Network node stays empty until you expand it. A connectable share then lists like any other path. A network timeout is not retried on its own. More options looks for `menu_host` beside `list_shell` (`cargo build --bins` builds both). The helper returns no extra commands. A panic writes a local dump under `dumps` beside the executable and does not send it anywhere. Timeout stores the network timeout beside the executable. The default is 2 seconds, a missing file is not created, and local folders are not timed. A filename index keeps path records in memory and applies a scripted journal. It does not read the disk.

On Windows, measure a real directory with:

```
cargo run --example measure_listing -- C:\some\directory
```

The example prints first-batch and full-listing times next to the ≤250 ms and ≤1 s targets. It does not pass or fail those targets. No SSD measurement is recorded in the repo yet.

## Documentation

- Architecture: `docs/ARCHITECTURE.md`
- Roadmap: `docs/ROADMAP.md`
- Diagrams: `docs/charts/`
- Contributing: `CONTRIBUTING.md`
- Handoff (active work / agent protocol): `docs/HANDOFF.md`
- Theme and motion spec: `docs/theme-motion-spec.md`

## License

TBD.
