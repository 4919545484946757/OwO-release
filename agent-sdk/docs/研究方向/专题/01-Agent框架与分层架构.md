# Agent 框架与分层架构

本专题依据原始论文、下载项目 README 和部分源码声明整理。设计启示不等于 OwO 已实现；完整资料见 [调研总表](../论文与GitHub项目调研.md)。

## 框架应该替项目解决什么

[ReAct](https://arxiv.org/abs/2210.03629) 把推理与环境动作串成循环；[CoALA](https://arxiv.org/abs/2309.02427) 提供记忆、动作和决策的架构分类。它们适合解释 Agent 的组成，但宿主仍需处理身份、权限、取消、恢复、状态和审计。[Reflexion](https://arxiv.org/abs/2303.11366) 的反思反馈可以改善下一次尝试，却不能充当交付批准。

比较框架时，先问任务状态由谁维护、工具在哪里执行、失败如何表达、会话如何恢复。仅看“支持工具/多 Agent”会遗漏真正的运行边界。

| 层次 | 本项目应保持的职责 | 参考 |
| --- | --- | --- |
| 模型适配 | provider、流式增量、usage、错误、取消 | Rig、Pydantic AI、smolagents |
| Agent loop | 观察、决策、工具请求、回合终态 | ReAct、AutoGen、smolagents |
| 协调与工作流 | DAG、事件、持久化、等待、人机交互 | LangGraph、AgentScope、CrewAI |
| 工具宿主 | 参数验证、授权、执行收据、审计 | Rust MCP SDK 及本项目 ToolHost |
| 产品服务 | 会话、任务、API、薄客户端 | OpenHands、OpenCode、Letta Code |
| 质量与交付 | 变更、独立验证、版本绑定、未解决问题 | SWE-agent、SWE-bench、CAID |

## 可直接开始阅读的源码

[Rig 阅读卡片](../研究资料库/project-notes/0xPlaygrounds--rig/README.md) 收录 AgentBuilder 与真实 run 模块。当前快照的 agent/run.rs 是重导出，分析执行行为应继续进入 crates/rig-agent/src/run/。这说明代码导航必须追踪实际实现。

[LangGraph 阅读卡片](../研究资料库/project-notes/langchain-ai--langgraph/README.md) 含 PregelLoop、BaseCheckpointSaver 和图状态定义；可对照调度与持久化的接口分工。[CrewAI 阅读卡片](../研究资料库/project-notes/crewAIInc--crewAI/README.md) 含 Crew 与 Flow 核心文件，当前目录在 lib/crewai/src/crewai/，本地为选定核心模块包。

Microsoft Agent Framework、Google ADK、Strands harness、Agno、CAMEL 和 AgentScope 用于比较 API 组织和协作原语。它们的目录与能力按固定提交阅读，不用历史品牌名推断当前实现。

## 对 OwO 的建议

先沿共享 Daemon → Server → Core/WorkSwarm → ToolHost 的已有链路整理契约。新增功能接入这条链路，保持一个会话和任务状态权威者。Rust 框架可借鉴 trait、事件和错误模型；Python/TS 框架用于运行语义对照，是否引入依赖另行评估。

审查每层都记录入口、输入、状态变化、取消、错误、恢复和输出。对照结果应形成模块契约与待验证项，再决定拆分；源码文件多或框架 API 简洁，都不能证明实际运行可靠。
