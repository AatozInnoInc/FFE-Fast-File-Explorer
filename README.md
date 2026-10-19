# Rapid Explorer (working title)

A fast, lightweight, native Windows file explorer built to replace Windows 11's File Explorer, which suffers from render latency caused by its hybrid Win32/WinUI/XAML Islands architecture.

## What it does

Windows 11 File Explorer is slow to render even on a fresh install because of layered rendering frameworks, shell-extension loading, and COM-based folder enumeration. Rapid Explorer solves this by using native Win32 list/tree controls in virtual (owner-data) mode for the UI, a memory-safe Rust engine for filesystem enumeration and NTFS-level indexing, and process isolation for anything (shell extensions, thumbnail generation) that has historically made Explorer unstable. The goal is Explorer-level familiarity with materially better speed and no regression in reliability.

## Status

Design phase — no code written yet. See `docs/ROADMAP.md` for the v1 slice definition and current progress.

## Getting started

Setup instructions will land with v1.

## Documentation

- Architecture: `docs/ARCHITECTURE.md`
- Roadmap: `docs/ROADMAP.md`
- Diagrams: `docs/charts/`
- Contributing: `CONTRIBUTING.md`
- Handoff (active work / agent protocol): `docs/HANDOFF.md`
- Theme and motion spec: `docs/theme-motion-spec.md`

## License

TBD.
