# Agent 论文与 GitHub 项目调研

整理日期：2026-10-13。本地收录 48 篇论文 PDF 和 41 个项目快照，其中 40 个含源码、1 个为 Letta 迁移说明。前一批为 9 篇论文和 4 个源码包；本轮新增 39 篇论文及 36 个源码包，并保留迁移说明。

本轮完成主题检索、原始元数据与 PDF 结构核对、前三页提取和部分源码定位；未逐篇全文精读、未运行下载项目或复现实验。[检索与限制](检索方法与来源记录.md)、[本地资料库](研究资料库/README.md)。

## 按主题阅读

[框架](专题/01-Agent框架与分层架构.md) · [持久化工作流](专题/02-工作流与持久化执行.md) · [并行开发](专题/03-多Agent并行开发与协作.md) · [自动优化](专题/04-自动化工作流与自优化.md) · [记忆](专题/05-记忆与共享上下文.md) · [评测](专题/06-评测与交付验收.md) · [安全与协议](专题/07-权限注入防御与协议.md) · [发展方向](专题/08-发展方向与研究问题.md)

## 论文：基础框架

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| ReAct: Synergizing Reasoning and Acting in Language Models / 2210.03629 | 推理与工具动作交替；对照 Agent loop 与观察收据。 | 基础任务结果不能代表复杂工程可靠性。 | [PDF](研究资料库/papers/2210.03629.pdf) · [阅读卡片](研究资料库/paper-notes/2210.03629.md) · [原文](https://arxiv.org/abs/2210.03629) |
| Toolformer: Language Models Can Teach Themselves to Use Tools / 2302.04761 | 学习何时调用工具；参考工具选择及调用成本。 | 训练方法与现有不训练模型的宿主路线不同。 | [PDF](研究资料库/papers/2302.04761.pdf) · [阅读卡片](研究资料库/paper-notes/2302.04761.md) · [原文](https://arxiv.org/abs/2302.04761) |
| Reflexion: Language Agents with Verbal Reinforcement Learning / 2303.11366 | 把失败反馈转为反思记忆；区分反思建议与宿主验收。 | 反思不能代替独立测试，需防止错误记忆累积。 | [PDF](研究资料库/papers/2303.11366.pdf) · [阅读卡片](研究资料库/paper-notes/2303.11366.md) · [原文](https://arxiv.org/abs/2303.11366) |
| Cognitive Architectures for Language Agents / 2309.02427 | 按记忆、决策与动作划分架构；用于职责边界。 | 概念分类框架，不是现成生产运行时。 | [PDF](研究资料库/papers/2309.02427.pdf) · [阅读卡片](研究资料库/paper-notes/2309.02427.md) · [原文](https://arxiv.org/abs/2309.02427) |

## 论文：多Agent协作

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| CAMEL: Communicative Agents for "Mind" Exploration of Large Language Model Society / 2303.17760 | 角色扮演交互与通信协议的早期研究。 | 协作对话质量不直接等于真实工程产物质量。 | [PDF](研究资料库/papers/2303.17760.pdf) · [阅读卡片](研究资料库/paper-notes/2303.17760.md) · [原文](https://arxiv.org/abs/2303.17760) |
| ChatDev: Communicative Agents for Software Development / 2307.07924 | 设计、编码、测试的会话式软件开发链。 | 小型任务与角色对话不能覆盖大型代码库依赖。 | [PDF](研究资料库/papers/2307.07924.pdf) · [阅读卡片](研究资料库/paper-notes/2307.07924.md) · [原文](https://arxiv.org/abs/2307.07924) |
| MetaGPT: Meta Programming for A Multi-Agent Collaborative Framework / 2308.00352 | SOP 驱动角色协作与结构化交付。 | 角色分工仍需宿主验证和集成评估。 | [PDF](研究资料库/papers/2308.00352.pdf) · [阅读卡片](研究资料库/paper-notes/2308.00352.md) · [原文](https://arxiv.org/abs/2308.00352) |
| AutoGen: Enabling Next-Gen LLM Applications via Multi-Agent Conversation / 2308.08155 | 多 Agent 会话、工具和可编程控制流。 | 历史论文与当前框架版本不同。 | [PDF](研究资料库/papers/2308.08155.pdf) · [阅读卡片](研究资料库/paper-notes/2308.08155.md) · [原文](https://arxiv.org/abs/2308.08155) |
| AgentVerse: Facilitating Multi-Agent Collaboration and Exploring Emergent Behaviors / 2308.10848 | 动态角色与多 Agent 协作；对照角色编排边界。 | 任务域与模型特定，需同预算重新评测。 | [PDF](研究资料库/papers/2308.10848.pdf) · [阅读卡片](研究资料库/paper-notes/2308.10848.md) · [原文](https://arxiv.org/abs/2308.10848) |
| A Dynamic LLM-Powered Agent Network for Task-Oriented Agent Collaboration / 2310.02170 | 动态 Agent 网络与贡献选择；研究并发收益策略。 | 需计入选择、通信与额外模型调用的成本。 | [PDF](研究资料库/papers/2310.02170.pdf) · [阅读卡片](研究资料库/paper-notes/2310.02170.md) · [原文](https://arxiv.org/abs/2310.02170) |
| AgentScope: A Flexible yet Robust Multi-Agent Platform / 2402.14034 | 多 Agent 开发平台及分布式消息协作。 | 框架能力需按当前版本与实际运行核对。 | [PDF](研究资料库/papers/2402.14034.pdf) · [阅读卡片](研究资料库/paper-notes/2402.14034.md) · [原文](https://arxiv.org/abs/2402.14034) |
| Why Do Multi-Agent LLM Systems Fail? / 2503.13657 | 失败分类与协调、验证、终止边界 | 案例归因不能代替因果实验 | [PDF](研究资料库/papers/2503.13657.pdf) · [阅读卡片](研究资料库/paper-notes/2503.13657.md) · [原文](https://arxiv.org/abs/2503.13657) |
| Towards a Science of Scaling Agent Systems / 2512.08296 | 并行收益、拓扑和成本条件 | 不能为本项目性能背书 | [PDF](研究资料库/papers/2512.08296.pdf) · [阅读卡片](研究资料库/paper-notes/2512.08296.md) · [原文](https://arxiv.org/abs/2512.08296) |
| Effective Strategies for Asynchronous Software Engineering Agents / 2603.21489 | CAID：依赖、隔离 worktree、异步执行与测试集成 | 模型、预算与任务特定 | [PDF](研究资料库/papers/2603.21489.pdf) · [阅读卡片](研究资料库/paper-notes/2603.21489.md) · [原文](https://arxiv.org/abs/2603.21489) |
| Multi-agent Collaboration with State Management / 2605.20563 | STORM：版本化读写与共享状态冲突 | 语义集成仍需独立验证 | [PDF](研究资料库/papers/2605.20563.pdf) · [阅读卡片](研究资料库/paper-notes/2605.20563.md) · [原文](https://arxiv.org/abs/2605.20563) |
| AgentRoom: Concurrent Multi-Agent Coding in a CRDT-Backed Shared Workspace / 2608.23740 | CRDT、claim、状态与广播 | 文本收敛不等于代码语义正确 | [PDF](研究资料库/papers/2608.23740.pdf) · [阅读卡片](研究资料库/paper-notes/2608.23740.md) · [原文](https://arxiv.org/abs/2608.23740) |
| Coordinating Agents on a Shared Git Workspace: An Empirical Study of Action-Space Observation for Agentic Software Engineering / LIPIcs.ESEM.2026.18 | PASC：共享 Git 工作区与队友动作可见性 | 两 Agent 特定任务不能外推大团队 | [PDF](研究资料库/papers/LIPIcs.ESEM.2026.18.pdf) · [阅读卡片](研究资料库/paper-notes/LIPIcs.ESEM.2026.18.md) · [原文](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ESEM.2026.18) |

## 论文：记忆上下文

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| Generative Agents: Interactive Simulacra of Human Behavior / 2304.03442 | 观察、记忆检索、反思与规划的组合。 | 模拟社会环境与软件工程交付的目标不同。 | [PDF](研究资料库/papers/2304.03442.pdf) · [阅读卡片](研究资料库/paper-notes/2304.03442.md) · [原文](https://arxiv.org/abs/2304.03442) |
| MemGPT: Towards LLMs as Operating Systems / 2310.08560 | 分层记忆与上下文管理；对照会话压缩和外部记忆。 | 长上下文管理并非事实正确性或权限保证。 | [PDF](研究资料库/papers/2310.08560.pdf) · [阅读卡片](研究资料库/paper-notes/2310.08560.md) · [原文](https://arxiv.org/abs/2310.08560) |
| A-MEM: Agentic Memory for LLM Agents / 2502.12110 | 动态组织、关联和更新记忆。 | 检索质量需结合过期、污染与删除语义评估。 | [PDF](研究资料库/papers/2502.12110.pdf) · [阅读卡片](研究资料库/paper-notes/2502.12110.md) · [原文](https://arxiv.org/abs/2502.12110) |
| Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory / 2504.19413 | 生产导向长期记忆提取与检索。 | 作者实现与基准设置特定，需复核成本及隐私边界。 | [PDF](研究资料库/papers/2504.19413.pdf) · [阅读卡片](研究资料库/paper-notes/2504.19413.md) · [原文](https://arxiv.org/abs/2504.19413) |
| Anatomy of Agentic Memory: Taxonomy and Empirical Analysis of Evaluation and System Limitations / 2602.19320 | 记忆结构、评估指标及系统限制的近期梳理。 | 分类与实验覆盖边界需阅读全文确认。 | [PDF](研究资料库/papers/2602.19320.pdf) · [阅读卡片](研究资料库/paper-notes/2602.19320.md) · [原文](https://arxiv.org/abs/2602.19320) |

## 论文：自动化环境

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| WebArena: A Realistic Web Environment for Building Autonomous Agents / 2307.13854 | 真实感 Web 任务与可执行验收。 | 浏览器环境结果不能覆盖本地命令和文件任务。 | [PDF](研究资料库/papers/2307.13854.pdf) · [阅读卡片](研究资料库/paper-notes/2307.13854.md) · [原文](https://arxiv.org/abs/2307.13854) |
| OSWorld: Benchmarking Multimodal Agents for Open-Ended Tasks in Real Computer Environments / 2404.07972 | 真实计算机任务与执行式评估。 | 环境、操作系统和初始状态影响结果。 | [PDF](研究资料库/papers/2404.07972.pdf) · [阅读卡片](研究资料库/paper-notes/2404.07972.md) · [原文](https://arxiv.org/abs/2404.07972) |
| $\tau$-bench: A Benchmark for Tool-Agent-User Interaction in Real-World Domains / 2406.12045 | 工具、用户和领域策略交互。 | 领域 API 任务不能代表全部工程任务。 | [PDF](研究资料库/papers/2406.12045.pdf) · [阅读卡片](研究资料库/paper-notes/2406.12045.md) · [原文](https://arxiv.org/abs/2406.12045) |
| $\tau^2$-Bench: Evaluating Conversational Agents in a Dual-Control Environment / 2506.07982 | 双重控制环境中的协作与工具交互。 | 对话服务任务不能直接等价并行软件开发。 | [PDF](研究资料库/papers/2506.07982.pdf) · [阅读卡片](研究资料库/paper-notes/2506.07982.md) · [原文](https://arxiv.org/abs/2506.07982) |

## 论文：评测基准

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| AgentBench: Evaluating LLMs as Agents / 2308.03688 | 多环境 Agent 评测与失败分析。 | 历史环境与现代宿主不同，需固定版本及配置。 | [PDF](研究资料库/papers/2308.03688.pdf) · [阅读卡片](研究资料库/paper-notes/2308.03688.md) · [原文](https://arxiv.org/abs/2308.03688) |
| SWE-bench: Can Language Models Resolve Real-World GitHub Issues? / 2310.06770 | 真实仓库 Issue 与可执行测试；用于产物验收基线。 | 数据泄漏、环境失败和测试覆盖要单独记录。 | [PDF](研究资料库/papers/2310.06770.pdf) · [阅读卡片](研究资料库/paper-notes/2310.06770.md) · [原文](https://arxiv.org/abs/2310.06770) |
| AgentBoard: An Analytical Evaluation Board of Multi-turn LLM Agents / 2401.13178 | 多回合 Agent 的过程性进度评估。 | 进度指标不能替代最终验收。 | [PDF](研究资料库/papers/2401.13178.pdf) · [阅读卡片](研究资料库/paper-notes/2401.13178.md) · [原文](https://arxiv.org/abs/2401.13178) |
| Evaluating Agentic AI in the Wild: Failure Modes, Drift Patterns, and a Production Evaluation Framework / 2605.01604 | 关注真实运行中的失败、漂移与生产评估。 | 近期预印本；框架建议应经过项目数据验证。 | [PDF](研究资料库/papers/2605.01604.pdf) · [阅读卡片](研究资料库/paper-notes/2605.01604.md) · [原文](https://arxiv.org/abs/2605.01604) |
| AsynCodeBench: Benchmarking Collaboration of Asynchronous Multi-Agent Systems in Software Engineering / 2609.32662 | 依赖检查器、依赖解决比例与时间 | 评测域有限 | [PDF](研究资料库/papers/2609.32662.pdf) · [阅读卡片](研究资料库/paper-notes/2609.32662.md) · [原文](https://arxiv.org/abs/2609.32662) |

## 论文：综述方向

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| A Survey on Large Language Model based Autonomous Agents / 2308.11432 | 规划、记忆、工具与环境组成的系统分类。 | 覆盖早期研究，不能作为当前项目排名。 | [PDF](研究资料库/papers/2308.11432.pdf) · [阅读卡片](研究资料库/paper-notes/2308.11432.md) · [原文](https://arxiv.org/abs/2308.11432) |
| The Rise and Potential of Large Language Model Based Agents: A Survey / 2309.07864 | 单 Agent、多 Agent、人机协作的发展脉络。 | 早期综述，能力结论应结合近期研究复核。 | [PDF](研究资料库/papers/2309.07864.pdf) · [阅读卡片](研究资料库/paper-notes/2309.07864.md) · [原文](https://arxiv.org/abs/2309.07864) |
| Large Language Model based Multi-Agents: A Survey of Progress and Challenges / 2402.01680 | 多 Agent 角色、通信及协调方式的综述。 | 汇总研究不等于同任务受控性能比较。 | [PDF](研究资料库/papers/2402.01680.pdf) · [阅读卡片](研究资料库/paper-notes/2402.01680.md) · [原文](https://arxiv.org/abs/2402.01680) |
| From LLMs to LLM-based Agents for Software Engineering: A Survey of Current, Challenges and Future / 2408.02479 | 区分 SE 中纯 LLM 与 Agent，梳理任务及基准。 | 不同任务和设置的指标不可直接比较。 | [PDF](研究资料库/papers/2408.02479.pdf) · [阅读卡片](研究资料库/paper-notes/2408.02479.md) · [原文](https://arxiv.org/abs/2408.02479) |
| A Survey on Agent Workflow – Status and Future / 2508.01186 | 工作流构造、优化及发展脉络 | 综述覆盖受资料范围限制 | [PDF](研究资料库/papers/2508.01186.pdf) · [阅读卡片](研究资料库/paper-notes/2508.01186.md) · [原文](https://arxiv.org/abs/2508.01186) |
| Developing LLM-based Multi-Agent Systems in Software Engineering: A Mixed-Method Experience Report / 2608.11965 | 软件工程多 Agent 框架开发体验与遥测缺口。 | 摘要涉及 README 摘要任务，不能外推代码交付质量。 | [PDF](研究资料库/papers/2608.11965.pdf) · [阅读卡片](研究资料库/paper-notes/2608.11965.md) · [原文](https://arxiv.org/abs/2608.11965) |

## 论文：工作流优化

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines / 2310.03714 | 声明模块、定义指标并优化模型调用程序。 | 指标与数据划分决定效果，不能只优化展示样例。 | [PDF](研究资料库/papers/2310.03714.pdf) · [阅读卡片](研究资料库/paper-notes/2310.03714.md) · [原文](https://arxiv.org/abs/2310.03714) |
| Automated Design of Agentic Systems / 2408.08435 | 元 Agent 设计、评估及归档 Agent 系统。 | 需要训练/验证分离、预算限制和代码执行隔离。 | [PDF](研究资料库/papers/2408.08435.pdf) · [阅读卡片](研究资料库/paper-notes/2408.08435.md) · [原文](https://arxiv.org/abs/2408.08435) |
| AFlow: Automating Agentic Workflow Generation / 2410.10762 | 搜索代码表示的工作流，用执行反馈优化。 | 优化任务上的增益不能外推持久化和安全能力。 | [PDF](研究资料库/papers/2410.10762.pdf) · [阅读卡片](研究资料库/paper-notes/2410.10762.md) · [原文](https://arxiv.org/abs/2410.10762) |
| Optimizing Agentic Workflows using Meta-tools / 2601.22037 | 重复工具序列抽象为元工具 | 优化仍需权限与失败边界 | [PDF](研究资料库/papers/2601.22037.pdf) · [阅读卡片](研究资料库/paper-notes/2601.22037.md) · [原文](https://arxiv.org/abs/2601.22037) |

## 论文：安全治理

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| InjecAgent: Benchmarking Indirect Prompt Injections in Tool-Integrated Large Language Model Agents / 2403.02691 | 工具集成 Agent 的间接提示注入评估。 | 检测率和正常任务误报需共同评估。 | [PDF](研究资料库/papers/2403.02691.pdf) · [阅读卡片](研究资料库/paper-notes/2403.02691.md) · [原文](https://arxiv.org/abs/2403.02691) |
| AgentDojo: A Dynamic Environment to Evaluate Prompt Injection Attacks and Defenses for LLM Agents / 2406.13352 | 间接提示注入攻击与防御测试。 | 样本集有限；仍需要宿主权限边界。 | [PDF](研究资料库/papers/2406.13352.pdf) · [阅读卡片](研究资料库/paper-notes/2406.13352.md) · [原文](https://arxiv.org/abs/2406.13352) |
| LLM-Based Agents for Software and Systems Security: Approaches, Applications, and Assessment / 2608.28490 | 安全 Agent 的架构、应用与评估分类。 | 预印本；需区分安全任务成功与权限边界。 | [PDF](研究资料库/papers/2608.28490.pdf) · [阅读卡片](研究资料库/paper-notes/2608.28490.md) · [原文](https://arxiv.org/abs/2608.28490) |
| Passing the Test You Trained On: Re-evaluating Prompt-Injection Detectors for LLM Agents / 2610.03448 | 用工具输出评估注入检测器及误报；用于信任边界测试。 | 近期预印本；检测器效果受评测分布影响。 | [PDF](研究资料库/papers/2610.03448.pdf) · [阅读卡片](研究资料库/paper-notes/2610.03448.md) · [原文](https://arxiv.org/abs/2610.03448) |

## 论文：编程Agent

| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |
| --- | --- | --- | --- |
| SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering / 2405.15793 | Agent 与计算机接口设计，编辑、搜索和执行反馈。 | SWE-bench 修复不能覆盖产品发布及全流程工作。 | [PDF](研究资料库/papers/2405.15793.pdf) · [阅读卡片](研究资料库/paper-notes/2405.15793.md) · [原文](https://arxiv.org/abs/2405.15793) |
| Agentless: Demystifying LLM-based Software Engineering Agents / 2407.01489 | 定位、修复与验证的受控流程，提供简化基线。 | 针对缺陷修复，不能替代开放式开发 Agent。 | [PDF](研究资料库/papers/2407.01489.pdf) · [阅读卡片](研究资料库/paper-notes/2407.01489.md) · [原文](https://arxiv.org/abs/2407.01489) |
| OpenHands: An Open Platform for AI Software Developers as Generalist Agents / 2407.16741 | 软件工程 Agent 平台、动作与执行环境。 | 论文与当前实现版本有差异，执行环境需单独验证。 | [PDF](研究资料库/papers/2407.16741.pdf) · [阅读卡片](研究资料库/paper-notes/2407.16741.md) · [原文](https://arxiv.org/abs/2407.16741) |

## 项目：Rust框架

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig) | Rust trait、provider、工具及组合 | 9e7cb6000355 · [阅读卡片](研究资料库/project-notes/0xPlaygrounds--rig/README.md) · [ZIP](研究资料库/project-snapshots/0xPlaygrounds--rig-9e7cb6000355.zip) |

## 项目：协作协议

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [a2aproject/a2a-python](https://github.com/a2aproject/a2a-python) | Agent card、task lifecycle 与流式事件 | 0ee47d7baa47 · [阅读卡片](研究资料库/project-notes/a2aproject--a2a-python/README.md) · [ZIP](研究资料库/project-snapshots/a2aproject--a2a-python-0ee47d7baa47.zip) |

## 项目：多Agent框架

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [agentscope-ai/agentscope](https://github.com/agentscope-ai/agentscope) | 消息、运行时及协作管理 | bcf908f6d851 · [阅读卡片](研究资料库/project-notes/agentscope-ai--agentscope/README.md) · [ZIP](研究资料库/project-snapshots/agentscope-ai--agentscope-bcf908f6d851.zip) |
| [camel-ai/camel](https://github.com/camel-ai/camel) | 角色、通信及工具封装 | e5e55256e944 · [阅读卡片](研究资料库/project-notes/camel-ai--camel/README.md) · [ZIP](研究资料库/project-snapshots/camel-ai--camel-e5e55256e944.zip) |

## 项目：Agent框架

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [agno-agi/agno](https://github.com/agno-agi/agno) | Agent、Team、Workflow 的组合边界 | 5b854c59730c · [阅读卡片](研究资料库/project-notes/agno-agi--agno/README.md) · [ZIP](研究资料库/project-snapshots/agno-agi--agno-5b854c59730c.zip) |
| [crewAIInc/crewAI](https://github.com/crewAIInc/crewAI) | Crew/Task 与 Flow 的分工 | c0eda9f53889 · [阅读卡片](研究资料库/project-notes/crewAIInc--crewAI/README.md) · [ZIP](研究资料库/project-snapshots/crewAIInc--crewAI-c0eda9f53889-core.zip) |
| [google/adk-python](https://github.com/google/adk-python) | Sequential/Parallel/Loop Agent 与会话 | c213adf02d49 · [阅读卡片](研究资料库/project-notes/google--adk-python/README.md) · [ZIP](研究资料库/project-snapshots/google--adk-python-c213adf02d49.zip) |
| [huggingface/smolagents](https://github.com/huggingface/smolagents) | 精简 Agent loop、代码/工具动作 | 96f33faaf028 · [阅读卡片](研究资料库/project-notes/huggingface--smolagents/README.md) · [ZIP](研究资料库/project-snapshots/huggingface--smolagents-96f33faaf028.zip) |
| [microsoft/agent-framework](https://github.com/microsoft/agent-framework) | 统一 Agent 与 workflow 编排 | dcbbf7967afe · [阅读卡片](研究资料库/project-notes/microsoft--agent-framework/README.md) · [ZIP](研究资料库/project-snapshots/microsoft--agent-framework-dcbbf7967afe.zip) |
| [microsoft/autogen](https://github.com/microsoft/autogen) | A programming framework for agentic AI | 027ecf0a379b · [阅读卡片](研究资料库/project-notes/microsoft--autogen/README.md) · [ZIP](研究资料库/project-snapshots/AutoGen-027ecf0a379b.zip) |
| [pydantic/pydantic-ai](https://github.com/pydantic/pydantic-ai) | 类型化工具、输出验证与 durable 集成 | 0a28fc079668 · [阅读卡片](研究资料库/project-notes/pydantic--pydantic-ai/README.md) · [ZIP](研究资料库/project-snapshots/pydantic--pydantic-ai-0a28fc079668.zip) |
| [strands-agents/harness-sdk](https://github.com/strands-agents/sdk-python) | 工具执行、状态和协作 | dc03b1c127ee · [阅读卡片](研究资料库/project-notes/strands-agents--harness-sdk/README.md) · [ZIP](研究资料库/project-snapshots/strands-agents--harness-sdk-dc03b1c127ee.zip) |

## 项目：编程Agent

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [Aider-AI/aider](https://github.com/Aider-AI/aider) | 仓库映射、编辑格式及 Git 操作 | 5dc9490bb35f · [阅读卡片](研究资料库/project-notes/Aider-AI--aider/README.md) · [ZIP](研究资料库/project-snapshots/Aider-AI--aider-5dc9490bb35f.zip) |
| [anomalyco/opencode](https://github.com/anomalyco/opencode) | 客户端/服务端、工具、会话和权限 | 1015cb67d088 · [阅读卡片](研究资料库/project-notes/anomalyco--opencode/README.md) · [ZIP](研究资料库/project-snapshots/anomalyco--opencode-1015cb67d088.zip) |
| [OpenHands/OpenHands](https://github.com/OpenHands/OpenHands) | 动作、sandbox、运行事件与开发环境 | 5a8635a3f983 · [阅读卡片](研究资料库/project-notes/OpenHands--OpenHands/README.md) · [ZIP](研究资料库/project-snapshots/OpenHands--OpenHands-5a8635a3f983.zip) |
| [SWE-agent/SWE-agent](https://github.com/SWE-agent/SWE-agent) | Agent-computer interface 与补丁评测 | 3ea751c087f3 · [阅读卡片](研究资料库/project-notes/SWE-agent--SWE-agent/README.md) · [ZIP](研究资料库/project-snapshots/SWE-agent--SWE-agent-3ea751c087f3.zip) |

## 项目：多Agent协作

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [cooperbench/CooperBench](https://github.com/cooperbench/CooperBench) | A benchmark for evaluating how well AI coding agents can cooperate on software engineering tasks with potential conflicts. | 63b9d44d9f39 · [阅读卡片](研究资料库/project-notes/cooperbench--CooperBench/README.md) · [ZIP](研究资料库/project-snapshots/CooperBench-63b9d44d9f39.zip) |
| [hasenoz/PASC](https://github.com/hasenoz/PASC) | Replication package for "Coordinating Agents on a Shared Git Workspace: An Empirical Study of Action-Space Observation for Agentic Software Engineering". Includ | ae1ee633e91c · [阅读卡片](研究资料库/project-notes/hasenoz--PASC/README.md) · [ZIP](研究资料库/project-snapshots/PASC-source-ae1ee633e91c.zip) |
| [JiayiGeng/CAID](https://github.com/JiayiGeng/CAID) | Code repo for paper: Effective Strategies for Asynchronous Software Engineering Agents | f364d9e95727 · [阅读卡片](研究资料库/project-notes/JiayiGeng--CAID/README.md) · [ZIP](研究资料库/project-snapshots/CAID-f364d9e95727.zip) |

## 项目：安全评测

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [ethz-spylab/agentdojo](https://github.com/ethz-spylab/agentdojo) | 工具注入、策略与攻击防御 | 089ed468cf3e · [阅读卡片](研究资料库/project-notes/ethz-spylab--agentdojo/README.md) · [ZIP](研究资料库/project-snapshots/ethz-spylab--agentdojo-089ed468cf3e.zip) |
| [uiuc-kang-lab/InjecAgent](https://github.com/uiuc-kang-lab/InjecAgent) | 工具输出注入、攻击样本与评估脚本 | f19c9f2c79a4 · [阅读卡片](研究资料库/project-notes/uiuc-kang-lab--InjecAgent/README.md) · [ZIP](研究资料库/project-snapshots/uiuc-kang-lab--InjecAgent-f19c9f2c79a4.zip) |

## 项目：工作流优化

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [FoundationAgents/AFlow](https://github.com/FoundationAgents/AFlow) | 工作流搜索、算子和执行评估 | 3f457218fc71 · [阅读卡片](研究资料库/project-notes/FoundationAgents--AFlow/README.md) · [ZIP](研究资料库/project-snapshots/FoundationAgents--AFlow-3f457218fc71.zip) |
| [ShengranHu/ADAS](https://github.com/ShengranHu/ADAS) | 元 Agent 设计与候选档案 | 2702bee8fefd · [阅读卡片](研究资料库/project-notes/ShengranHu--ADAS/README.md) · [ZIP](研究资料库/project-snapshots/ShengranHu--ADAS-2702bee8fefd.zip) |
| [stanfordnlp/dspy](https://github.com/stanfordnlp/dspy) | 类型化模块、优化器、指标和轨迹 | 099982b3c3ee · [阅读卡片](研究资料库/project-notes/stanfordnlp--dspy/README.md) · [ZIP](研究资料库/project-snapshots/stanfordnlp--dspy-099982b3c3ee.zip) |

## 项目：多Agent编程

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [FoundationAgents/MetaGPT](https://github.com/FoundationAgents/MetaGPT) | SOP、角色交接与工程产物 | 11cdf466d042 · [阅读卡片](研究资料库/project-notes/FoundationAgents--MetaGPT/README.md) · [ZIP](研究资料库/project-snapshots/FoundationAgents--MetaGPT-11cdf466d042.zip) |
| [OpenBMB/ChatDev](https://github.com/OpenBMB/ChatDev) | 软件开发流程与角色通信 | 4fb2db0ea903 · [阅读卡片](研究资料库/project-notes/OpenBMB--ChatDev/README.md) · [ZIP](研究资料库/project-snapshots/OpenBMB--ChatDev-4fb2db0ea903.zip) |

## 项目：工作流运行时

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [langchain-ai/langgraph](https://github.com/langchain-ai/langgraph) | 图状态、checkpoint、interrupt、持久化执行 | afbe7e48b17f · [阅读卡片](研究资料库/project-notes/langchain-ai--langgraph/README.md) · [ZIP](研究资料库/project-snapshots/langchain-ai--langgraph-afbe7e48b17f.zip) |

## 项目：自动化工作流

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [langgenius/dify](https://github.com/langgenius/dify) | 图编排、节点结果与人机交互 | 8a237c6daf52 · [阅读卡片](研究资料库/project-notes/langgenius--dify/README.md) · [ZIP](研究资料库/project-snapshots/langgenius--dify-8a237c6daf52.zip) |
| [n8n-io/n8n](https://github.com/n8n-io/n8n) | 节点、触发器、凭据与执行生命周期 | de090cb2a0aa · [阅读卡片](研究资料库/project-notes/n8n-io--n8n/README.md) · [ZIP](研究资料库/project-snapshots/n8n-io--n8n-de090cb2a0aa.zip) |

## 项目：MCP工作流

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [lastmile-ai/mcp-agent](https://github.com/lastmile-ai/mcp-agent) | MCP 工具组合与 workflow | f62d84935081 · [阅读卡片](研究资料库/project-notes/lastmile-ai--mcp-agent/README.md) · [ZIP](研究资料库/project-snapshots/lastmile-ai--mcp-agent-f62d84935081.zip) |

## 项目：长期记忆

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [letta-ai/letta](https://github.com/letta-ai/letta) | 有状态 Agent、记忆块及上下文管理 | 5bcdd177d70f · [阅读卡片](研究资料库/project-notes/letta-ai--letta/README.md) · [ZIP](研究资料库/project-snapshots/letta-ai--letta-5bcdd177d70f.zip) |
| [mem0ai/mem0](https://github.com/mem0ai/mem0) | 记忆提取、检索、更新与删除 | d26ba5e4518c · [阅读卡片](研究资料库/project-notes/mem0ai--mem0/README.md) · [ZIP](研究资料库/project-snapshots/mem0ai--mem0-d26ba5e4518c.zip) |

## 项目：有状态Agent

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [letta-ai/letta-code](https://github.com/letta-ai/letta-code) | 当前 Letta Agent harness、server 与记忆集成 | 2943efe47e8c · [阅读卡片](研究资料库/project-notes/letta-ai--letta-code/README.md) · [ZIP](研究资料库/project-snapshots/letta-ai--letta-code-2943efe47e8c.zip) |

## 项目：协议工具

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) | Rust MCP Client/Server、transport 与取消 | 8763a3c3ffb2 · [阅读卡片](研究资料库/project-notes/modelcontextprotocol--rust-sdk/README.md) · [ZIP](研究资料库/project-snapshots/modelcontextprotocol--rust-sdk-8763a3c3ffb2.zip) |

## 项目：持久化工作流

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [PrefectHQ/prefect](https://github.com/PrefectHQ/prefect) | 任务、flow、状态、重试与可观察性 | 302beb37eb79 · [阅读卡片](研究资料库/project-notes/PrefectHQ--prefect/README.md) · [ZIP](研究资料库/project-snapshots/PrefectHQ--prefect-302beb37eb79.zip) |
| [restatedev/sdk-typescript](https://github.com/restatedev/sdk-typescript) | 持久化函数、日志与幂等语义 | 02a12acb879f · [阅读卡片](研究资料库/project-notes/restatedev--sdk-typescript/README.md) · [ZIP](研究资料库/project-snapshots/restatedev--sdk-typescript-02a12acb879f.zip) |
| [temporalio/sdk-python](https://github.com/temporalio/sdk-python) | workflow/activity 分离、重放和取消 | dca3a006dcd7 · [阅读卡片](研究资料库/project-notes/temporalio--sdk-python/README.md) · [ZIP](研究资料库/project-snapshots/temporalio--sdk-python-dca3a006dcd7.zip) |

## 项目：交互评测

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [sierra-research/tau-bench](https://github.com/sierra-research/tau-bench) | 用户模拟、工具策略与最终状态 | 59a200c6d575 · [阅读卡片](研究资料库/project-notes/sierra-research--tau-bench/README.md) · [ZIP](研究资料库/project-snapshots/sierra-research--tau-bench-59a200c6d575.zip) |

## 项目：评测

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [SWE-bench/SWE-bench](https://github.com/SWE-bench/SWE-bench) | 任务环境、补丁和测试结果 | 02e7a74ffd0b · [阅读卡片](研究资料库/project-notes/SWE-bench--SWE-bench/README.md) · [ZIP](研究资料库/project-snapshots/SWE-bench--SWE-bench-02e7a74ffd0b.zip) |

## 项目：通用Agent评测

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [THUDM/AgentBench](https://github.com/THUDM/AgentBench) | 交互环境、控制器与评估 | d1e4a10db08c · [阅读卡片](研究资料库/project-notes/THUDM--AgentBench/README.md) · [ZIP](研究资料库/project-snapshots/THUDM--AgentBench-d1e4a10db08c.zip) |

## 项目：桌面自动化评测

| 项目 | 阅读目标 | 版本与本地入口 |
| --- | --- | --- |
| [xlang-ai/OSWorld](https://github.com/xlang-ai/OSWorld) | 任务环境、桌面动作与最终状态检查 | b138d3482560 · [阅读卡片](研究资料库/project-notes/xlang-ai--OSWorld/README.md) · [ZIP](研究资料库/project-snapshots/xlang-ai--OSWorld-b138d3482560.zip) |

## 如何用于开发

先读专题结论，再选 2–3 个项目追踪实际源码；用[项目开发映射](项目开发映射与研究验证计划.md)形成契约、实验和验收任务。不得跨论文直接比较成功率或将外部结果当作本项目证据。

源码包保留固定提交、校验与筛选范围。CrewAI 为核心模块选读包，新收录其他项目为筛选文本源码，缺少依赖/大数据不能直接复现实验。[全部元数据](研究资料库/project-snapshots/catalog.json)和[许可证原文](研究资料库/README.md)用于进一步核对。
