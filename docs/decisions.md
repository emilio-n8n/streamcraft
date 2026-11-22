# Decisions — StreamCraft

Short architecture decision records. Newest first.

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
