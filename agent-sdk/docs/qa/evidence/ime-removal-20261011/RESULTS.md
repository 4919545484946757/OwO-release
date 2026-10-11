# Agent 输入法剔除记录（2026-10-11）

## 本次变更

- 删除 agent-sdk/crates/owo-agent-ime 中 14 个已跟踪且删除前无修改的文件。
- 移除 workspace 登记和 Cargo.lock 的对应包；其他包、版本和依赖均未变更。
- 根 README 以 Agent 为入口，SDK README 改为当前 Agent 设计文档入口，旧基线明确标记为历史记录。
- 更新开发约束，清理模拟聊天场景的输入法项目名称。
- 新增 check-agent-boundary.py 和 CI agent-boundary 步骤，检查 workspace、清单、别名与平台依赖、锁文件及可选解析图；负例涵盖重新引入、别名依赖、孤立包和陈旧锁文件。

## 保留与恢复

历史 C++ 源码、构建文件、系统安装、用户数据和已有资料保留。历史入口说明见仓库 docs/archive/owo-input-method/README.md；删除前文件身份和 Git 恢复提交见同目录 agent-adapter-removal.json。没有重写历史，也没有改动其他本地未提交文件。

## 验证

- 离线锁定解析通过：21 个 workspace 成员、335 个 Windows 解析包，默认成员及完整解析图均不含 IME 适配库。
- Windows PowerShell 5.1 实跑 CI agent-boundary 通过；正反例检查通过。
- Rust server、CLI、owo-sim check 通过；owo-sim Clippy -D warnings 通过。
- Workspace fmt --check 通过；桌面定向契约 4/4 通过。
- 当前入口链接、UTF-8、PowerShell BOM 和 git diff --check 通过。
- 历史 C++ 代码未修改；原有本地技术文档哈希未变。

CLI 保留既有 render_final_result_jsonl dead_code 警告。本次没有运行云端模型、全量 Rust 测试、GUI 或重新打包安装程序；历史截图和审查包保留原始快照身份。
