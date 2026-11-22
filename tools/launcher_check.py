#!/usr/bin/env python3
"""Validate a CraftKit application against the public ArtCraft launcher rules.

Checks (without any third-party dependency):

- craft-app.json is at most 16 KiB and has the expected fields;
- the application id matches ^[a-z0-9-]+$;
- the manifest does not claim an official ArtCraft namespace or team;
- the icon exists and is a 128x128 PNG;
- with --release-dir, every file is covered by SHA256SUMS.txt, hashes match,
  and every asset follows <id>-<version>-<os>-<arch>.<ext>.

This is a local guard, not a replacement for the launcher's own verifier, and
it does not sign packages.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import sys
from pathlib import Path

MAX_MANIFEST_BYTES = 16 * 1024
ID_RE = re.compile(r"^[a-z0-9-]+$")
REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
COLOR_RE = re.compile(r"^#[0-9a-fA-F]{6}$")
BUNDLE_RE = re.compile(r"^[A-Za-z0-9.-]+$")
OFFICIAL_PREFIXES = ("ai.storyteller.",)
OFFICIAL_TEAM = "DJ6XS33FX8"
OFFICIAL_PUBLISHERS = {"learning machines llc", "artcraft", "storytold"}
ASSET_RE = re.compile(
    r"^(?P<id>[a-z0-9-]+)-(?P<version>[0-9][0-9A-Za-z.+-]*)-"
    r"(?P<os>linux|windows|macos|freebsd)-"
    r"(?P<arch>x86_64|x64|x86|aarch64|arm64|universal|riscv64)"
    r"(?:-portable)?\.(?P<ext>tar\.gz|zip|msi|dmg|AppImage|deb|rpm|pkg\.tar\.zst|flatpak)$"
)
HASH_RE = re.compile(r"^[0-9a-fA-F]{64}$")


def error(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)


def read_png_size(path: Path) -> tuple[int, int] | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    if data[12:16] != b"IHDR":
        return None
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def load_manifest(path: Path) -> dict | None:
    if not path.is_file():
        error(f"manifest not found: {path}")
        return None
    size = path.stat().st_size
    if size > MAX_MANIFEST_BYTES:
        error(f"manifest is {size} bytes; maximum is {MAX_MANIFEST_BYTES}")
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        error(f"cannot parse {path}: {exc}")
        return None
    if not isinstance(data, dict):
        error("manifest root must be an object")
        return None
    return data


def validate_manifest(data: dict, expected_repo: str | None) -> bool:
    ok = True
    required = {
        "schema": int,
        "id": str,
        "name": str,
        "tagline": str,
        "repo": str,
        "publisher": str,
        "upstream": (str, type(None)),
        "bundle_id": str,
        "msi_publisher": str,
        "apple_team": (str, type(None)),
        "website": str,
        "color": str,
        "icon": str,
    }
    for field, expected_type in required.items():
        if field not in data:
            error(f"missing field: {field}")
            ok = False
            continue
        if not isinstance(data[field], expected_type):
            error(f"{field} has the wrong type")
            ok = False
    if not ok:
        return False

    if data["schema"] != 1:
        error("schema must be 1")
        ok = False
    if not ID_RE.match(data["id"]):
        error("id must match ^[a-z0-9-]+$")
        ok = False
    if not REPO_RE.match(data["repo"]):
        error("repo must be owner/name")
        ok = False
    if expected_repo and data["repo"] != expected_repo:
        error(f"repo must be {expected_repo}, found {data['repo']}")
        ok = False
    if data["upstream"] is not None and not REPO_RE.match(data["upstream"]):
        error("upstream must be null or owner/name")
        ok = False
    if not BUNDLE_RE.match(data["bundle_id"]):
        error("bundle_id contains invalid characters")
        ok = False
    if any(data["bundle_id"].startswith(prefix) for prefix in OFFICIAL_PREFIXES):
        error("bundle_id must not use the official ai.storyteller.* namespace")
        ok = False
    for field in ("publisher", "msi_publisher"):
        if data[field].strip().lower() in OFFICIAL_PUBLISHERS:
            error(f"{field} must not claim an official ArtCraft publisher")
            ok = False
    if data["apple_team"] == OFFICIAL_TEAM:
        error("apple_team must not use ArtCraft's Developer ID team")
        ok = False
    if not COLOR_RE.match(data["color"]):
        error("color must be #RRGGBB")
        ok = False
    if not data["website"].startswith(("https://", "http://")):
        error("website must be an http(s) URL")
        ok = False
    if not data["icon"].endswith(".png"):
        error("icon must be a PNG file name")
        ok = False
    return ok


def validate_icon(app_dir: Path, data: dict) -> bool:
    icon = data["icon"]
    candidates = [app_dir / icon, app_dir / "assets" / icon]
    path = next((candidate for candidate in candidates if candidate.is_file()), None)
    if path is None:
        error(f"icon not found: {icon} (looked at {app_dir} and {app_dir / 'assets'})")
        return False
    size = read_png_size(path)
    if size is None:
        error(f"icon is not a valid PNG: {path}")
        return False
    width, height = size
    if width != 128 or height != 128:
        error(f"icon must be 128x128, found {width}x{height}: {path}")
        return False
    return True


def validate_release(app_dir: Path, data: dict, release_dir: Path) -> bool:
    ok = True
    sums_path = release_dir / "SHA256SUMS.txt"
    if not sums_path.is_file():
        error(f"missing {sums_path}")
        return False

    sums: dict[str, str] = {}
    for line in sums_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if len(parts) != 2 or not HASH_RE.match(parts[0]):
            error(f"invalid SHA256SUMS.txt line: {line}")
            ok = False
            continue
        name = parts[1].lstrip("*")
        sums[name] = parts[0].lower()

    files = [path for path in release_dir.iterdir() if path.is_file()]
    names = {path.name for path in files}
    for required in ("SHA256SUMS.txt", "craft-app.json", data["icon"]):
        if required not in names:
            error(f"release is missing {required}")
            ok = False

    for path in files:
        if path.name == "SHA256SUMS.txt":
            continue
        expected = sums.get(path.name)
        if expected is None:
            error(f"SHA256SUMS.txt does not cover {path.name}")
            ok = False
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            error(f"checksum mismatch for {path.name}")
            ok = False

    for line_name in sums:
        if line_name not in names:
            error(f"SHA256SUMS.txt names a missing file: {line_name}")
            ok = False

    for name in names:
        if name in {"SHA256SUMS.txt", "craft-app.json", data["icon"]}:
            continue
        match = ASSET_RE.match(name)
        if not match:
            error(f"asset does not follow <id>-<version>-<os>-<arch>.<ext>: {name}")
            ok = False
        elif match.group("id") != data["id"]:
            error(f"asset id does not match manifest id: {name}")
            ok = False
        elif name.endswith(".zip") and not name.endswith("-portable.zip"):
            error(f"Windows zip must end in -portable.zip: {name}")
            ok = False

    if not (app_dir / "craft-app.json").is_file():
        error("craft-app.json must be committed at the repository root")
        ok = False
    return ok


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app_dir", type=Path, help="application repository root")
    parser.add_argument(
        "--release-dir",
        type=Path,
        help="directory containing the files attached to the release",
    )
    parser.add_argument(
        "--expected-repo",
        help="expected GitHub repository, for example owner/app",
    )
    args = parser.parse_args()

    app_dir = args.app_dir.resolve()
    data = load_manifest(app_dir / "craft-app.json")
    if data is None:
        return 1

    ok = validate_manifest(data, args.expected_repo)
    ok = validate_icon(app_dir, data) and ok
    if args.release_dir is not None:
        ok = validate_release(app_dir, data, args.release_dir.resolve()) and ok

    if ok:
        print("launcher check: OK")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
