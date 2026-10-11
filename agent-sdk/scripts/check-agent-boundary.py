#!/usr/bin/env python3
"""Keep the active Agent workspace independent of the separate IME project."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
import tempfile
import tomllib

FORBIDDEN_PACKAGE = "owo-agent-ime"
LEGACY_ROOT_ENTRIES = (
    "CMakeLists.txt", "CMakePresets.json", "vcpkg.json", "global.json",
    "src/tsf", "include/owo", "apps/settings_center", "packaging/windows",
    "docs/archive/owo-input-method",
)


def mentions_ime(value: object) -> bool:
    if isinstance(value, str):
        return FORBIDDEN_PACKAGE in value.lower().replace("_", "-")
    if isinstance(value, dict):
        return any(mentions_ime(k) or mentions_ime(v) for k, v in value.items())
    if isinstance(value, list):
        return any(mentions_ime(v) for v in value)
    return False


def inspect(root: Path) -> list[str]:
    root = root.resolve()
    errors: list[str] = []
    for relative in LEGACY_ROOT_ENTRIES:
        if (root.parent / relative).exists():
            errors.append("Legacy IME entry exists in repository root: " + relative)
    manifest = root / "Cargo.toml"
    if not manifest.is_file():
        return ["Agent workspace manifest is missing"]
    workspace = tomllib.loads(manifest.read_text(encoding="utf-8")).get("workspace", {})
    if not workspace.get("members"):
        errors.append("Agent workspace members are missing")
    if mentions_ime(workspace):
        errors.append("IME reference exists in workspace configuration")
    manifests = set((root / "crates").glob("*/Cargo.toml"))
    manifests.update((root / "devtools").glob("*/Cargo.toml"))
    for pattern in workspace.get("members", []):
        for member in root.glob(pattern):
            target = (member / "Cargo.toml").resolve()
            if not target.is_relative_to(root):
                errors.append("Workspace member escapes Agent root: " + pattern)
            elif not target.is_file():
                errors.append("Workspace member manifest is missing: " + pattern)
            else:
                manifests.add(target)
    for path in sorted(manifests):
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        if mentions_ime(data):
            errors.append("IME package or dependency exists in " + path.relative_to(root).as_posix())
    lock = root / "Cargo.lock"
    if not lock.is_file():
        errors.append("Agent lockfile is missing")
    elif mentions_ime(tomllib.loads(lock.read_text(encoding="utf-8"))):
        errors.append("IME reference exists in Agent lockfile")
    return errors


def metadata_errors(data: dict) -> list[str]:
    return [
        "Resolved dependency graph contains IME package: " + package["name"]
        for package in data.get("packages", [])
        if mentions_ime(package.get("name", ""))
    ]


def self_test() -> None:
    """Exercise clean graphs, alias/target dependencies and stale/orphan fixtures."""
    with tempfile.TemporaryDirectory(prefix="owo-agent-boundary-") as directory:
        root = Path(directory) / "agent-sdk"
        crate = root / "crates/cli"
        crate.mkdir(parents=True)
        workspace = root / "Cargo.toml"
        workspace.write_text('[workspace]\nmembers = ["crates/cli"]\n', encoding="utf-8")
        manifest = crate / "Cargo.toml"
        clean = '[package]\nname = "owo-agent-cli"\nversion = "0.1.0"\n'
        manifest.write_text(clean, encoding="utf-8")
        lock = root / "Cargo.lock"
        lock.write_text('version = 4\n', encoding="utf-8")
        assert not inspect(root), "Clean Agent fixture should pass"
        manifest.write_text(
            clean + '\n[target.\'cfg(windows)\'.dependencies.bridge]\npackage = "owo-agent-ime"\nversion = "0.1.0"\n',
            encoding="utf-8",
        )
        assert inspect(root), "Aliased target-specific IME dependency must fail"
        manifest.write_text(clean, encoding="utf-8")
        lock.write_text('version = 4\n[[package]]\nname = "owo-agent-ime"\nversion = "0.1.0"\n', encoding="utf-8")
        assert inspect(root), "Stale IME lock entry must fail"
        lock.write_text('version = 4\n', encoding="utf-8")
        orphan = root / "crates/orphan"
        orphan.mkdir()
        (orphan / "Cargo.toml").write_text(clean.replace("owo-agent-cli", "owo-agent-ime"), encoding="utf-8")
        assert inspect(root), "Orphan IME package outside members must fail"
        (orphan / "Cargo.toml").unlink()
        workspace.write_text('[workspace]\nmembers = ["crates/cli", "crates/owo-agent-ime"]\n', encoding="utf-8")
        assert inspect(root), "IME workspace membership must fail"
        assert metadata_errors({"packages": [{"name": "owo-agent-ime"}]}), "Resolved IME must fail"
        assert not metadata_errors({"packages": [{"name": "owo-agent-cli"}]}), "Clean graph should pass"
        workspace.write_text('[workspace]\nmembers = ["crates/cli"]\n', encoding="utf-8")
        legacy = root.parent / "CMakeLists.txt"
        legacy.write_text("project(OwOInputMethod)\n", encoding="utf-8")
        assert inspect(root), "Root C++ IME project must fail"
        legacy.unlink()
        assert not inspect(root), "Clean root should pass after removing legacy project"

    print("Agent boundary negative fixtures passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--metadata", type=Path, help="Optional cargo metadata JSON to verify")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        errors = inspect(args.root)
        if args.metadata:
            errors.extend(metadata_errors(json.loads(args.metadata.read_text(encoding="utf-8"))))
        if args.self_test:
            self_test()
    except (OSError, ValueError, AssertionError) as error:
        print("Agent boundary check failed: " + str(error), file=sys.stderr)
        return 1
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("Agent boundary passed: workspace, crate manifests and lockfile contain no IME integration")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
