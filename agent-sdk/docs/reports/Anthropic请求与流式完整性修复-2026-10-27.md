# Anthropic 请求与流式完整性修复（2026-10-27）

对应架构审查 F7（可靠观测）、F8（Worker 输入检查点）及 F9（实际请求模型）。本文件记录这批修复和当次验证，不把适配器测试等同于完整产品验收。原始审查报告和此前各轮记录保留。

## 修复结果

1. Anthropic 的普通、带模型覆盖、流式、思考通道及 observed 入口，现在把本次选定模型写进同一请求体；空值及 default 使用配置模型。并发 Worker 不改环境变量或共享配置，也不据模型名字推断另一套端点/凭据。request-id 取实际 HTTP 头，metadata.model 表示已发送的模型名，不证明后端实际服务的别名。
2. 流式 message_delta 的累计用量合并到 message_start 输入用量，缓存创建和读取计入输入；缺字段、类型错误、溢出保持 unknown，不伪造为已知零用量。现有回归验证请求体及 token 计算，不代表云端账单或真实成本。
3. 流内 error 返回失败，错误说明不进入正文回调，配置凭据在返回前脱敏。HTTP 正常 EOF 也不能代替 message_stop；未完成响应不会变成成功的文本或可执行工具调用。工具参数必须属于已存在且尚未结束的块，并收到 content_block_stop；格式错误明确失败。
4. 新的字节级 SSE 分帧器支持 LF、CRLF、CR、多行 data 和开头 BOM，未知事件仍可忽略。完整事件先交付，不因下一个事件的半个 UTF-8 字符而等待。非法 UTF-8/JSON 拒绝，未由空行结束的尾事件不补造。收到 message_stop 后结束读取，不再等待 HTTP 断开。
5. 单事件按归一化行结束计最多 1 MiB；保留正文 16 MiB、思考 8 MiB、单工具参数 4 MiB 上限，另限工具块 1,024 个、工具 id/name/参数累计 16 MiB。超限即时失败，不能只靠最终落盘上限控制内存。
6. 保留现有网关的空闲超时重试识别和连接级超时时间。只有显式配置的备用连接可在策略允许且尚无正文时使用；已有正文则报错，不自动重复输出。

协议依据：[Claude 官方流式说明](https://platform.claude.com/docs/en/build-with-claude/streaming) 的终止事件、错误事件和累计 usage；[WHATWG SSE 规范](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream) 的行分帧与多行 data。兼容端点若遗漏结束事件，现在会明确报告不完整；没有增加静默成功兼容分支。

源码：

- [适配器与状态检查](../../crates/owo-agent-core/src/anthropic.rs)
- [有界 SSE 分帧器](../../crates/owo-agent-core/src/anthropic/sse.rs)
- [请求级模型及用量 HTTP 回归](../../crates/owo-agent-core/tests/anthropic_model_routing_tests.rs)
- [流式完整性 HTTP 回归](../../crates/owo-agent-core/tests/anthropic_stream_integrity_tests.rs)
- [Worker 检查点回归](../../crates/owo-agent-core/src/worker_checkpoint_tests.rs)

## 实际验证

| 检查 | 实际结果 | 范围 |
| --- | --- | --- |
| 最终定向 cargo check | exit=0，9 秒 | Core lib 和两个 HTTP 测试目标 |
| 请求级模型/用量 | 5/5，exit=0 | 实际 wire 模型、各入口、并发图片块、拒绝模型不降级、未知用量 |
| 流式完整性 | 13/13，exit=0 | 真实 HTTP 分片、失败/EOF、CRLF/多行、中文及时交付、工具块与内存边界、终止、超时及恢复；与上行同一命令共 52 秒 |
| Anthropic 单元回归 | 8/8，exit=0，122 秒（含编译/链接） | 请求/工具契约、图片块、文本/参数解析、异常与状态累积 |
| Worker 检查点 | 4/4，exit=0，2 秒 | 当前重新链接的同一 Core 单元二进制，真实 SQLite 持久化 |
| Clippy | exit=0，37 秒，-D warnings | Core lib 和两个 HTTP 测试目标 |
| 静态检查 | exit=0 | 受影响文件 rustfmt --check、git diff --check、Agent/IME 边界 |

最终共 30 项原生测试通过，未运行完整 Core 测试或完整 workspace。

修复前先用当前 Provider 对真实本地 TCP/HTTP 对端运行 8 项新增测试，结果 0 通过、8 失败（exit=101）。已复现错误/EOF 假成功、CRLF/多行解析失败、完整中文事件延迟、非法事件被忽略、超大帧不提前拒绝、终止后仍等待连接关闭。保留失败日志；随后新增的工具内存、块完成、超时与恢复策略用例不冒称也跑过这次修复前验证。

HTTP 夹具使用随机本地端口、实际 chunked body、明确分片及回调通知。中文增量测试在服务端等待前一完整事件的通知后才发下一字符剩余字节，因此不是一次性传入完整字符串的解析测试。错误与上限用例会主动断开本任务的连接；测试夹具随作用域结束回收。

Worker 的 4 项原生回归覆盖：输入与完成工具历史在等待 Provider 时已存入独立 SQLite 连接；丢弃运行 future 后仍可读；保存失败阻止模型请求或成功回报；预先取消保留输入。读取历史不自动重放工具。它们不证明自动恢复、exactly-once 或整个团队的崩溃恢复。

最终验证期间捕获的 **634 项 Rust/manifest/lock/config 输入哈希全部未变**。基线 HEAD 为检查点 191c19265fc8503efc454e7f443522d1ba817a45，新的流式源码在该提交之后修改，目前尚未提交或推送；不能把原检查点哈希当成包含本修复的提交。HTTP、单元和检查点二进制的 SHA-256 及本批源码摘要见证据 JSON。

统一使用 scripts/ci-shared.ps1 的 Invoke-CiCargo strict 档、-j 1、--test-threads=1、30 秒心跳和资源守护；手写 Cargo 前已 Resolve-OwoOrtEnv，仅注入本进程。沿用已预热的 C:/Users/23843/AppData/Local/Temp/owo-audit-iteration-20261025/native-target，明确传入 T 盘真实 Cargo.toml 及 .cargo/config.toml，保留 /NODEFAULTLIB:LIBCMT。构建前门禁检查实际 C 盘空间和内存，未绕过资源限制。没有再次清理缓存、停止用户 Agent 服务或替换运行中的二进制。

持久证据：[目录与复跑说明](../qa/evidence/2026-10-27-anthropic-stream/README.md)、[结果与完整参数](../qa/evidence/2026-10-27-anthropic-stream/result.json)、[输入身份](../qa/evidence/2026-10-27-anthropic-stream/source-identity.json)。目录保留 before / check / native / unit / checkpoint / clippy 日志，不包含可执行体、真实模型凭据或运行数据库。

## 保留的未完成范围

- chat/vision/STT 的独立连接分配、按实际连接和模型绑定的视觉准入，以及媒体保险库适配和真实服务验证仍待完成。传输了 image block 不等于真实模型已理解图片。
- 主 HTTP 回合的新增检查点原生回归、Worker 自动恢复、旧记录迁移、音频/STT/视频输入、团队对话卡片内的输入入口仍未整体验收。
- F5 独立工具节点和有类型数据端口、F7 完整任务—工具—产物因果链与全局游标/未闭合请求诊断、F10 ToolHost—租约—发布集成，以及默认重依赖/评测隔离继续保留。
- 本批没有修改前端、HTTP 路由或 DTO，没有重新验收 Web/Electron、云端模型、麦克风、安装包或发布。失败调用的完整用量和服务实际模型别名不因这次测试自动成为已验证数据。

下一步优先处理 F8/F9 的模型能力与请求前准入，使文本模型、未声明或未验证能力在接收图片前给出准确诊断，再继续其余原审查范围。
