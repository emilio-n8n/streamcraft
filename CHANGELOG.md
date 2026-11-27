# Changelog — StreamCraft

All notable changes to StreamCraft are documented here.

## [Unreleased]

### Fixed

- Capture preview, audio metering and scene transitions now request repaints
  while active, including on an otherwise static scene.
- X11 capture uses the visual, pixel format, masks and byte order returned by
  the server, reconnects for every frame, rejects stale window IDs and clears
  obsolete preview frames after errors.
- Recording uses an exact wall-clock frame timeline (including 60 fps), bounds
  catch-up, counts only successfully written frames and reports empty recordings.
- ffmpeg stderr is drained concurrently with a bounded diagnostic tail; child
  processes are terminated and reaped on write failures, cancellation and exit.
- PNG fallback recordings use unique directories and include FPS/frame metadata,
  preventing silent overwrites and making manual muxing deterministic.
- Integer microphone formats now use RMS rather than peak level; non-finite
  samples and asynchronous device errors are handled visibly.
- Project loading rejects duplicate IDs and invalid scene selection, clamps
  malformed geometry and advances stale ID counters without changing schema 1.
- Transition durations are recorded as one undo gesture, scene navigation no
  longer dirties a one-scene project, and transparent sources no longer intercept
  canvas selection.
- PNG encoding writes IDAT incrementally, removing full-size raw/compressed/CRC
  copies from the recording path.

### Added

- Recording frame-rate selection (15/24/30/60 fps), capture-target refresh,
  webcam rescan, blended incoming/outgoing scenes in preview and true cross-fade
  rendering for recorded transition frames, and regression tests for X11
  formats, ffmpeg encoding/cleanup and project validation.
- CI runs the real ffmpeg recorder test and ignored X11 capture tests under Xvfb.

## [0.2.2] - 2026-11-23

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
