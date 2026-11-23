# Changelog — StreamCraft

All notable changes to StreamCraft are documented here.

## [0.2.1] - 2026-11-23

### Fixed

- Recording now requests a repaint at the configured frame rate, so a static
  scene keeps producing frames instead of stopping after a short burst.
- Missed intervals repeat the last frame, keeping the video duration close to
  wall-clock time when rendering is slower than the target frame rate.

## [0.2.0] - 2026-11-22

### Added

- Linux/X11 screen and window capture preview behind a `CaptureBackend`
  abstraction, with an explicit unsupported fallback on other platforms.
  Smoke-tested under Xvfb via an ignored unit test.
- Microphone level meter built on `cpal`, reporting "no device" honestly.
- Webcam device detection (`/dev/video*`) with a clear notice that frame
  decoding is not implemented in this build.
- Local recording: composed PNG frames piped into `ffmpeg` for MKV/MP4 when it
  is on `PATH`, otherwise a numbered PNG sequence with mux instructions.
- Scene transitions (cut / fade) with a per-scene duration, a canvas fade
  overlay and pure progress math.
- Shortcuts: PageUp/PageDown for scenes, F11 for fullscreen, Ctrl+R to record.
- CI installs `libasound2-dev` for the `cpal` dependency.

### Changed

- The document keeps the `streamcraft` format and schema 1: `Scene.transition`
  is defaulted so files written by v0.1.0 still load unchanged.
- Tests cover scene graph navigation, transition math, schema-1 compatibility,
  capture frame validation, audio RMS, ffmpeg fallback planning and a tested
  PNG-sequence recording path.

## [0.1.0] - 2026-11-22

### Added

- Initial CraftKit-based application shell.
- Versioned project files with atomic save.
- Undo/redo through the shared document history.
- Background jobs with progress and cancellation.
- Scene compositor: z-ordered sources, canvas drawing, drag-to-move with one
  undo step per drag, SVG/PNG export.
- Domain tests for lifecycle, z-order, opacity, transform clamping, JSON
  round-trip, undo/redo and non-empty raster/vector exports.
