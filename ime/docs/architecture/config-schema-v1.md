# OwO 内部配置 Schema v1

- 状态：已由配置 Schema v3 替代；读取器继续兼容迁移
- 编码：严格 UTF-8 文本、禁止 BOM
- 大小上限：16 KiB

## 稳定序列化

```text
schema_version=1
candidate_page_size=5
user_learning_enabled=true
model_ranking_enabled=false
model_timeout_ms=50
```

所有字段必须且只能出现一次。空行、注释、空白修饰、未知字段、未知版本和非 `true`/`false` 布尔值均失败封闭。序列化顺序固定，文件以一个换行结束。

## 字段

| 字段 | 类型 | 默认值 | 范围与语义 |
| --- | --- | ---: | --- |
| `schema_version` | uint32 | 1 | 只接受 1；未来版本必须提供显式迁移 |
| `candidate_page_size` | uint32 | 5 | 1～9；尚未接入 TSF，接入时不得改变数字选词可达性 |
| `user_learning_enabled` | bool | true | 控制后续 Core 用户词频写入；关闭不删除既有数据 |
| `model_ranking_enabled` | bool | false | 默认关闭；只有可分发真实模型通过质量门禁后才允许产品默认值变化 |
| `model_timeout_ms` | uint32 | 50 | 5～500；只约束模型增量预算，不阻塞基础候选 |

首版不接受路径、插件权限、网络地址、任意命令、快捷键或 UI 主题字符串，避免把尚未冻结的安全与交互决策写入稳定格式。

## 存储与恢复语义

1. 新配置先写入同目录 `.tmp`；Windows 使用 `FlushFileBuffers` 确保持久化请求完成。
2. 仅当现有主文件能够通过完整 Schema 校验时，才覆盖 `.bak`；损坏主文件不得污染有效备份。
3. 使用 `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` 原子替换主文件。
4. 首次加载：主文件有效则使用主文件；否则尝试备份；两者都不可用则使用编译期默认值并返回明确诊断。
5. 热加载：只读取主文件。解析失败时返回失败，保持上一份内存快照和 generation，不自动切换备份或默认值。
6. 内容真正变化时 generation 单调加一；重新加载相同内容不加一。

配置存储已接入 Core Service；TSF 仍不直接读取配置文件。Core 默认监听 `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf`，可用 `--config <path>` 显式覆盖；`--no-config` 只用于隔离测试与诊断。Core 在每个 IPC 请求边界读取不可变原子快照：`user_learning_enabled` 控制后续学习写入但不删除或屏蔽已有数据，`model_ranking_enabled` 控制新模型请求及结果发布，`model_timeout_ms` 同时约束模型请求预算和管道等待预算。无效热加载继续使用上一份有效快照。

`candidate_page_size` 在 Core IPC 请求边界应用，取值范围 1～9 与 TSF 数字选词键范围一致。改变页大小后，后续翻页请求使用新值；TSF 当前页不会被后台配置线程直接改写，并继续以 Core 返回的 `page`/`has_more` 校验响应。

## 后台监控与快照发布

`ConfigMonitor` 在调用线程同步完成首次加载并建立文件状态基线，随后才返回，避免启动后首次写入被漏记。后台线程按 10～5000 ms 的受限间隔检查主文件元数据；默认 250 ms。停止令牌可立即唤醒等待，不需要等待完整轮询周期。

有效且内容变化的配置以 `shared_ptr<const AppConfig>` 原子发布，并同步发布单调 generation。读取者无需持有存储锁，也不会看到半更新对象。`wait_for_generation` 支持超时和停止令牌，供 Core Service 的非按键线程等待变更。

无效、缺失或读取失败的热加载只更新可诊断状态，不增加 generation、不发布默认值，也不替换上一有效快照。监控停止后不再读取文件或发布变化。

## 诊断 CLI

Windows `owo_config_shell <path>` 提供以下非 UI 诊断入口：

- `show`：显示严格解析后的主文件、备份恢复值或默认值，并把降级原因写入 stderr；
- `set <field> <value>`：读取当前有效快照、严格修改一个已知字段并原子保存；
- `watch <timeout-ms>`：完成监控握手后等待一次 generation 变化；
- `repair`：将有效主文件、恢复的备份或最终默认值重新原子写入主文件。

无效字段和值在写入前失败，不改变主文件。该 CLI 是设置中心接入前的诊断工具，不是稳定公共命令行接口。
