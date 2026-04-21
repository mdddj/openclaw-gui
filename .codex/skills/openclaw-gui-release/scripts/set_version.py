#!/usr/bin/env python3
"""
Sync the openclaw-gui project version across package.json, Cargo.toml, and tauri.conf.json.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

VERSION_RE = re.compile(
    r"^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Sync openclaw-gui version fields across project files."
    )
    parser.add_argument("version", help="Target version, for example 0.1.1 or v0.1.1")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show the files that would change without writing them",
    )
    return parser.parse_args()


def normalize_version(raw_version: str) -> str:
    if not VERSION_RE.match(raw_version):
        raise ValueError(
            f"Invalid version `{raw_version}`. Expected semver like 0.1.1 or v0.1.1."
        )
    return raw_version[1:] if raw_version.startswith("v") else raw_version


def repo_root() -> Path:
    return Path(__file__).resolve().parents[4]


def update_json_version(path: Path, version: str, dry_run: bool) -> bool:
    data = json.loads(path.read_text(encoding="utf-8"))
    changed = data.get("version") != version
    data["version"] = version

    if changed and not dry_run:
        path.write_text(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )

    return changed


def update_cargo_version(path: Path, version: str, dry_run: bool) -> bool:
    original = path.read_text(encoding="utf-8")
    updated, count = re.subn(
        r'(?m)^version = "[^"]+"$',
        f'version = "{version}"',
        original,
        count=1,
    )

    if count != 1:
        raise ValueError(f"Could not find package version line in {path}")

    changed = updated != original
    if changed and not dry_run:
        path.write_text(updated, encoding="utf-8")

    return changed


def main() -> int:
    args = parse_args()

    try:
        version = normalize_version(args.version)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1

    root = repo_root()
    targets = [
        ("package.json", root / "package.json", update_json_version),
        ("src-tauri/Cargo.toml", root / "src-tauri" / "Cargo.toml", update_cargo_version),
        (
            "src-tauri/tauri.conf.json",
            root / "src-tauri" / "tauri.conf.json",
            update_json_version,
        ),
    ]

    changed_files: list[str] = []
    unchanged_files: list[str] = []

    for label, path, updater in targets:
        changed = updater(path, version, args.dry_run)
        if changed:
            changed_files.append(label)
        else:
            unchanged_files.append(label)

    action = "Would update" if args.dry_run else "Updated"
    if changed_files:
        print(f"{action} version to {version}:")
        for label in changed_files:
            print(f"  - {label}")
    else:
        print(f"All target files already use version {version}.")

    if unchanged_files and changed_files:
        print("Already up to date:")
        for label in unchanged_files:
            print(f"  - {label}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
