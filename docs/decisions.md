# Decisions — StreamCraft

Short architecture decision records. Newest first.

## ADR-004 — Capture abstraction and honest fallbacks

Date: 2026-11-22

Decision: capture goes through a `CaptureBackend` trait. Linux builds implement
it with `x11rb`'s core `GetImage`; other platforms return an explicit
`Unsupported` error. A missing display is a normal error, never a fake frame.
The preview shows target selection, live frames and the real error text.

Alternatives: use a heavyweight cross-platform capture crate; render a fake
desktop rectangle when no device exists; fail to start the application.

Reasons: the prompt asks for Linux/X11 first and a clear fallback elsewhere;
`x11rb` was already resolved in the dependency graph; tests must run headless.

Consequences: the X11 path is smoke-tested under Xvfb with an ignored test; it
is not claimed to work on Wayland or a physical monitor. Window titles use
`WM_NAME`, which is absent for some window managers.

## ADR-005 — Recording without promising video encoding

Date: 2026-11-22

Decision: recording is planned from the environment. If `ffmpeg` is on `PATH`,
composed PNG frames are piped into it to produce MKV/MP4. Otherwise the
recorder writes a numbered PNG sequence into a directory and explains how to
mux it. No video encoder is bundled.

Alternatives: bundle FFmpeg bindings; claim recording works without testing;
require ffmpeg at build time.

Reasons: ffmpeg is an optional runtime dependency, so the fallback must be real
and testable; the UI must never overstate capabilities.

Consequences: MKV/MP4 encoding is exercised by an integration test when ffmpeg
is available and by CI. PNG sequences use unique directories plus FPS metadata.
The recorder anchors its frame timeline to wall-clock time, drains bounded
ffmpeg diagnostics concurrently, and kills/reaps the child on cancellation.

## ADR-006 — Schema-compatible transitions

Date: 2026-11-22

Decision: `Scene.transition` is an optional serde field defaulting to a cut.
The format stays `streamcraft` and `schema_version` stays 1.

Alternatives: bump the schema and write a migration; store transitions in a
separate file.

Reasons: older v0.1.0 JSON must still open; a defaulted additive field is
backward compatible and keeps the release snapshot intact.

Consequences: a test parses a v0.1.0 document without the field. A future
breaking change still requires a schema bump.

## ADR-001 — Built on CraftKit

Date: 2026-11-22

Decision: use the pinned CraftKit crates for the native shell, project
lifecycle, history and background jobs.

Alternatives: build the desktop foundation from scratch; use another UI
framework.

Reasons: consistent behaviour across the portfolio, fewer duplicated bugs,
shared release and testing conventions.

Consequences: CraftKit upgrades are explicit tag changes; domain code stays in
this repository.

## ADR-002 — Initial document model

Date: 2026-11-22

Decision: a `Document` owns versioned `Scene`s, each holding `Source`s in a
`Vec` (later entries are drawn on top). Geometry is in logical canvas pixels
(1280x720), positions are top-left origins. Source identifiers come from a
document-wide `next_id` counter so they stay unique across scenes.

Alternatives: per-scene counters; normalized 0..1 coordinates; a tree of
layers.

Reasons: integer-like identifiers keep undo snapshots and JSON stable; pixel
geometry maps directly to both the egui canvas and exported images; a flat
z stack is enough for a first compositor.

Consequences: z-order changes are vector reorders; a schema bump is required
if source geometry ever becomes normalized.

## ADR-003 — Exports without an image dependency

Date: 2026-11-22

Decision: SVG is produced by a pure string serializer and PNG by a small
software rasterizer plus a stored-DEFLATE PNG encoder in `src/render.rs`.

Alternatives: add the `png`/`image` crates; read pixels back from egui's GPU
painter.

Reasons: keeps `cargo test` runnable without a GPU or window server, avoids new
dependencies and makes export byte-for-byte testable.

Consequences: PNG text uses a compact 3x5 bitmap font rather than the system
font used on screen, and images are uncompressed (stored DEFLATE blocks).
