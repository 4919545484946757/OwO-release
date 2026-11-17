#!/usr/bin/env python3
"""Agent Plugins 1.0.0 示例 MCP 服务器（stdio，JSON-RPC 2.0）。

演示两件事：
1. 规范包里的 `mcp.json` 如何声明一个 stdio 服务器（command/args/env/cwd）；
2. `${PLUGIN_ROOT}` 展开后的根目录确实传到了子进程（`plugin_info` 会回显）。

只依赖 Python 标准库；无网络、无文件写入。
"""

import json
import os
import sys
import io

PLUGIN_ROOT = os.environ.get("OWO_DEMO_PLUGIN_ROOT", "")


def handle(method, message):
    identifier = message.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": identifier, "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "owo-demo-kit", "version": "1.0.0"},
        }}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": identifier, "result": {"tools": [
            {
                "name": "echo",
                "description": "原样返回输入文本（无副作用，用于验证工具通道）",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "要回显的文本"},
                    },
                    "required": ["text"],
                },
            },
            {
                "name": "plugin_info",
                "description": "返回插件根目录与环境标签（验证 ${PLUGIN_ROOT} 展开与 cwd）",
                "inputSchema": {"type": "object", "properties": {}},
            },
        ]}}
    if method == "tools/call":
        params = message.get("params") or {}
        name = params.get("name")
        arguments = params.get("arguments") or {}
        if name == "echo":
            text = arguments.get("text") or ""
            return {"jsonrpc": "2.0", "id": identifier, "result": {
                "content": [{"type": "text", "text": text}]
            }}
        if name == "plugin_info":
            payload = {
                "plugin_root": PLUGIN_ROOT,
                "cwd": os.getcwd(),
                "python": sys.version.split()[0],
            }
            return {"jsonrpc": "2.0", "id": identifier, "result": {
                "content": [{"type": "text", "text": json.dumps(payload, ensure_ascii=False)}]
            }}
        return {"jsonrpc": "2.0", "id": identifier, "error": {
            "code": -32602, "message": f"unknown tool: {name}"
        }}
    if method in ("notifications/initialized", "exit"):
        return None
    return {"jsonrpc": "2.0", "id": identifier, "error": {
        "code": -32601, "message": f"method not found: {method}"
    }}


def main():
    stdin = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", errors="replace")
    stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = message.get("method", "")
        response = handle(method, message)
        if response is None:
            if method == "exit":
                break
            continue
        stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
        stdout.flush()


if __name__ == "__main__":
    main()
