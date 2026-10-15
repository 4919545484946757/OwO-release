#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
from pathlib import Path


def expect_failure(callback, label: str) -> None:
    try:
        callback()
    except (OSError, ValueError):
        return
    raise AssertionError(f"evaluator accepted invalid case: {label}")


def main() -> int:
    source_root = Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(source_root / "tools"))
    import evaluate_ranker as evaluator

    manifest = source_root / "tests/data/ranker_dataset_fixture.manifest.json"
    dataset = source_root / "tests/data/ranker_dataset_fixture.jsonl"
    records = evaluator.load_records(manifest, dataset, "test")
    example_id = records[0]["example_id"]
    result = evaluator.evaluate(records, {example_id: [0.1, 0.9]})
    if result != {
        "schema_version": 1, "examples": 1, "baseline_top1_accuracy": 0.0,
        "model_top1_accuracy": 1.0, "net_top1_accuracy": 1.0, "mrr": 1.0,
        "helpful_fixes": 1, "harmful_regressions": 0, "harmful_regression_rate": 0.0
    }:
        raise AssertionError(f"unexpected metrics: {json.dumps(result, sort_keys=True)}")

    expect_failure(lambda: evaluator.load_predictions(
        source_root / "tests/data/ranker_predictions_missing.jsonl", {example_id: 2}),
        "missing prediction file")
    expect_failure(lambda: evaluator.load_predictions(
        source_root / "tests/data/ranker_predictions_invalid.jsonl", {example_id: 2}),
        "non-finite scores")
    tie = evaluator.evaluate(records, {example_id: [1.0, 1.0]})
    if tie["model_top1_accuracy"] != 0.0 or tie["mrr"] != 0.5:
        raise AssertionError("score ties must preserve deterministic base order")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
