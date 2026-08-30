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

单个 JSON 最大 262144 字节，单次操作默认超时 10 秒，可配置为 500–30000 毫秒。服务端必须处理客户端超时或断开，不能把半包当作完整请求。

当前发送协议为 3，连接器和参考解码器仍接受协议 v2。JSON 对象使用严格字段集合，不接受重复或未知字段。字符串可使用 UTF-8 字符或标准 JSON `\uXXXX` 转义；命令、请求和上下文 ID 仅允许 ASCII 字母、数字、点、连字符或下划线。第三方 Agent 应读取 `capabilities` 决定可用能力，不得仅根据版本号猜测字段语义。

机器可校验定义见 `agent-ipc-v3.schema.json`。发布 SDK 会把该文件复制到 `docs/`。

## 请求结构

```json
{
  "schema_version": 3,
  "action": "submit",
  "session_id": "0123456789abcdef0123456789abcdef",
  "request_id": "req-120",
  "parent_request_id": "",
  "idempotency_key": "req-120",
  "capabilities": ["protocol.negotiation", "input.structured", "context.entries", "commands.risk", "tasks.slots", "tasks.incremental"],
  "protocol_min": 2,
  "protocol_max": 3,
  "required_features": ["protocol.negotiation", "input.structured", "commands.risk", "tasks.slots"],
  "user_input": "bangwozhaowenjian",
  "input": {
    "raw_pinyin": "bangwozhaowenjian",
    "segmented_pinyin": "bang'wo'zhao'wen'jian",
    "selected_text": "帮我找文件",
    "pending_pinyin": "",
    "natural_language": "帮我找文件",
    "input_mode": "agent",
    "correction_enabled": true
  },
  "application": {
    "process_id": 4242,
    "thread_id": 8120,
    "executable": "notepad.exe",
    "window_class": "Notepad",
    "focus_window_class": "RichEditD2DPT",
    "context_id": "ctx-current-01234567",
    "sensitive_input": false
  },
  "session_context": "当前输入位置的近期上下文",
  "context_entries": [
    {
      "context_id": "ctx-editor-89abcdef",
      "application": {
        "process_id": 3000,
        "thread_id": 3001,
        "executable": "editor.exe",
        "window_class": "EditorWindow",
        "focus_window_class": "TextBox",
        "context_id": "ctx-editor-89abcdef",
        "sensitive_input": false
      },
      "text": "上一输入位置由 OwO 上屏的有限片段",
      "sequence": 17,
      "context_type": "committed_text",
      "source": "owo_commit",
      "created_at_ms": 1787830000000,
      "relevance_milli": 800,
      "is_current": false,
      "privacy": {"filtered": true, "redacted": false, "truncated": false}
    }
  ],
  "command_id": "",
  "page": 0,
  "task_revision": 0,
  "slot_updates": []
}
```

字段规则：

| 字段 | 说明 |
| --- | --- |
| `action` | `submit`、`select`、`page`、`cancel` 或 `poll` |
| `session_id` | 16–64 位十六进制或带连字符标识；同一会话必须原样回传 |
| `request_id` | 每次操作唯一；响应必须原样回传 |
| `parent_request_id` | 当前操作直接关联的上一请求，可为空 |
| `idempotency_key` | 重试时保持不变；Agent 必须返回已缓存结果而不是重复执行 |
| `capabilities` | OwO 此次请求支持的能力；Agent 在响应中返回实际支持的交集 |
| `protocol_min/max` | OwO 可解析的协议范围；当前请求仍以 `schema_version` 的结构发送 |
| `required_features` | Agent 响应必须声明支持的能力；缺失时连接器拒绝继续执行 |
| `user_input` | `submit` 时必填；不包含模式标记 `v` |
| `input` | 原始/分割拼音、已选文字、剩余拼音和最终自然语言的结构化视图 |
| `application` | 当前输入位置；包括进程/线程、程序名、顶层/焦点窗口类和不透明 `context_id`；不含窗口标题 |
| `session_context` | 当前输入位置的上下文，可为空；其标识使用顶层 `application.context_id`，仍应按敏感数据处理 |
| `context_entries` | 按旧到新排列的其他应用或输入位置上下文；带来源、时间、相关度和过滤状态 |
| `command_id` | `select` 时必填，必须来自当前会话最近一次响应 |
| `page` | `page` 时为请求的候选页；不会创建新会话 |
| `task_revision` | `select` 所依据的任务草稿版本；版本不一致必须拒绝补丁 |
| `slot_updates` | 本次候选选择造成的最小槽位变更；普通提交、翻页和轮询为空数组 |

`poll` 是 OwO 内部等待适配的一部分，第三方 Agent 通常不会收到它。官方连接器会把一次第三方调用作为后台任务，Core 的 `poll` 只查询该任务结果。

## 响应结构

```json
{
  "schema_version": 3,
  "session_id": "0123456789abcdef0123456789abcdef",
  "request_id": "req-120",
  "message": "我找到了几个可选操作",
  "commands": [
    {
      "id": "show-results", "label": "显示搜索结果", "high_risk": false,
      "description": "显示已经找到的文件", "category": "file.search",
      "risk_level": "low", "requires_confirmation": false,
      "preview": "不会修改文件", "enabled": true, "disabled_reason": "",
      "task_revision": 0, "slot_updates": [], "commit_task": false
    },
    {
      "id": "delete-files", "label": "删除这些文件", "high_risk": true,
      "description": "删除选定文件", "category": "file.delete",
      "risk_level": "high", "requires_confirmation": true,
      "preview": "必须在 Agent 可信界面确认", "enabled": true, "disabled_reason": "",
      "task_revision": 0, "slot_updates": [], "commit_task": false
    }
  ],
  "executing_command": "",
  "status": "agent_mode",
  "page": 0,
  "has_more": true,
  "error_code": "",
  "state_revision": 4,
  "progress": 65,
  "retry_after_ms": 150,
  "can_cancel": true,
  "can_continue_input": true,
  "expires_at_ms": 1787830060000,
  "error_message": "",
  "retryable": false,
  "capabilities": ["protocol.negotiation", "input.structured", "context.entries", "commands.risk", "tasks.slots", "tasks.incremental"],
  "task": {"intent": "", "intent_ranges": [], "slots": [], "unconsumed_ranges": [], "revision": 0}
}
```

`status` 只能是：

- `connecting` / `submitting`：正在连接或发送；
- `agent_mode`：等待补充输入或候选选择；
- `waiting_for_user`：等待用户继续输入；
- `thinking`：正在处理；
- `executing`：正在执行已确认的低风险操作，或正在第三方界面等待确认；
- `waiting_for_confirmation`：等待第三方可信界面确认；
- `cancelling` / `cancelled`：正在取消或已经取消；
- `completed`：任务完成，OwO 结束会话；
- `disconnected`：Agent 不可用；
- `timeout`：请求超时且没有执行未知状态的回退命令；
- `error`：请求失败。

`message` 是只读展示文本，永远不会被 OwO 解析为操作。`commands[]` 是唯一可选择的结构化候选来源。每个 `id` 在当前响应中必须唯一且稳定；`label` 只用于显示。`commands[]` 为空时，OwO 可显示 `executing_command`；为空则省略该行。

高风险操作必须设为 `high_risk: true`。这类候选不会从输入法界面直接回传选择。Agent 可以返回一个低风险命令，用于“打开 Agent 自己的确认界面”；真正的删除、执行、账户、支付、凭据或系统更改必须在该可信界面中单独确认。

## 可逐段消耗拼音的任务候选

Agent 可声明 `tasks.slots` 和 `tasks.incremental`，把候选表达为任务树上的补丁。所有范围均相对于去掉开头 `v` 后的原始 ASCII 拼音，采用从 0 开始、左闭右开 `[start, end)`；范围必须有效、互不重叠且不得超出原拼音。

例如 `vsanzhongtixingwokaihui` 可返回：

```json
{
  "task": {
    "intent": "create_reminder",
    "intent_ranges": [{"start": 8, "end": 16}],
    "slots": [
      {"id": "time", "label": "时间", "value": "三点钟", "source_ranges": [{"start": 0, "end": 8}], "locked": false, "confidence_milli": 650},
      {"id": "content", "label": "事项", "value": "开会", "source_ranges": [{"start": 16, "end": 22}], "locked": true, "confidence_milli": 980}
    ],
    "unconsumed_ranges": [],
    "revision": 1
  },
  "command": {
    "id": "time-tomorrow-3",
    "label": "明天3点提醒我开会",
    "task_revision": 1,
    "slot_updates": [{"slot_id": "time", "value": "明天3点", "consumed_ranges": [{"start": 0, "end": 8}], "lock": true}],
    "commit_task": false
  }
}
```

选择该候选时，OwO 只发送 `command_id`、`task_revision` 和 `slot_updates`。TSF 会先检查版本与锁定状态，在本地更新任务草稿，不把候选文字上屏；第三方 Agent 只需重算受影响槽位。已锁定槽位不得被不同值覆盖，`unconsumed_ranges` 对应的拼音继续留在组合态，可供普通输入或下一项任务使用。只有 `commit_task: true` 才表示候选在请求提交完整任务，但是否执行仍受风险确认机制约束。

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

第三方 Agent 应按 `idempotency_key` 幂等处理重复请求，并以单调递增的 `state_revision` 防止旧状态覆盖新状态。收到 `cancel` 后返回 `cancelling` 或 `cancelled`；若操作已无法撤销，应只报告真实状态，不能伪造取消成功。不同 `session_id`、`request_id` 的状态不得混用，OwO 会拒绝不匹配的响应。

## 隐私和安全实现要求

- 管道只监听本机，并以当前用户 SID 限制访问；不要启用远程命名管道。
- 不要把 `message`、`label` 或用户自然语言当作无需确认的控制代码。
- `submit` 只表示用户发送请求，不等于授权高风险操作。
- 对 `session_context` 按最短保留周期处理；日志默认不得记录原文。
- 对 `context_entries` 同样按最短保留周期处理；不要用 `context_id` 构建持久用户画像。
- `sequence` 只表示 Core 内存记录中的相对先后，不是时间戳；应用或输入位置切换时以每段携带的应用信息为准。
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
8. 在两个应用及同一应用的两个输入位置分别用 OwO 上屏文字，再发送请求；验证 `context_entries` 保留分段顺序且每段应用标识正确。
9. 在密码框中测试，验证 `session_context`、`context_entries`、`executable` 和窗口类为空。
10. 返回错误 JSON、错误会话 ID和超过 262144 字节的数据，验证 OwO 不执行命令并显示简短错误。
11. 返回 `high_risk: true`，验证输入法只提示前往第三方 Agent 确认。
12. 选择部分候选后按 Backspace，验证候选恢复为原拼音且会话不取消；删除开头 `v` 后再验证 `cancel`。
13. 使用同一 `idempotency_key` 重发请求，验证 Agent 返回缓存状态且不重复执行操作。
14. 返回更小的 `state_revision` 或错误 `request_id`，验证旧状态不会覆盖当前候选框。
15. 返回 `risk_level=high` 但未同时设置 `high_risk=true` 和 `requires_confirmation=true`，验证响应被拒绝。
