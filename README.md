# Rapid Explorer (working title)

A fast, lightweight, native Windows file explorer built to replace Windows 11's File Explorer, which suffers from render latency caused by its hybrid Win32/WinUI/XAML Islands architecture.

## What it does

Windows 11 File Explorer is slow to render even on a fresh install because of layered rendering frameworks, shell-extension loading, and COM-based folder enumeration. Rapid Explorer solves this by using native Win32 list/tree controls in virtual (owner-data) mode for the UI, a memory-safe Rust engine for filesystem enumeration and NTFS-level indexing, and process isolation for anything (shell extensions, thumbnail generation) that has historically made Explorer unstable. The goal is Explorer-level familiarity with materially better speed and no regression in reliability.

## Status

`fx-core` enumerates directories with one worker per volume, caches a finished listing, and exposes `shell` (open, poll, cancel). `list_shell` is a Win32 `SysListView32` in virtual mode for one directory. It calls `shell` in-process. There is no tree, address bar, icon, or context menu yet. See `docs/ROADMAP.md`.

## Getting started

The enumeration core is the `fx-core` crate. From that directory:

```
cargo test
```

On Windows, open one directory in the list window:

```
cargo run --bin list_shell -- C:\some\directory
```

A cache hit paints stored names before the live listing finishes.

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
