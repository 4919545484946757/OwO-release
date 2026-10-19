#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import json
import sys
import tempfile
from pathlib import Path


def main() -> int:
    source_root = Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(source_root / "tools"))
    import validate_ranker_dataset as validator

    fixture_path = source_root / "tests/data/ranker_dataset_fixture.jsonl"
    manifest_path = source_root / "tests/data/ranker_dataset_fixture.manifest.json"
    records = [json.loads(line) for line in fixture_path.read_text(encoding="utf-8").splitlines()]
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    temporary_root = source_root / "build"
    temporary_root.mkdir(exist_ok=True)

    def expect_failure(name: str, changed_records: list[dict], changed_manifest: dict | None = None) -> None:
        with tempfile.TemporaryDirectory(prefix="owo-ranker-validator-",
                                         dir=temporary_root) as directory:
            root = Path(directory)
            dataset = root / "dataset.jsonl"
            dataset_bytes = ("\n".join(json.dumps(record, ensure_ascii=False, separators=(",", ":"))
                                        for record in changed_records) + "\n").encode("utf-8")
            dataset.write_bytes(dataset_bytes)
            candidate_manifest = copy.deepcopy(changed_manifest if changed_manifest is not None else manifest)
            candidate_manifest["dataset_sha256"] = hashlib.sha256(dataset_bytes).hexdigest()
            manifest_file = root / "manifest.json"
            manifest_file.write_text(json.dumps(candidate_manifest, ensure_ascii=False), encoding="utf-8")
            try:
                validator.validate(manifest_file, dataset)
            except ValueError:
                return
            raise AssertionError(f"validator accepted invalid case: {name}")

    bad = copy.deepcopy(records)
    bad[1]["group_id"] = bad[0]["group_id"]
    expect_failure("cross-split group leakage", bad)

    bad = copy.deepcopy(records)
    bad[0]["candidates"] = ["你好", "你好"]
    expect_failure("duplicate candidate", bad)

    bad = copy.deepcopy(records)
    bad[0]["selected_index"] = 2
    expect_failure("selected index", bad)

    bad = copy.deepcopy(records)
    bad[0]["context"] = "Cafe\u0301"
    expect_failure("non-NFC text", bad)

    bad_manifest = copy.deepcopy(manifest)
    bad_manifest["sources"][0]["privacy_class"] = "explicit_consent"
    expect_failure("missing consent record", records, bad_manifest)

    with tempfile.TemporaryDirectory(prefix="owo-ranker-validator-", dir=temporary_root) as directory:
        root = Path(directory)
        dataset = root / "dataset.jsonl"
        dataset.write_bytes(fixture_path.read_bytes() + b" ")
        copied_manifest = root / "manifest.json"
        copied_manifest.write_bytes(manifest_path.read_bytes())
        try:
            validator.validate(copied_manifest, dataset)
        except ValueError:
            pass
        else:
            raise AssertionError("validator accepted a dataset SHA-256 mismatch")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
