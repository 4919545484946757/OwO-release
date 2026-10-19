# modelcontextprotocol/rust-sdk 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：协议工具
- 参考目标：Rust MCP Client/Server、transport 与取消
- 完整提交：`8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/modelcontextprotocol/rust-sdk)
- [本地源码 ZIP](../../project-snapshots/modelcontextprotocol--rust-sdk-8763a3c3ffb2.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### crates/rmcp/src/service.rs

[本地文件](selected-source/crates/rmcp/src/service.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/service.rs)

源码定位声明：

- `pub trait MaybeSend: Send + Sync {}`
- `pub trait MaybeSend {}`
- `pub trait MaybeSendFuture: Send {}`
- `pub trait MaybeSendFuture {}`
- `pub enum ServiceError {`

### crates/rmcp/src/task_manager.rs

[本地文件](selected-source/crates/rmcp/src/task_manager.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/task_manager.rs)

源码定位声明：

- `pub fn current_timestamp() -> String {`
- `pub struct TaskContext {`
- `pub fn task_id(&self) -> &str {`
- `pub async fn request_input(`
- `pub fn set_status_message(&self, message: impl Into<String>) {`

### crates/rmcp/src/model/task.rs

[本地文件](selected-source/crates/rmcp/src/model/task.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/model/task.rs)

源码定位声明：

- `pub enum TaskStatus {`
- `pub fn is_terminal(&self) -> bool {`
- `pub struct Task {`
- `pub fn new(`
- `pub fn with_status_message(mut self, status_message: impl Into<String>) -> Self {`

### crates/rmcp/src/lib.rs

[本地文件](selected-source/crates/rmcp/src/lib.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/lib.rs)

### crates/rmcp/src/error.rs

[本地文件](selected-source/crates/rmcp/src/error.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/error.rs)

源码定位声明：

- `pub(crate) struct ErrorChain<'a>(pub(crate) &'a (dyn std::error::Error + 'static));`
- `pub enum RmcpError {`
- `pub fn transport_creation<T: 'static>(`

### crates/rmcp/src/model.rs

[本地文件](selected-source/crates/rmcp/src/model.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/model.rs)

源码定位声明：

- `pub fn object(value: serde_json::Value) -> JsonObject {`
- `pub struct EmptyObject {}`
- `pub trait ConstString: Default {`
- `pub struct $name;`
- `pub struct ProtocolVersion(Cow<'static, str>);`

### crates/rmcp/src/handler.rs

[本地文件](selected-source/crates/rmcp/src/handler.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/handler.rs)

### crates/rmcp/src/transport.rs

[本地文件](selected-source/crates/rmcp/src/transport.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp/src/transport.rs)

源码定位声明：

- `pub trait Transport<R>: Send`
- `pub trait IntoTransport<R, E, A>: Send + 'static`
- `pub enum TransportAdapterIdentity {}`
- `pub struct OneshotTransport<R>`
- `pub fn new(`

### conformance/src/bin/client.rs

[本地文件](selected-source/conformance/src/bin/client.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/conformance/src/bin/client.rs)

### conformance/src/bin/server.rs

[本地文件](selected-source/conformance/src/bin/server.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/conformance/src/bin/server.rs)

### crates/rmcp-macros/src/lib.rs

[本地文件](selected-source/crates/rmcp-macros/src/lib.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp-macros/src/lib.rs)

源码定位声明：

- `pub fn tool(attr: TokenStream, input: TokenStream) -> TokenStream {`
- `pub fn tool_router(attr: TokenStream, input: TokenStream) -> TokenStream {`
- `pub fn tool_handler(attr: TokenStream, input: TokenStream) -> TokenStream {`
- `pub fn prompt(attr: TokenStream, input: TokenStream) -> TokenStream {`
- `pub fn prompt_router(attr: TokenStream, input: TokenStream) -> TokenStream {`

### crates/rmcp-macros/src/tool.rs

[本地文件](selected-source/crates/rmcp-macros/src/tool.rs) · [固定提交来源](https://github.com/modelcontextprotocol/rust-sdk/blob/8763a3c3ffb259254d6a16ec1eb4ae9173d5d57d/crates/rmcp-macros/src/tool.rs)

源码定位声明：

- `pub struct ToolAttribute {`
- `pub struct ResolvedToolAttribute {`
- `pub fn into_fn(self, fn_ident: Ident) -> syn::Result<ImplItemFn> {`
- `pub fn #fn_ident() -> rmcp::model::Tool {`
- `pub struct ToolAnnotationsAttribute {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
