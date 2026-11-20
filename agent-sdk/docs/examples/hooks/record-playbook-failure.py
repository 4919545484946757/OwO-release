#!/usr/bin/env python3
"""把 playbook_check / playbook_run 的失败记录成"待转规则"清单。

用法（settings.json 的 hooks，stdin 由宿主注入事件 JSON）：

    {
      "hooks": [
        {
          "event": "post_tool_use",
          "matcher": "playbook_*",
          "command": "python docs/examples/hooks/record-playbook-failure.py learned-failures.md"
        }
      ]
    }

语义：post_tool_use 是通知型钩子（工具已执行、结果不可撤销）；本脚本只追加记录，
不阻断、不改结果。退出码恒为 0，避免误伤回合。

记录的每条都带 tool/kind/error，人工复核后可以转成剧本里的 `check` 断言或
`max_attempts` 调整 —— 这是"事故 → 检查项"的最小闭环。
"""

import datetime
import json
import os
import sys

FAILING_TOOLS = {"playbook_check", "playbook_run"}


def main() -> int:
    target = sys.argv[1] if len(sys.argv) > 1 else "learned-failures.md"
    try:
        payload = json.load(sys.stdin)
    except (ValueError, OSError):
        return 0
    tool = str(payload.get("tool") or "")
    if tool not in FAILING_TOOLS:
        return 0
    if payload.get("ok") is not False:
        return 0
    error = str(payload.get("error") or "未知错误").replace("\n", " ").strip()
    if len(error) > 400:
        error = error[:400] + "…"
    line = "- {} `{}` 失败：{}\n".format(
        datetime.datetime.now().strftime("%Y-%m-%d %H:%M"), tool or "?", error
    )
    directory = os.path.dirname(os.path.abspath(target))
    os.makedirs(directory, exist_ok=True)
    with open(target, "a", encoding="utf-8") as handle:
        handle.write(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
