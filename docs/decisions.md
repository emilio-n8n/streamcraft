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

Decision: <describe the document, units and identifiers>

Alternatives:

Reasons:

Consequences:
