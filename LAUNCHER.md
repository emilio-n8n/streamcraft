# Launcher metadata — StreamCraft

This application can be described by the ArtCraft Launcher through a
`craft-app.json` descriptor. The descriptor is data, not code: the launcher
validates it and never executes the application inside its own process.

## Two onboarding paths

1. **Approved community app** (for example SolidCraft or MailCraft): the
   launcher maintainers review the project and pin its repository in the
   launcher catalog. This requires a launcher release and, on macOS, a matching
   Developer ID signature.
2. **Unofficial app** (the `craft-app.json` proposal, storytold/craft-launcher
   issue #989): the user adds this repository in the launcher settings. The app
   appears in an unofficial section, without review or endorsement. This is the
   faster path, but the launcher assumes no liability.

`craft-app.json` is the descriptor for the second path. It is also useful
metadata if the app later applies for the approved community catalog.

## Descriptor

`craft-app.json` is at the repository root, under 16 KiB:

```json
{
  "schema": 1,
  "id": "streamcraft",
  "name": "StreamCraft",
  "tagline": "Scenes, sources and a compositor for local recording",
  "repo": "emilio-n8n/streamcraft",
  "publisher": "emilio-n8n",
  "upstream": null,
  "bundle_id": "io.github.emilion8n.streamcraft",
  "msi_publisher": "emilio-n8n",
  "apple_team": null,
  "website": "https://github.com/emilio-n8n/streamcraft",
  "color": "#ef4444",
  "icon": "streamcraft-128.png"
}
```

- The identifier must match `^[a-z0-9-]+$`.
- Do not use `ai.storyteller.*`, `Learning Machines LLC`, or ArtCraft's
  Developer ID team. This project is independent.
- `repo` must be the exact repository the launcher downloads from.
- The icon is a valid 128×128 PNG stored in `assets/` and attached to releases
  as `streamcraft-128.png`.
- `apple_team` stays `null` until a Developer ID certificate exists. A null team
  means the launcher will not install a macOS DMG; it can still open an
  existing copy.

## Release contract

Each release should attach:

- archives named `<id>-<version>-<os>-<arch>.<ext>`:
  - `streamcraft-<version>-linux-x86_64.tar.gz` or `.AppImage` / `.deb` / `.rpm`
  - `streamcraft-<version>-windows-x64-portable.zip` or `.msi`
  - `streamcraft-<version>-macos-aarch64.dmg` / `macos-x86_64.dmg`
- `craft-app.json`
- `streamcraft-128.png`
- `SHA256SUMS.txt` covering every file above.

The launcher verifies the checksums before running or installing anything.

Validate locally:

```bash
python3 tools/launcher_check.py .
python3 tools/launcher_check.py . --release-dir dist
```

## Signing status

- Linux: AppImage and tar.gz can be unsigned.
- Windows: a portable ZIP works for the unofficial path; an MSI needs an
  Authenticode certificate and the publisher in the descriptor must match.
- macOS: a DMG is the launcher's preferred package and requires a Developer ID
  signature and notarization for installation. Without certificates, keep
  `apple_team: null` and ship a portable archive for manual use.

## Submission

Once the app is stable:

1. Run the release checks and verify `SHA256SUMS.txt`.
2. Open a discussion or issue in `storytold/craft-launcher` with the repository,
   a short demo, the manifest and the current signing status.
3. Ask which path the maintainers prefer before adding signing or an MSI/DMG
   pipeline.
4. Do not claim official affiliation before written approval.

The ArtCraft name and marks belong to their owners. This project is independent.
