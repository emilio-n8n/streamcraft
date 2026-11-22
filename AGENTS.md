# StreamCraft — agent and contributor guide

StreamCraft is a native Rust desktop application built on CraftKit.

## Environment

Never build large artifacts inside `$HOME`.

```sh
export CARGO_HOME=/tmp/cargo-home
export RUSTUP_HOME=/tmp/rustup-home
export PATH=/tmp/cargo-home/bin:$PATH
export CARGO_TARGET_DIR=/tmp/cargo-target
```

- Light checks: `cargo check -p streamcraft`.
- Full release binaries are built by GitHub Actions, not locally.
- Keep `target/`, archives and `squashfs-root/` out of Git.

## Architecture

```text
src/main.rs       CLI, window options and eframe startup
src/app.rs        UI, menus, actions and project lifecycle
src/document.rs   application document model
```

CraftKit provides:

- `craftkit-core`: `History<T>`, `ProjectEnvelope<T>`, schema validation.
- `craftkit-io`: `DocumentHost<T>`, atomic JSON saves, settings, recent files.
- `craftkit-jobs`: background jobs with progress and cancellation.
- `craftkit-ui`: shell, theme, dialogs, CLI and `run_native`.

The application owns its document, panels and domain algorithms. Do not put
domain logic in the UI event handlers; keep it in testable modules.

## Rules

- Every persisted mutation goes through `DocumentHost::apply` or a history
  gesture (`begin_gesture` / `current_mut` / `end_gesture`).
- Long operations run through `JobRunner`; never block `ui()`.
- Parse external input defensively; no `unwrap` on user files.
- A schema change bumps `SCHEMA_VERSION` and adds a migration test.
- No `unsafe`. Comments and UI wording in English.
- Fixtures and demo assets must be generated or licence-clear.

## Quality gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Release

- Annotated `vX.Y.Z` tag after CI is green.
- The release workflow builds Linux, Windows and macOS archives and publishes
  `SHA256SUMS.txt`.
- Download and run the produced binary before announcing a release.
