---
name: demo-kit
description: Agent Plugins 1.0.0 示例技能：说明 echo 与 plugin_info 两个演示工具的用法与边界。
---

# demo-kit

这个技能属于 `owo-demo-kit` 插件包，用来演示 Agent Plugins 1.0.0 的
「技能 + MCP 服务器」组合。它本身不执行任何动作，只描述工具语义。

## 工具

| 工具 | 用途 | 边界 |
|---|---|---|
| `echo` | 原样返回输入文本 | 纯字符串回显，不访问文件、不联网 |
| `plugin_info` | 返回插件根目录与环境标签 | 只读；根目录来自 `${PLUGIN_ROOT}` 展开 |

## 何时使用

- 需要验证插件是否被宿主正确加载（看 `plugin_info` 的根目录）。
- 需要一条确定性的、无副作用的工具调用（`echo`）。

## 权限

- 只声明 `agent:tools`；没有文件、命令、网络能力。
