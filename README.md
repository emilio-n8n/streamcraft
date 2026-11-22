# StreamCraft

Scenes, sources and a compositor for local recording

**StreamCraft is an independent, community open-source project.** It is not an
official product of, endorsed by, or affiliated with any third-party vendor it
may interoperate with.

Built on [CraftKit](https://github.com/emilio-n8n/craftkit), a small native Rust
desktop foundation.

## Features

- Scene compositor: scenes, z-ordered sources, drag-to-move on the canvas.
- Sources: flat colour, simplified SMPTE test pattern and text, with opacity,
  visibility and transform controls.
- Export the selected scene as SVG or PNG (dependency-free raster encoder).
- Versioned JSON project files (`streamcraft`, schema 1) with atomic saves.
- Bounded undo/redo through the shared document history; one step per drag.
- Native Rust desktop UI (egui/eframe), no web runtime.
- Offline-first: no account, no telemetry, no network required.

**Scope:** this release is a compositor only. Screen capture, webcam, microphone
and video encoding are not implemented yet; the UI states
"Compositor only - capture/recording next".

## Run

```bash
cargo run --release -- --sample
cargo run --release -- --open project.json
```

Without arguments the application opens an empty document.

- `--sample` opens the built-in demonstration document.
- `--open <file>` opens a project.
- `--version` prints the version.
- `--help` prints usage.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Linux windowing development packages are installed by the CI workflow.

## Project layout

```text
src/main.rs       CLI and window startup
src/app.rs        UI, menus, canvas (Painter) and lifecycle
src/document.rs   UI-free scene/source model, SVG export
src/render.rs     dependency-free raster renderer and PNG encoder
```

The application depends on pinned CraftKit crates from Git. Keep the tag in
`Cargo.toml` and `Cargo.lock` consistent when upgrading the foundation.

## File format

Projects are JSON envelopes:

```json
{
  "format": "streamcraft",
  "schema_version": 1,
  "app_version": "0.1.0",
  "document": {
    "title": "Untitled",
    "scenes": [
      { "id": 1, "name": "Scene 1", "sources": [] }
    ],
    "selected_scene": 0,
    "next_id": 2
  }
}
```

The application owns the `document` schema and its migrations. A newer
`schema_version` is rejected instead of being silently misread.

## Licence

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.

## Third-party notices

See [THIRD_PARTY.md](THIRD_PARTY.md).
