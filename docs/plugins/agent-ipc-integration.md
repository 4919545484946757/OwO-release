# 第三方 Agent 识别与接入规范

## 接入模型

OwO 通过官方进程插件 `org.owo.agent-ipc` 连接第三方 Agent。第三方 Agent 不加载进输入法进程，也不能直接调用 OwO 的插件服务。它只需在当前 Windows 用户会话中创建一个本机命名管道服务器。

默认端点：

```text
\\.\pipe\OwO.Agent.External.v1
```

可以在插件配置页改为 `\\.\pipe\` 下由 ASCII 字母、数字、点、连字符或下划线组成的其他名称。UNC、远程管道、相对路径和带目录穿越字符的名称会被拒绝。

OwO 对第三方 Agent 的识别依据是：用户明确配置的本机管道端点、协议版本、严格响应结构和完全相同的 `session_id`。显示名称或自然语言 `message` 不构成身份凭据。生产实现应为管道设置仅允许当前用户 SID 访问的 ACL；如需区分多个 Agent，应分别使用不同固定管道名并由用户明确选择。

## 传输格式

每次连接处理一问一答：

1. 客户端发送 4 字节无符号小端长度；
2. 客户端发送对应长度的 UTF-8 JSON；
3. 服务端发送 4 字节无符号小端长度；
4. 服务端发送对应长度的 UTF-8 JSON，然后可关闭连接。

单个 JSON 最大 65536 字节，单次操作默认超时 10 秒，可配置为 500–30000 毫秒。服务端必须处理客户端超时或断开，不能把半包当作完整请求。

协议版本为 1。JSON 对象使用严格字段集合，不接受重复或未知字段。字符串可使用 UTF-8 字符或标准 JSON `\uXXXX` 转义；命令 ID 仅允许 ASCII 字母、数字、点、连字符和下划线。

## 请求结构

```json
{
  "schema_version": 1,
  "action": "submit",
  "session_id": "0123456789abcdef0123456789abcdef",
  "user_input": "bangwozhaowenjian",
  "application": {
    "process_id": 4242,
    "executable": "notepad.exe",
    "window_class": "Notepad",
    "sensitive_input": false
  },
  "session_context": "必要的短上下文",
  "command_id": "",
  "page": 0
}
```

字段规则：

| 字段 | 说明 |
| --- | --- |
| `action` | `submit`、`select`、`page`、`cancel` 或 `poll` |
| `session_id` | 16–64 位十六进制或带连字符标识；同一会话必须原样回传 |
| `user_input` | `submit` 时必填；不包含模式标记 `v` |
| `application` | 最小应用信息；敏感场景或用户关闭权限时字段可能为空 |
| `session_context` | 可为空；已由插件过滤和限长，但 Agent 仍应按敏感数据处理 |
| `command_id` | `select` 时必填，必须来自当前会话最近一次响应 |
| `page` | `page` 时为请求的候选页；不会创建新会话 |

`poll` 是 OwO 内部等待适配的一部分，第三方 Agent 通常不会收到它。官方连接器会把一次第三方调用作为后台任务，Core 的 `poll` 只查询该任务结果。

## 响应结构

```json
{
  "schema_version": 1,
  "session_id": "0123456789abcdef0123456789abcdef",
  "message": "我找到了几个可选操作",
  "commands": [
    {"id": "show-results", "label": "显示搜索结果", "high_risk": false},
    {"id": "delete-files", "label": "删除这些文件", "high_risk": true}
  ],
  "executing_command": "",
  "status": "agent_mode",
  "page": 0,
  "has_more": true,
  "error_code": ""
}
```

`status` 只能是：

- `agent_mode`：等待补充输入或候选选择；
- `thinking`：正在处理；
- `executing`：正在执行已确认的低风险操作，或正在第三方界面等待确认；
- `completed`：任务完成，OwO 结束会话；
- `disconnected`：Agent 不可用；
- `error`：请求失败。

`message` 是只读展示文本，永远不会被 OwO 解析为操作。`commands[]` 是唯一可选择的结构化候选来源。每个 `id` 在当前响应中必须唯一且稳定；`label` 只用于显示。`commands[]` 为空时，OwO 可显示 `executing_command`；为空则省略该行。

高风险操作必须设为 `high_risk: true`。这类候选不会从输入法界面直接回传选择。Agent 可以返回一个低风险命令，用于“打开 Agent 自己的确认界面”；真正的删除、执行、账户、支付、凭据或系统更改必须在该可信界面中单独确认。

## 状态机

```text
普通输入
  └─ 首字符 v → agent_mode（本地预览，仍生成普通拼音候选但不提交）
       ├─ 候选完整消耗请求，或 Enter/Space → submit → thinking
       ├─ commands 非空 → select → thinking/executing
       ├─ 继续输入 → submit（同一 session_id）
       ├─ 翻页 → page（同一 session_id）
       ├─ completed → 结束会话
       └─ 删除 v / Escape / 失焦 / 关闭 → cancel → 结束会话
```

预览中的已选候选仍属于当前 Agent 请求草稿。Backspace 首先撤销光标相邻的已选候选并
恢复其原拼音，然后才逐字删除恢复后的拼音；仅当模式标记 `v` 本身被删除时退出。单独
输入且从未发送请求的 `v` 在 Enter、Space 或切换中英文时按普通字母处理；已经发送过
请求的会话继续遵循状态机，不会把模式标记提交给目标应用。

第三方 Agent 应按 `(session_id, action)` 幂等处理重复请求。收到 `cancel` 后立即停止尚未执行的工作并释放会话资源。若操作已无法撤销，应只报告真实状态，不能伪造取消成功。不同 `session_id` 的状态不得混用；OwO 会拒绝不匹配的响应。

## 隐私和安全实现要求

- 管道只监听本机，并以当前用户 SID 限制访问；不要启用远程命名管道。
- 不要把 `message`、`label` 或用户自然语言当作无需确认的控制代码。
- `submit` 只表示用户发送请求，不等于授权高风险操作。
- 对 `session_context` 按最短保留周期处理；日志默认不得记录原文。
- 当 `sensitive_input` 为 `true` 时，不得请求或推断缺失的应用身份和局部上下文。
- 对命令做独立授权和参数校验，不能把输入法插件的高权限继承为 Agent 的系统权限。
- 响应格式错误、超时、会话不匹配或未知命令 ID时，返回 `error`，不要执行回退命令。

## 最小接入验证

1. 启动只允许当前用户连接的管道服务器。
2. 在 OwO 设置中心安装并明确授权 Agent IPC 连接器，确认端点一致。
3. 输入 `vbangwozhaowenjian`，确认 `v` 未进入拼音解析，后续文本仍有拼音预览且没有上屏。
4. 完整选择拼音候选，验证可自动收到 `submit` 及非空 `session_id`；另行验证 Enter 和 Space 显式确认。
5. 返回一个 `message` 和两个低风险 `commands`，验证点击第二项后收到同一会话的 `select` 和正确 ID。
6. 返回分页结果，验证 `page` 不改变会话 ID。
7. 删除开头 `v` 或按 Escape，验证收到 `cancel`。
8. 在密码框中测试，验证 `session_context`、`executable` 和 `window_class` 为空。
9. 返回错误 JSON、错误会话 ID和超过 65536 字节的数据，验证 OwO 不执行命令并显示简短错误。
10. 返回 `high_risk: true`，验证输入法只提示前往第三方 Agent 确认。
11. 选择部分候选后按 Backspace，验证候选恢复为原拼音且会话不取消；删除开头 `v` 后再验证 `cancel`。
