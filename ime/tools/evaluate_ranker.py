#!/usr/bin/env python3
"""Evaluate candidate-ranker scores against an OwO dataset without external packages."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

import validate_ranker_dataset as dataset_validator

PREDICTION_KEYS = {"schema_version", "example_id", "scores"}


def load_records(manifest_path: Path, dataset_path: Path, split: str) -> list[dict[str, object]]:
    dataset_validator.validate(manifest_path, dataset_path)
    records = []
    for line_number, line in enumerate(dataset_path.read_text(encoding="utf-8").splitlines(), 1):
        record = dataset_validator.parse_json(line, f"dataset line {line_number}")
        if record["split"] == split:
            records.append(record)
    if not records:
        raise ValueError(f"dataset has no records in split {split}")
    return records


def load_predictions(path: Path, expected: dict[str, int]) -> dict[str, list[float]]:
    if path.stat().st_size > 100 * 1024 * 1024:
        raise ValueError("predictions exceed 100 MiB")
    predictions: dict[str, list[float]] = {}
    text = path.read_text(encoding="utf-8")
    if text.startswith("\ufeff"):
        raise ValueError("predictions must not contain a UTF-8 BOM")
    for line_number, line in enumerate(text.splitlines(), 1):
        if not line or len(line.encode("utf-8")) > 64 * 1024:
            raise ValueError(f"prediction line {line_number} is empty or too large")
        prediction = dataset_validator.strict_object(
            dataset_validator.parse_json(line, f"prediction line {line_number}"),
            PREDICTION_KEYS, f"prediction line {line_number}")
        if prediction["schema_version"] != 1:
            raise ValueError(f"prediction line {line_number}: unsupported schema_version")
        example_id = dataset_validator.identifier(
            prediction["example_id"], f"prediction line {line_number}.example_id")
        if example_id not in expected:
            raise ValueError(f"prediction line {line_number}: unexpected example_id")
        if example_id in predictions:
            raise ValueError(f"prediction line {line_number}: duplicate example_id")
        scores = prediction["scores"]
        if not isinstance(scores, list) or len(scores) != expected[example_id]:
            raise ValueError(f"prediction line {line_number}: score count mismatch")
        if any(isinstance(score, bool) or not isinstance(score, (int, float)) or
               not math.isfinite(score) for score in scores):
            raise ValueError(f"prediction line {line_number}: scores must be finite numbers")
        predictions[example_id] = [float(score) for score in scores]
    missing = set(expected) - set(predictions)
    if missing:
        raise ValueError(f"predictions missing {len(missing)} expected examples")
    return predictions


def evaluate(records: list[dict[str, object]], predictions: dict[str, list[float]]) -> dict[str, object]:
    baseline_correct = 0
    model_correct = 0
    reciprocal_rank_sum = 0.0
    helpful_fixes = 0
    harmful_regressions = 0
    for record in records:
        selected = record["selected_index"]
        baseline_hit = selected == 0
        baseline_correct += int(baseline_hit)
        scores = predictions[record["example_id"]]
        ranking = sorted(range(len(scores)), key=lambda index: (-scores[index], index))
        model_hit = ranking[0] == selected
        model_correct += int(model_hit)
        reciprocal_rank_sum += 1.0 / (ranking.index(selected) + 1)
        helpful_fixes += int(not baseline_hit and model_hit)
        harmful_regressions += int(baseline_hit and not model_hit)
    count = len(records)
    baseline_accuracy = baseline_correct / count
    model_accuracy = model_correct / count
    harmful_rate = harmful_regressions / baseline_correct if baseline_correct else 0.0
    return {
        "schema_version": 1,
        "examples": count,
        "baseline_top1_accuracy": baseline_accuracy,
        "model_top1_accuracy": model_accuracy,
        "net_top1_accuracy": model_accuracy - baseline_accuracy,
        "mrr": reciprocal_rank_sum / count,
        "helpful_fixes": helpful_fixes,
        "harmful_regressions": harmful_regressions,
        "harmful_regression_rate": harmful_rate,
    }


def input_length_bucket(record: dict[str, object]) -> str:
    length = len(record["input"].replace("'", ""))
    return "1-4" if length <= 4 else "5-8" if length <= 8 else "9+"


def context_length_bucket(record: dict[str, object]) -> str:
    length = len(record["context"])
    return "0" if length == 0 else "1-8" if length <= 8 else "9-32" if length <= 32 else "33+"


def breakdown(records: list[dict[str, object]], predictions: dict[str, list[float]], key) -> dict[str, object]:
    groups: dict[str, list[dict[str, object]]] = {}
    for record in records:
        groups.setdefault(str(key(record)), []).append(record)
    return {name: evaluate(group, predictions) for name, group in sorted(groups.items())}


def evaluate_report(records: list[dict[str, object]],
                    predictions: dict[str, list[float]]) -> dict[str, object]:
    result = evaluate(records, predictions)
    result["breakdown"] = {
        "input_length": breakdown(records, predictions, input_length_bucket),
        "candidate_count": breakdown(records, predictions, lambda record: len(record["candidates"])),
        "context_length": breakdown(records, predictions, context_length_bucket),
        "source_id": breakdown(records, predictions, lambda record: record["source_id"]),
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("predictions", type=Path)
    parser.add_argument("--split", choices=sorted(dataset_validator.SPLITS), default="test")
    parser.add_argument("--minimum-net-accuracy", type=float)
    parser.add_argument("--maximum-harmful-regression-rate", type=float)
    arguments = parser.parse_args()
    if (arguments.minimum_net_accuracy is not None and
            (not math.isfinite(arguments.minimum_net_accuracy) or
             not -1.0 <= arguments.minimum_net_accuracy <= 1.0)):
        parser.error("--minimum-net-accuracy must be finite and between -1 and 1")
    if (arguments.maximum_harmful_regression_rate is not None and
            (not math.isfinite(arguments.maximum_harmful_regression_rate) or
             not 0.0 <= arguments.maximum_harmful_regression_rate <= 1.0)):
        parser.error("--maximum-harmful-regression-rate must be finite and between 0 and 1")
    try:
        records = load_records(arguments.manifest, arguments.dataset, arguments.split)
        expected = {record["example_id"]: len(record["candidates"]) for record in records}
        result = evaluate_report(records, load_predictions(arguments.predictions, expected))
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
        print(f"ranker evaluation failed: {error}", file=sys.stderr)
        return 1
    result["split"] = arguments.split
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    if (arguments.minimum_net_accuracy is not None and
            result["net_top1_accuracy"] < arguments.minimum_net_accuracy):
        print("ranker evaluation gate failed: net accuracy below minimum", file=sys.stderr)
        return 3
    if (arguments.maximum_harmful_regression_rate is not None and
            result["harmful_regression_rate"] > arguments.maximum_harmful_regression_rate):
        print("ranker evaluation gate failed: harmful regression rate above maximum", file=sys.stderr)
        return 4
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
