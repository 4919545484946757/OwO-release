#!/usr/bin/env python3

from __future__ import annotations

import sys
import tempfile
from pathlib import Path


def main() -> int:
    source_root = Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(source_root / "tools"))
    import evaluate_ranker
    import generate_synthetic_ranker_dataset as generator
    import validate_ranker_dataset

    records, all_predictions = generator.generate(10)
    if (records, all_predictions) != generator.generate(10):
        raise AssertionError("synthetic generator output must be deterministic")
    if len(records) != 120 or {record["split"] for record in records} != {"train", "validation", "test"}:
        raise AssertionError("synthetic generator must deterministically cover 120 records and all splits")
    temporary_root = source_root / "build"
    temporary_root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="owo-synthetic-ranker-", dir=temporary_root) as directory:
        root = Path(directory)
        dataset_path = root / "dataset.jsonl"
        dataset_bytes = generator.write_jsonl(dataset_path, records)
        import hashlib
        import json
        manifest = {
            "schema_version": 1, "dataset_id": "owo.ranker.synthetic-pipeline",
            "dataset_version": "1.0.0", "license": "project-test-data",
            "dataset_sha256": hashlib.sha256(dataset_bytes).hexdigest(),
            "sources": [{"source_id": "owo.synthetic.pipeline.v1", "license": "project-test-data",
                         "redistribution_allowed": True, "privacy_class": "synthetic",
                         "consent_record": None}],
        }
        manifest_path = root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        validate_ranker_dataset.validate(manifest_path, dataset_path)
        test_records = evaluate_ranker.load_records(manifest_path, dataset_path, "test")
        test_ids = {record["example_id"] for record in test_records}
        predictions = {value["example_id"]: value["scores"] for value in all_predictions
                       if value["example_id"] in test_ids}
        report = evaluate_ranker.evaluate_report(test_records, predictions)
        if report["model_top1_accuracy"] != 1.0 or report["mrr"] != 1.0:
            raise AssertionError("oracle synthetic predictions must be perfect")
        breakdown = report["breakdown"]
        if set(breakdown) != {"input_length", "candidate_count", "context_length", "source_id"}:
            raise AssertionError("all required breakdown dimensions must be present")
        if len(breakdown["candidate_count"]) < 3 or len(breakdown["input_length"]) < 2:
            raise AssertionError("synthetic test split does not exercise enough strata")
        if len(breakdown["context_length"]) < 3:
            raise AssertionError("synthetic test split must exercise short, medium and long contexts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
