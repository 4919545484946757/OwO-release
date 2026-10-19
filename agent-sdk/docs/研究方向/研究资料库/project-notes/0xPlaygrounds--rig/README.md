# 0xPlaygrounds/rig 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Rust框架
- 参考目标：Rust trait、provider、工具及组合
- 完整提交：`9e7cb6000355b7ae1a89c913bf98b8a6f17661ee`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/0xPlaygrounds/rig)
- [本地源码 ZIP](../../project-snapshots/0xPlaygrounds--rig-9e7cb6000355.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### crates/rig-agent/src/agent/builder.rs

[本地文件](selected-source/crates/rig-agent/src/agent/builder.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/crates/rig-agent/src/agent/builder.rs)

源码定位声明：

- `pub struct NoToolConfig;`
- `pub struct WithToolServerHandle {`
- `pub struct WithBuilderTools(ToolServer);`
- `pub struct AgentBuilder<ToolState = NoToolConfig> {`
- `pub fn name(mut self, name: impl Into<String>) -> Self {`

### crates/rig-agent/src/run/mod.rs

[本地文件](selected-source/crates/rig-agent/src/run/mod.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/crates/rig-agent/src/run/mod.rs)

源码定位声明：

- `pub enum AgentRunStep {`
- `pub struct ModelTurn {`
- `pub fn from_response(resp: &CompletionResponse, prepared: &prepare::PreparedRequest) -> Self {`
- `pub fn from_policy(resp: &CompletionResponse, policy: TurnPolicy) -> Self {`
- `pub fn new(`

### crates/rig-agent/src/run/policy.rs

[本地文件](selected-source/crates/rig-agent/src/run/policy.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/crates/rig-agent/src/run/policy.rs)

源码定位声明：

- `pub enum InvalidToolCallReason {`
- `pub fn malformed_arguments(raw: &str) -> Self {`
- `pub(crate) fn arguments_parse_error(raw: &str) -> String {`
- `pub struct InvalidToolCallContext {`
- `pub struct TurnPolicy {`

### crates/rig-agent/src/run/calls.rs

[本地文件](selected-source/crates/rig-agent/src/run/calls.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/crates/rig-agent/src/run/calls.rs)

源码定位声明：

- `pub enum PendingToolCall {`
- `pub struct ExecCall {`
- `pub fn id(&self) -> &CallId {`
- `pub fn name(&self) -> &ToolName {`
- `pub fn tool_call(&self) -> &ToolCall {`

### crates/rig-agent/src/agent/runner.rs

[本地文件](selected-source/crates/rig-agent/src/agent/runner.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/crates/rig-agent/src/agent/runner.rs)

源码定位声明：

- `pub struct AgentRunner<O = ()> {`
- `pub(crate) enum RunOrigin {`
- `pub(crate) fn from_agent(agent: &Agent, prompt: impl Into<Message>) -> Self {`
- `pub(crate) fn resuming(agent: &Agent, run: AgentRun) -> Self {`
- `pub(crate) fn output_tool(`

### xtask/src/bevy.rs

[本地文件](selected-source/xtask/src/bevy.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/bevy.rs)

源码定位声明：

- `pub(crate) enum Error {`
- `pub(crate) fn check(metadata: &Value) -> Result<(), Error> {`

### xtask/src/main.rs

[本地文件](selected-source/xtask/src/main.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/main.rs)

### xtask/src/wires.rs

[本地文件](selected-source/xtask/src/wires.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/wires.rs)

源码定位声明：

- `pub(crate) fn check(workspace: &Path) -> Result<(), String> {`

### xtask/src/verify.rs

[本地文件](selected-source/xtask/src/verify.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/verify.rs)

源码定位声明：

- `pub(crate) enum Error {`
- `pub(crate) fn run(root: &Path, args: Vec<String>) -> Result<()> {`

### xtask/src/catalog.rs

[本地文件](selected-source/xtask/src/catalog.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/catalog.rs)

源码定位声明：

- `pub(crate) fn run(root: &Path, args: Vec<String>) -> Result<(), String> {`
- `pub(crate) fn rfc3339(time: SystemTime) -> Result<String, String> {`
- `pub(crate) fn is_rfc3339_utc(text: &str) -> bool {`
- `pub(crate) fn generate(models_dev: &Value, review: &Value) -> Result<Rows, String> {`
- `pub(crate) fn render(rows: &Rows) -> String {`

### xtask/src/support.rs

[本地文件](selected-source/xtask/src/support.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/support.rs)

源码定位声明：

- `pub(crate) fn files_under(dir: &Path, extension: Option<&str>) -> Result<Vec<PathBuf>, String> {`
- `pub(crate) fn output(dir: &Path, program: &str, args: &[&str]) -> Result<String, String> {`

### xtask/src/cassette.rs

[本地文件](selected-source/xtask/src/cassette.rs) · [固定提交来源](https://github.com/0xPlaygrounds/rig/blob/9e7cb6000355b7ae1a89c913bf98b8a6f17661ee/xtask/src/cassette.rs)

源码定位声明：

- `pub(crate) fn attempt_root(root: &Path) -> PathBuf {`
- `pub(crate) fn run(root: &Path, args: Vec<String>) -> Result<(), String> {`
- `pub(crate) fn cleanup(root: &Path, args: &[String]) -> Result<(), String> {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
