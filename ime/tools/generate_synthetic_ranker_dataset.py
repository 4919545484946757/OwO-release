#!/usr/bin/env python3
"""Generate deterministic pipeline-only OwO ranker data and oracle predictions."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

CASES = (
    ("nihao", ["泥号", "你好"], 1, "请向大家"),
    ("xian", ["先", "线", "西安"], 2, "我来自"),
    ("shijie", ["视界", "世界", "试解", "时节"], 1, "放眼整个"),
    ("ceshi", ["侧视", "测试"], 1, "这是一次"),
    ("gongyuan", ["公元", "公园", "工员"], 1, "周末去"),
    ("tianqi", ["天启", "天气", "田七", "添气"], 1, "今天的"),
    ("beijing", ["背景", "北京"], 1, "首都是"),
    ("changcheng", ["长成", "长城", "厂城"], 1, "参观万里"),
    ("renmin", ["人名", "人民", "任敏"], 1, "服务于"),
    ("zhongguo", ["中过", "中国", "钟果", "忠国"], 1, "我的祖国是"),
    ("wenhuagong", ["文化宫", "问话工"], 0, "活动地点在"),
    ("jisuanji", ["计算机", "寄算计", "集算机"], 0, "学习使用"),
)


def generate(repetitions: int) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    records = []
    predictions = []
    splits = ("train", "train", "train", "train", "train", "train", "validation", "validation", "test", "test")
    for repetition in range(repetitions):
        for case_index, (raw_input, candidates, selected, context) in enumerate(CASES):
            sequence = repetition * len(CASES) + case_index
            split = splits[sequence % len(splits)]
            suffixes = (
                f"{sequence:03d}",
                f"，场景{sequence:03d}",
                f"，这是用于验证分层聚合的较长合成上下文，并且绝不用于真实模型质量结论，场景编号{sequence:03d}",
                f"，合成场景{sequence:03d}",
            )
            suffix = suffixes[sequence % len(suffixes)]
            example_id = f"synthetic.{split}.{sequence:04d}"
            records.append({
                "schema_version": 1,
                "example_id": example_id,
                "split": split,
                "group_id": f"synthetic.group.{sequence:04d}",
                "source_id": "owo.synthetic.pipeline.v1",
                "source_record_id": f"case.{sequence:04d}",
                "context": context + suffix,
                "input": raw_input,
                "candidates": candidates,
                "selected_index": selected,
            })
            scores = [float(index == selected) for index in range(len(candidates))]
            predictions.append({"schema_version": 1, "example_id": example_id, "scores": scores})
    return records, predictions


def write_jsonl(path: Path, values: list[dict[str, object]]) -> bytes:
    content = ("\n".join(json.dumps(value, ensure_ascii=False, separators=(",", ":"))
                           for value in values) + "\n").encode("utf-8")
    path.write_bytes(content)
    return content


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_directory", type=Path)
    parser.add_argument("--repetitions", type=int, default=10)
    arguments = parser.parse_args()
    if not 1 <= arguments.repetitions <= 1000:
        parser.error("--repetitions must be between 1 and 1000")
    arguments.output_directory.mkdir(parents=True, exist_ok=True)
    records, predictions = generate(arguments.repetitions)
    dataset_path = arguments.output_directory / "synthetic-ranker.jsonl"
    dataset_bytes = write_jsonl(dataset_path, records)
    test_ids = {record["example_id"] for record in records if record["split"] == "test"}
    prediction_path = arguments.output_directory / "synthetic-ranker-test-predictions.jsonl"
    write_jsonl(prediction_path, [value for value in predictions if value["example_id"] in test_ids])
    manifest = {
        "schema_version": 1,
        "dataset_id": "owo.ranker.synthetic-pipeline",
        "dataset_version": "1.0.0",
        "license": "project-test-data",
        "dataset_sha256": hashlib.sha256(dataset_bytes).hexdigest(),
        "sources": [{
            "source_id": "owo.synthetic.pipeline.v1",
            "license": "project-test-data",
            "redistribution_allowed": True,
            "privacy_class": "synthetic",
            "consent_record": None,
        }],
    }
    manifest_path = arguments.output_directory / "synthetic-ranker.manifest.json"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n",
                             encoding="utf-8")
    print(json.dumps({"records": len(records), "test_predictions": len(test_ids),
                      "manifest": str(manifest_path), "dataset": str(dataset_path),
                      "predictions": str(prediction_path)}, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
