# Changelog — StreamCraft

All notable changes to StreamCraft are documented here.

## [0.1.0] - 2026-11-22

### Added

- Initial CraftKit-based application shell.
- Versioned project files with atomic save.
- Undo/redo through the shared document history.
- Background jobs with progress and cancellation.
- Application-specific document model and UI.

### Added in tranche 1 (compositor)

- `Document`, `Scene`, `Source` and `SourceKind` domain model with a sample
  scene: coloured background, simplified SMPTE bars and a title.
- Scene add/remove/select and z-ordered source add/remove/raise/lower,
  visibility and opacity controls.
- Canvas rendering through egui's `Painter` with source selection and
  drag-to-move; a whole drag records one undo step.
- SVG export and dependency-free PNG export of the selected scene.
- Domain tests for lifecycle, z-order, opacity, transform clamping, JSON
  round-trip, undo/redo and non-empty raster/vector exports.
