"""Verify the OwOBot registration demo's two delivered files."""

import argparse
from collections import Counter
import csv
from pathlib import Path
import re


COLUMNS = ["姓名", "分组", "手机号"]
SOURCE = Path(__file__).with_name("活动报名-演示数据.csv")
DETAILS = "报名明细-脱敏.csv"
SUMMARY = "报名统计.md"


def read_csv(path):
    with path.open(encoding="utf-8-sig", newline="") as stream:
        table = csv.DictReader(stream)
        if table.fieldnames != COLUMNS:
            raise ValueError(f"{path.name} 的列应为 {'、'.join(COLUMNS)}")
        rows = list(table)
        if any(None in row or any(value is None for value in row.values()) for row in rows):
            raise ValueError(f"{path.name}: CSV row width differs from its header")
        return rows


def verify(directory):
    source = read_csv(SOURCE)
    details = read_csv(directory / DETAILS)
    expected_rows = Counter(
        (row["姓名"], row["分组"], "*" * 7 + row["手机号"][-4:]) for row in source
    )
    delivered_rows = Counter(tuple(row[column] for column in COLUMNS) for row in details)
    if delivered_rows != expected_rows:
        raise ValueError("脱敏明细的姓名、分组、手机号后四位或行数有误")

    report = (directory / SUMMARY).read_text(encoding="utf-8-sig")
    if any(row["手机号"] in report for row in source):
        raise ValueError("统计报表包含完整手机号")
    parsed = {}
    for line in report.splitlines():
        match = re.fullmatch(r"\s*\|\s*(A组|B组|总计)\s*\|\s*(\d+)\s*\|\s*", line)
        if match:
            label, value = match.groups()
            if label in parsed:
                raise ValueError(f"统计报表重复登记 {label}")
            parsed[label] = int(value)
    expected_counts = dict(Counter(row["分组"] for row in source))
    expected_counts["总计"] = len(source)
    if parsed != expected_counts:
        raise ValueError("统计报表的人数或总计有误")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()
    try:
        verify(args.output_dir)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print("通过。总计 12 人，A 组 7 人，B 组 5 人，明细只保留手机号后四位。")


if __name__ == "__main__":
    main()
