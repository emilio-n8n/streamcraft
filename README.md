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
- Scene transitions (cut / fade) on scene activation, with keyboard shortcuts
  (PageUp/PageDown for scenes, F11 for fullscreen, Ctrl+R to record).
- Screen/window capture preview on Linux/X11 via `x11rb`, behind a
  `CaptureBackend` abstraction. Other platforms report an explicit
  "unsupported" status rather than faking a preview.
- Microphone level meter using `cpal` (reports "no device" when absent).
- Local recording: MKV/MP4 through `ffmpeg` when it is on `PATH`; otherwise a
  numbered PNG sequence that can be muxed later.
- Webcam device detection, with the UI stating clearly that frame capture is
  not implemented in this build.
- Export the selected scene as SVG or PNG (dependency-free raster encoder).
- Versioned JSON project files (`streamcraft`, schema 1) with atomic saves.
- Bounded undo/redo through the shared document history; one step per drag.
- Native Rust desktop UI (egui/eframe), no web runtime.
- Offline-first: no account, no telemetry, no network required.

**Scope and honesty:** this is a local scene compositor, not an OBS clone and
not a streaming/encoding pipeline. Screen capture is implemented for Linux/X11
and was smoke-tested under Xvfb; it has not been exercised on a physical
monitor or with Wayland. Microphone and webcam depend on host devices; in this
environment no microphone or `/dev/video*` device was available, so those paths
only report status and are not claimed as verified. Video recording requires
`ffmpeg` at runtime; without it the recorder writes PNG frames.

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

Linux windowing development packages and ALSA headers (`libasound2-dev`) are
installed by the CI workflow.

To smoke-test the X11 capture path on a headless machine (the test is ignored by
default because it needs a display):

```bash
Xvfb :99 -screen 0 1280x720x24 -nolisten tcp &
DISPLAY=:99 CARGO_TARGET_DIR=/tmp/cargo-target cargo test --workspace --locked -- --ignored
```

## Project layout

```text
src/main.rs       CLI and window startup
src/app.rs        UI, menus, capture preview, recording controls, canvas (Painter)
src/document.rs   UI-free scene/source model, transitions, SVG export
src/render.rs     dependency-free raster renderer and PNG encoder
src/capture.rs    CaptureBackend abstraction + Linux/X11 implementation
src/audio.rs      cpal microphone level meter
src/webcam.rs     camera device detection with explicit unavailable status
src/recording.rs  ffmpeg MKV/MP4 path + PNG-sequence fallback
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
