#!/usr/bin/env python3
"""Validate OwO candidate-ranking dataset manifest and JSONL without external packages."""

from __future__ import annotations

import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path

ID_PATTERN = re.compile(r"^[a-z0-9][a-z0-9._-]{0,127}$")
INPUT_PATTERN = re.compile(r"^[a-z']{1,64}$")
SPLITS = {"train", "validation", "test"}
PRIVACY_CLASSES = {"synthetic", "public_licensed", "explicit_consent"}
MANIFEST_KEYS = {"schema_version", "dataset_id", "dataset_version", "license", "dataset_sha256", "sources"}
SOURCE_KEYS = {"source_id", "license", "redistribution_allowed", "privacy_class", "consent_record"}
RECORD_KEYS = {
    "schema_version", "example_id", "split", "group_id", "source_id", "source_record_id",
    "context", "input", "candidates", "selected_index"
}


def fail(message: str) -> None:
    raise ValueError(message)


def parse_json(text: str, label: str) -> object:
    def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                fail(f"{label}: duplicate JSON key {key}")
            result[key] = value
        return result

    return json.loads(text, object_pairs_hook=unique_object,
                      parse_constant=lambda value: fail(f"{label}: invalid number {value}"))


def strict_object(value: object, keys: set[str], label: str) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != keys:
        fail(f"{label}: expected exactly {sorted(keys)}")
    return value


def identifier(value: object, label: str) -> str:
    if not isinstance(value, str) or not ID_PATTERN.fullmatch(value):
        fail(f"{label}: invalid identifier")
    return value


def nfc_text(value: object, label: str, maximum: int, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value) or len(value) > maximum:
        fail(f"{label}: invalid text length")
    if unicodedata.normalize("NFC", value) != value:
        fail(f"{label}: text must be NFC normalized")
    if any(unicodedata.category(char) in {"Cc", "Cs"} for char in value):
        fail(f"{label}: control characters and surrogate code points are forbidden")
    return value


def load_manifest(path: Path, dataset_bytes: bytes) -> dict[str, dict[str, object]]:
    manifest = strict_object(parse_json(path.read_text(encoding="utf-8"), "manifest"),
                             MANIFEST_KEYS, "manifest")
    if manifest["schema_version"] != 1:
        fail("manifest: unsupported schema_version")
    identifier(manifest["dataset_id"], "manifest.dataset_id")
    identifier(manifest["dataset_version"], "manifest.dataset_version")
    if not isinstance(manifest["license"], str) or manifest["license"].lower() in {"", "unknown"}:
        fail("manifest: dataset license must be explicit")
    digest = hashlib.sha256(dataset_bytes).hexdigest()
    if manifest["dataset_sha256"] != digest:
        fail(f"manifest: dataset SHA-256 mismatch ({digest})")
    if not isinstance(manifest["sources"], list) or not manifest["sources"]:
        fail("manifest: at least one source is required")
    sources: dict[str, dict[str, object]] = {}
    for index, raw_source in enumerate(manifest["sources"]):
        source = strict_object(raw_source, SOURCE_KEYS, f"source[{index}]")
        source_id = identifier(source["source_id"], f"source[{index}].source_id")
        if source_id in sources:
            fail(f"source[{index}]: duplicate source_id")
        if not isinstance(source["license"], str) or source["license"].lower() in {"", "unknown"}:
            fail(f"source[{index}]: source license must be explicit")
        if source["redistribution_allowed"] is not True:
            fail(f"source[{index}]: redistribution must be explicitly allowed")
        if source["privacy_class"] not in PRIVACY_CLASSES:
            fail(f"source[{index}]: forbidden privacy_class")
        consent = source["consent_record"]
        if source["privacy_class"] == "explicit_consent":
            identifier(consent, f"source[{index}].consent_record")
        elif consent is not None:
            fail(f"source[{index}]: consent_record is only valid for explicit_consent")
        sources[source_id] = source
    return sources


def validate(manifest_path: Path, dataset_path: Path) -> tuple[int, dict[str, int]]:
    if dataset_path.stat().st_size > 100 * 1024 * 1024:
        fail("dataset: file exceeds 100 MiB validation limit")
    dataset_bytes = dataset_path.read_bytes()
    sources = load_manifest(manifest_path, dataset_bytes)
    try:
        text = dataset_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        fail(f"dataset: invalid UTF-8 at byte {error.start}")
    if text.startswith("\ufeff"):
        fail("dataset: UTF-8 BOM is forbidden")

    example_ids: set[str] = set()
    group_splits: dict[str, str] = {}
    fingerprints: set[str] = set()
    counts = {split: 0 for split in SPLITS}
    lines = text.splitlines()
    if not lines:
        fail("dataset: empty file")
    for line_number, line in enumerate(lines, 1):
        if not line or len(line.encode("utf-8")) > 64 * 1024:
            fail(f"line {line_number}: empty or larger than 64 KiB")
        record = strict_object(parse_json(line, f"line {line_number}"), RECORD_KEYS,
                               f"line {line_number}")
        if record["schema_version"] != 1:
            fail(f"line {line_number}: unsupported schema_version")
        example_id = identifier(record["example_id"], f"line {line_number}.example_id")
        if example_id in example_ids:
            fail(f"line {line_number}: duplicate example_id")
        example_ids.add(example_id)
        split = record["split"]
        if split not in SPLITS:
            fail(f"line {line_number}: invalid split")
        counts[split] += 1
        group_id = identifier(record["group_id"], f"line {line_number}.group_id")
        if group_id in group_splits and group_splits[group_id] != split:
            fail(f"line {line_number}: group leaks across splits")
        group_splits[group_id] = split
        source_id = identifier(record["source_id"], f"line {line_number}.source_id")
        if source_id not in sources:
            fail(f"line {line_number}: unknown source_id")
        identifier(record["source_record_id"], f"line {line_number}.source_record_id")
        context = nfc_text(record["context"], f"line {line_number}.context", 256, allow_empty=True)
        raw_input = record["input"]
        if not isinstance(raw_input, str) or not INPUT_PATTERN.fullmatch(raw_input):
            fail(f"line {line_number}: input must be lowercase pinyin")
        candidates = record["candidates"]
        if not isinstance(candidates, list) or not 2 <= len(candidates) <= 8:
            fail(f"line {line_number}: candidates must contain 2 to 8 strings")
        candidates = [nfc_text(item, f"line {line_number}.candidates", 64) for item in candidates]
        if len(set(candidates)) != len(candidates):
            fail(f"line {line_number}: duplicate candidate")
        selected = record["selected_index"]
        if isinstance(selected, bool) or not isinstance(selected, int) or not 0 <= selected < len(candidates):
            fail(f"line {line_number}: selected_index is out of range")
        fingerprint = json.dumps([context, raw_input, candidates, selected], ensure_ascii=False, separators=(",", ":"))
        if fingerprint in fingerprints:
            fail(f"line {line_number}: duplicate example content")
        fingerprints.add(fingerprint)
    if any(count == 0 for count in counts.values()):
        fail("dataset: train, validation and test splits must all be non-empty")
    return len(lines), counts


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: validate_ranker_dataset.py <manifest.json> <dataset.jsonl>", file=sys.stderr)
        return 2
    try:
        total, counts = validate(Path(sys.argv[1]), Path(sys.argv[2]))
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
        print(f"ranker dataset validation failed: {error}", file=sys.stderr)
        return 1
    print(f"ranker dataset valid: records={total} train={counts['train']} validation={counts['validation']} test={counts['test']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
