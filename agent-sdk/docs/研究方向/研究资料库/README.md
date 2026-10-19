# Agent 研究资料库

采集与整理日期：2026-10-13。当前有 48 篇论文 PDF、40 个源码包和 1 个项目迁移说明包，约 406.4 MiB（不含选读副本、元数据及工具）。

## 入口

- [论文与项目总表](../论文与GitHub项目调研.md)
- [八个主题综述](../README.md)
- [论文元数据及 PDF 哈希](papers/manifest.json)
- [项目提交、范围与源码哈希](project-snapshots/catalog.json)
- [统一 SHA-256](SHA256SUMS.txt)
- [BibTeX](references.bib)
- [文件与链接校验结果](validation.json)

## 离线资料结构

| 目录 | 内容 |
| --- | --- |
| papers/ | 48 篇原始 PDF；作者、来源、版本和页数见 manifest |
| paper-notes/ | 每篇论文的阅读卡片、机制与限制 |
| paper-text/ | PDF 前三页文本，用于搜索和初步阅读 |
| project-snapshots/ | 固定提交 ZIP；manifest.json 保留前一批，catalog.json 为全部清单 |
| project-notes/ | 41 份阅读卡片、上游 README、许可与文件范围 |
| project-notes/*/selected-source/ | 合计 477 个可直接打开的选读源码文件 |
| metadata/ | 原始出版页 HTML、下载记录、候选错误记录 |
| tools/ | 采集、筛选、整理工具及输入种子 |

## 快照范围与许可

新源码包保留可阅读文本及许可，过滤安装依赖、二进制、生成目录、实验运行输出和录制响应。逐文件范围在各项目 source-map.json 中。CrewAI 只下载选定核心模块，不能据此认为已取得完整仓库。所有包均未安装或执行。

Letta 主仓库当前是迁移说明，当前源码在 letta-ai/letta-code；Strands 旧地址已重定向到 strands-agents/harness-sdk。

n8n 保留 Sustainable Use 与 Enterprise 许可文件；Dify 保留带附加条件的许可；AutoGen 有 LICENSE 和 LICENSE-CODE。许可可能区分文档、源码和组件，GitHub SPDX 字段不充分，采用具体文件前核对原文。未确认许可的资料只记录事实，不推断可自由复制。

## 下载论文

| 主题 | 数量 |
| --- | --- |
| 基础框架 | 4 |
| 多Agent协作 | 13 |
| 记忆上下文 | 5 |
| 自动化环境 | 4 |
| 评测基准 | 5 |
| 综述方向 | 6 |
| 工作流优化 | 4 |
| 安全治理 | 4 |
| 编程Agent | 3 |

## 源码阅读卡片

- [0xPlaygrounds/rig](project-notes/0xPlaygrounds--rig/README.md)：9e7cb6000355，12 个选读文件。
- [a2aproject/a2a-python](project-notes/a2aproject--a2a-python/README.md)：0ee47d7baa47，12 个选读文件。
- [agentscope-ai/agentscope](project-notes/agentscope-ai--agentscope/README.md)：bcf908f6d851，12 个选读文件。
- [agno-agi/agno](project-notes/agno-agi--agno/README.md)：5b854c59730c，12 个选读文件。
- [Aider-AI/aider](project-notes/Aider-AI--aider/README.md)：5dc9490bb35f，12 个选读文件。
- [anomalyco/opencode](project-notes/anomalyco--opencode/README.md)：1015cb67d088，12 个选读文件。
- [camel-ai/camel](project-notes/camel-ai--camel/README.md)：e5e55256e944，12 个选读文件。
- [cooperbench/CooperBench](project-notes/cooperbench--CooperBench/README.md)：63b9d44d9f39，12 个选读文件。
- [crewAIInc/crewAI](project-notes/crewAIInc--crewAI/README.md)：c0eda9f53889，12 个选读文件。
- [ethz-spylab/agentdojo](project-notes/ethz-spylab--agentdojo/README.md)：089ed468cf3e，12 个选读文件。
- [FoundationAgents/AFlow](project-notes/FoundationAgents--AFlow/README.md)：3f457218fc71，12 个选读文件。
- [FoundationAgents/MetaGPT](project-notes/FoundationAgents--MetaGPT/README.md)：11cdf466d042，12 个选读文件。
- [google/adk-python](project-notes/google--adk-python/README.md)：c213adf02d49，12 个选读文件。
- [hasenoz/PASC](project-notes/hasenoz--PASC/README.md)：ae1ee633e91c，12 个选读文件。
- [huggingface/smolagents](project-notes/huggingface--smolagents/README.md)：96f33faaf028，12 个选读文件。
- [JiayiGeng/CAID](project-notes/JiayiGeng--CAID/README.md)：f364d9e95727，12 个选读文件。
- [langchain-ai/langgraph](project-notes/langchain-ai--langgraph/README.md)：afbe7e48b17f，12 个选读文件。
- [langgenius/dify](project-notes/langgenius--dify/README.md)：8a237c6daf52，12 个选读文件。
- [lastmile-ai/mcp-agent](project-notes/lastmile-ai--mcp-agent/README.md)：f62d84935081，12 个选读文件。
- [letta-ai/letta](project-notes/letta-ai--letta/README.md)：5bcdd177d70f，0 个选读文件。
- [letta-ai/letta-code](project-notes/letta-ai--letta-code/README.md)：2943efe47e8c，12 个选读文件。
- [mem0ai/mem0](project-notes/mem0ai--mem0/README.md)：d26ba5e4518c，12 个选读文件。
- [microsoft/agent-framework](project-notes/microsoft--agent-framework/README.md)：dcbbf7967afe，12 个选读文件。
- [microsoft/autogen](project-notes/microsoft--autogen/README.md)：027ecf0a379b，12 个选读文件。
- [modelcontextprotocol/rust-sdk](project-notes/modelcontextprotocol--rust-sdk/README.md)：8763a3c3ffb2，12 个选读文件。
- [n8n-io/n8n](project-notes/n8n-io--n8n/README.md)：de090cb2a0aa，12 个选读文件。
- [OpenBMB/ChatDev](project-notes/OpenBMB--ChatDev/README.md)：4fb2db0ea903，12 个选读文件。
- [OpenHands/OpenHands](project-notes/OpenHands--OpenHands/README.md)：5a8635a3f983，12 个选读文件。
- [PrefectHQ/prefect](project-notes/PrefectHQ--prefect/README.md)：302beb37eb79，12 个选读文件。
- [pydantic/pydantic-ai](project-notes/pydantic--pydantic-ai/README.md)：0a28fc079668，12 个选读文件。
- [restatedev/sdk-typescript](project-notes/restatedev--sdk-typescript/README.md)：02a12acb879f，12 个选读文件。
- [ShengranHu/ADAS](project-notes/ShengranHu--ADAS/README.md)：2702bee8fefd，12 个选读文件。
- [sierra-research/tau-bench](project-notes/sierra-research--tau-bench/README.md)：59a200c6d575，12 个选读文件。
- [stanfordnlp/dspy](project-notes/stanfordnlp--dspy/README.md)：099982b3c3ee，12 个选读文件。
- [strands-agents/harness-sdk](project-notes/strands-agents--harness-sdk/README.md)：dc03b1c127ee，12 个选读文件。
- [SWE-agent/SWE-agent](project-notes/SWE-agent--SWE-agent/README.md)：3ea751c087f3，12 个选读文件。
- [SWE-bench/SWE-bench](project-notes/SWE-bench--SWE-bench/README.md)：02e7a74ffd0b，12 个选读文件。
- [temporalio/sdk-python](project-notes/temporalio--sdk-python/README.md)：dca3a006dcd7，12 个选读文件。
- [THUDM/AgentBench](project-notes/THUDM--AgentBench/README.md)：d1e4a10db08c，12 个选读文件。
- [uiuc-kang-lab/InjecAgent](project-notes/uiuc-kang-lab--InjecAgent/README.md)：f19c9f2c79a4，9 个选读文件。
- [xlang-ai/OSWorld](project-notes/xlang-ai--OSWorld/README.md)：b138d3482560，12 个选读文件。

论文核验仅涵盖元数据、结构和前三页；完整实验设置、附录及可靠性结论仍需进一步精读。PDF 或源码存在不构成 OwO 的功能验收。
