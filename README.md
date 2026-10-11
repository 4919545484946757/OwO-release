# OwO Agent

面向本地项目的人机协作 Agent 工作台。当前开发入口是 [agent-sdk](agent-sdk/README.md)，包含 Rust 核心、共享 Daemon、CLI/TUI 和 Electron 桌面客户端。

## 当前入口

- [Agent 源码与使用说明](agent-sdk/README.md)
- [主开发技术文档](builGoal/主开发技术文档-Agent-SDK-v1.md)
- [后续任务与验收约束](builGoal/Agent-SDK-后续任务实施指南-2026-09-18.md)
- [桌面六项优化与验证记录](agent-sdk/docs/review/desktop-six-items-implementation-20261011.md)
- [开发约束](AGENTS.md)

## 主要能力

Agent 支持会话与流式输出、文件和 Git 工具、权限审批与审计、任务取消与恢复，以及多 Agent 团队和产物评审。Electron 与终端共用 Daemon；桌面提供项目首页、会话历史、产物与证据、自定义模型配置和设置面板。

功能成熟度、配置状态和当次可用性以当前源码、诊断和验证记录为准。历史测试与截图不代替新版本的桌面或发布验收。

## 桌面运行

已有依赖和当前 debug 核心时，可双击 `agent-sdk/desktop/electron/start-current.cmd`。完整安装、模型环境配置和构建方式见 [Agent README](agent-sdk/README.md)；Rust 构建须遵守 [资源策略](AGENTS.md)。

## 项目边界

`agent-sdk/` 是活动 Agent 工程。2026-10-11 已移除旧输入法 IPC 适配库、历史 C++ 工程及相关构建、安装与归档文件；输入法源码由独立 OwO 主仓库维护，此工作树不保留输入法备份。CI 检查 Agent 依赖与仓库根目录边界。

用户级输入法安装和历史本地数据不受这次源码整理影响。仓库历史保留，未重写 Git 历史。

## 许可

源码采用 GPL-3.0-only。第三方组件许可按对应依赖与发行资料核对。
