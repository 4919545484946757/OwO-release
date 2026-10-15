"""校验 Agent IPC 黄金样例与 schema 一致。

- valid 样例必须通过 schema；
- level=schema 的 invalid 样例必须被 schema 拒绝；
- level=semantic 的 invalid 样例 schema 无法表达，由 C++/Rust 实现测试负责拒绝；
- contracts/ 内的 schema 必须与 ime/docs/plugins/ 中的副本逐字节一致。
"""
import json
import sys
from pathlib import Path

import jsonschema

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def main() -> int:
    manifest = json.loads((HERE / "manifest.json").read_text(encoding="utf-8"))
    schema_path = HERE / manifest["schema"]
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    validator = jsonschema.Draft202012Validator(schema)
    failures = []
    ime_copy = ROOT / "ime" / "docs" / "plugins" / schema_path.name
    if ime_copy.exists() and ime_copy.read_bytes().replace(b"\r\n", b"\n") != schema_path.read_bytes().replace(b"\r\n", b"\n"):
        failures.append(f"{ime_copy} 与 {schema_path} 不一致")
    listed = set()
    for entry in manifest["fixtures"]:
        listed.add(entry["file"])
        name = Path(entry["file"]).name
        if not (name.startswith(entry["kind"] + ".") and name.endswith(f".{entry['expect']}.json")):
            failures.append(f"{entry['file']}: 文件名必须是 <kind>.<名称>.<expect>.json（C++ 测试按文件名判定）")
        document = json.loads((HERE / entry["file"]).read_text(encoding="utf-8"))
        valid = validator.is_valid(document)
        if entry["expect"] == "valid" and not valid:
            errors = "; ".join(e.message for e in validator.iter_errors(document))
            failures.append(f"{entry['file']}: 应通过 schema：{errors}")
        if entry["expect"] == "invalid" and entry.get("level") == "schema" and valid:
            failures.append(f"{entry['file']}: 应被 schema 拒绝")
    for path in sorted((HERE / "fixtures").glob("*.json")):
        relative = path.relative_to(HERE).as_posix()
        if relative not in listed:
            failures.append(f"{relative}: 未登记到 manifest.json")
    for failure in failures:
        print(f"FAIL {failure}")
    print(f"checked {len(manifest['fixtures'])} fixtures, {len(failures)} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
