# SharedCore：共享核心上下文的多 Agent 并行开发框架

架构设计方案 v1.0

面向 AI Coding

## 一、总体设计思路

我建议采用 共享核心上下文（Shared Core Context）+ 独立 Agent 执行空间 + 事件驱动同步 + 任务依赖调度 + 自动代码集成 的架构。

它不是简单地让多个 AI 同时编程，而是构建一个有统一项目认知、可以自主分工、并行开发、互相协调、持续验证的软件开发系统。

核心目标是解决五个问题：

1. 上下文共享：每个 Agent 都知道项目的目标、架构、技术约束和已经确定的设计决策。

2. 开发并行化：可以将一个复杂需求拆分为多个任务，允许不同 Agent 同时编写代码。

3. 实时协作：当 Agent A 修改 API、数据结构或者接口规范时，相关 Agent B 能及时获知。

4. 代码冲突控制：多个 Agent 不会无序修改同一份代码，最终产物可以安全集成。

5. 持续执行能力：即使某个 Agent 的上下文窗口用尽、进程中断或任务失败，整个开发过程仍然可以恢复。

### 核心架构

用户需求 / 项目目标

自然语言、项目仓库、规范文档

Orchestrator · 全局编排器

任务拆分 · 依赖分析 · 调度 · 冲突检测 · 合并决策

Shared Context Hub

所有 Agent 共享的唯一权威项目状态

项目规范与架构

接口与数据契约

任务 DAG 与进度

决策日志与事件

代码索引与版本

测试结果与产物

Agent A

后端开发

Worktree A

Agent B

前端开发

Worktree B

Agent C

测试开发

Worktree C

Integration Gate · 集成验证

版本检查 → 合并分支 → 构建 → 测试 → 审查

生成可交付的软件版本

这个框架最重要的架构原则是：

共享项目知识，不共享所有对话；共享结构化状态，不共享工作目录；并行执行任务，但对影响全局的决策和代码集成进行有序控制。

这里的「共享上下文」不是让不同模型共享内部 KV Cache 或隐式推理状态，而是让所有 Agent 读取和更新一个受版本管理、可追溯的外部项目上下文。

## 二、框架的六个核心模块

| 核心模块          | 功能                                | 优先级 |
| ----------------- | ----------------------------------- | ------ |
| Context Hub       | 统一管理项目知识、规范、决策和状态  | P0     |
| Task Scheduler    | 将需求转化为任务 DAG，执行并行调度  | P0     |
| Agent Runtime     | 独立启动和管理多个 Coding Agent     | P0     |
| Workspace Manager | 管理 Git Worktree、分支、文件所有权 | P0     |
| Event Bus         | Agent 间传播变更、决策和依赖事件    | P1     |
| Integration Gate  | 合并、测试、质量验证、回滚          | P0     |

此外还需要一个持久化层，记录任务状态、Agent 运行状态、日志、成本和检查点，以支持故障恢复。

## 三、核心机制一：共享上下文系统

这是整套框架最有技术价值的部分。

传统多 Agent 系统通常采用两种方式：一种是把主 Agent 的上下文传给子 Agent，另一种是把所有 Agent 放在一个公共消息频道中。前者难以同步更新，后者容易造成信息过载。

我建议建立一个独立的 Context Hub，将项目上下文设计成版本化的结构化数据，而不是长篇聊天记录。

### 3.1 五层上下文模型

L0 · Core Context

所有 Agent 必读：项目目标、整体架构、技术规范、安全约束

L1 · Domain Context

按需加载：前端、后端、数据库、算法等模块知识

L2 · Task Context

当前任务：需求、验收标准、依赖、代码范围、版本

L3 · Working Memory

Agent 私有：临时推理、探索过程、未确认假设

L4 · Shared Events

跨 Agent 更新：接口变更、任务完成、冲突、测试失败

注意 L3 私有上下文不能直接覆盖 L0 核心上下文。例如后端 Agent 在调试时认为应该更换认证体系，这只能形成一个变更提案。只有经过架构决策流程后，才能正式写入 Core Context。

### 3.2 核心数据结构

建议定义如下项目状态：

```
project:
  id: project-001
  name: ExampleProject
  core_version: 12
  baseline_commit: a1b2c3d

  architecture:
    frontend: React + TypeScript
    backend: FastAPI
    database: PostgreSQL

  rules:
    - API 使用统一错误格式
    - 共享接口必须先定义契约
    - 禁止直接修改 main
    - 所有代码变更必须经过测试

  contracts:
    - id: auth-api
      version: 3
      owner: backend-agent

  decisions:
    - id: ADR-001
      title: 使用 JWT 认证
      status: accepted

  tasks:
    - id: T-101
      owner: frontend-agent
      status: running
      depends_on: [T-100]

  context_revision: 46
```

其中需要区别两个版本：

- `core_version`：正式项目规范的版本。只有经过批准的项目级变更才能递增。

- `context_revision`：共享状态修订号。任务状态、事件和新产物入库也会使其递增。

任务调度器可以要求 Agent 基于指定的 `core_version` 和代码 `baseline_commit` 开始工作，避免不同 Agent 使用不一致的设计基础。

### 3.3 上下文同步协议

我建议使用 快照 + 增量事件 + 乐观并发控制。

工作过程如下：

读取快照

Agent A 启动，读取核心上下文 v12 与自己任务相关的模块知识。

独立开发

Agent A 修改认证接口，在自己的工作区内验证。

提交变更提案

发布接口契约 v4 提案，附上变更原因、影响范围与兼容性信息。

审查与提交

通过版本检查和审批后，原子更新契约，产生新版本事件。

按需同步

依赖该接口的 Agent B 收到通知，重新加载契约并判断是否需要调整代码。

重要的是，Agent B 不需要重新读取全部上下文，只需拉取受影响的上下文片段。

例如事件格式：

```
{
  "event_id": "evt-00042",
  "type": "contract.updated",
  "source": "backend-agent",
  "resource": "contracts/auth-api",
  "old_version": 3,
  "new_version": 4,
  "affected_tasks": ["T-101", "T-104"],
  "breaking_change": true
}
```

对破坏性变更，调度器应暂停相关任务的集成，要求它们确认新契约；对向后兼容的变更，只发送通知，不强制中断 Agent 的当前执行。

### 3.4 并发写入一致性

多 Agent 共享状态不能等同于所有 Agent 都能随意修改公共 Markdown 文件。

建议采用以下规则：

| 数据类型       | 写入方式                     | 一致性策略           |
| -------------- | ---------------------------- | -------------------- |
| 核心项目规范   | 提案、审批后提交             | 单写入入口，版本校验 |
| API / 数据契约 | 契约拥有者提出，受影响方审查 | 版本化、兼容性检查   |
| 任务进度       | 任务拥有者更新               | 原子状态迁移         |
| Agent 消息     | 多 Agent 并发追加            | 不可变事件日志       |
| 临时推理       | 各 Agent 自己维护            | 私有状态             |
| 代码文件       | 独立 Git 分支修改            | 集成时检测冲突       |

实现上，核心数据写入时携带 `expected_version`。服务端进行原子 Compare-and-Swap 检查。若版本已经变化，则拒绝覆盖并要求重新读取、合并或者提交冲突解决提案。

这能避免一个典型问题：Agent A 和 B 同时读取 v12，然后各自写入，导致后写入者覆盖先写入者的决策。

## 四、核心机制二：多 Agent 并行任务调度

我不建议简单采用固定的「前端 Agent、后端 Agent、测试 Agent」分工。

因为同一个项目的实际任务经常存在依赖关系。固定角色同时启动，不一定是真正有效的并行开发。

更合适的方案是：任务图驱动的动态 Agent 调度。

### 4.1 构建任务 DAG

假设用户输入：

> 开发一个支持账号登录、课程上传、知识图谱展示和学情分析的 Web 应用。

Orchestrator 将需求拆分成如下任务：

T0 · 架构设计与契约定义

项目骨架、API Schema、数据模型

T1 · 身份认证

Agent A

T2 · 课程上传

Agent B

T3 · 知识图谱

Agent C

T4 · 学情分析

Agent D

T5 · 跨模块集成与联调

等待 T1、T2、T3、T4 满足验收条件

T6 · 回归测试与交付

集成测试、安全审查、构建

其中 T1～T4 只有在 T0 的共享契约已经确定后，才可以直接并行开发。它们可以按不同模块组织，也可以进一步拆分成更细粒度的任务。

### 4.2 任务调度算法

设任务集合为：

\[
T=\{t_1,t_2,\ldots,t_n\}
\]

任务之间的依赖形成有向无环图 \(G=(T,E)\)。

一个任务可以执行，需要同时满足：

\[
Ready(t_i)=
DepsDone(t_i)\land
ContextValid(t_i)\land
ResourcesFree(t_i)
\]

其中：

- `DepsDone`：所有前置依赖均已完成或达到允许继续开发的状态。

- `ContextValid`：依赖的项目规范和接口契约版本有效。

- `ResourcesFree`：目标文件、模块及运行资源没有不可兼容的并发占用。

调度算法采用：

拓扑依赖检查 + 资源冲突检测 + 优先级调度 + 动态并发数量。

并发数量不应越大越好。可以根据可运行任务数、Agent 资源、模型额度、测试资源和预计协作成本进行限制。

### 4.3 每个任务使用结构化 Task Contract

```
task_id: T-103
title: 实现知识图谱可视化

owner: frontend-agent
status: ready

depends_on:
  - T-100

context:
  core_version: 12
  contracts:
    graph-api: 2

read_scope:
  - docs/architecture/**
  - contracts/graph-api.yaml

write_scope:
  - frontend/src/components/graph/**
  - frontend/src/hooks/useGraph.ts

acceptance:
  - 支持节点拖拽
  - 支持缩放和平移
  - 正确展示概念关系
  - 组件测试全部通过

outputs:
  - Git commit
  - Test report
  - Change summary
```

Agent 只能在授予的写入范围中修改代码。若需要跨范围修改，必须申请扩展权限或新增依赖任务。

这使每个任务成为一个具备输入、输出、约束和验收条件的独立执行单元。

## 五、核心机制三：独立代码工作空间

建议直接使用 Git Worktree 作为初期实现基础。

Git Worktree 可以让同一个 Git 仓库关联多个工作目录，每个工作目录拥有自己的检出状态和索引，非常适合多个 Agent 并行处理不同分支。

git-worktree Documentation

项目结构例如：

```
project/
├── main/                   # 主开发工作目录
├── worktrees/
│   ├── agent-a/            # 分支 agent/T-101
│   ├── agent-b/            # 分支 agent/T-102
│   └── agent-c/            # 分支 agent/T-103
└── sharedcore/             # 编排状态及运行数据
```

创建工作目录：

```
git -C main worktree add -b agent/T-101 ../worktrees/agent-a HEAD
git -C main worktree add -b agent/T-102 ../worktrees/agent-b HEAD
git -C main worktree add -b agent/T-103 ../worktrees/agent-c HEAD
```

三个 Agent 可以分别编辑代码、执行 Git Commit，而不会直接污染其他 Agent 的未提交文件。

但 Worktree 只解决 Git 工作目录隔离，不自动隔离数据库、端口、Docker 容器、构建缓存或外部服务。因此每个 Agent 还需要独立的运行配置；执行不可信代码时应使用容器或沙箱。

### 代码冲突解决策略

我建议按照资源冲突分为三类：

| 冲突类型                               | 框架行为                       |
| -------------------------------------- | ------------------------------ |
| 两个 Agent 修改完全不同模块            | 允许并行                       |
| 修改同一个模块但不同接口或文件         | 先判断语义依赖，再决定是否并行 |
| 修改同一文件的相同逻辑，或接口版本互斥 | 暂停冲突任务，重新分配或协调   |

不要依赖 Git 在最后才发现冲突。

调度器应该预先维护 `read_scope` 和 `write_scope`，并使用路径、符号引用、API 依赖等信息预测潜在冲突。

对于未被预测到的冲突，Integration Gate 再进行二次验证。即使 Git 可以自动合并，也不代表程序的业务逻辑一定兼容。

## 六、核心机制四：Agent 间通信

我建议将通信划分为两种模式。

模式 A：事件广播。 用于通知状态变化，例如接口契约更新、任务完成、测试失败。这类消息可以发送给订阅了相关资源的多个 Agent。

模式 B：定向协作。 用于需要明确回应的请求，例如 Agent A 请 Agent B 确认接口参数，或者请求架构 Agent 批准跨模块修改。

这样可以避免所有 Agent 都参与每一条讨论。

### 通信协议

统一采用结构化消息：

```
{
  "message_id": "msg-001",
  "sender": "backend-agent",
  "receiver": "frontend-agent",
  "type": "contract.review_request",
  "task_id": "T-102",
  "payload": {
    "contract": "auth-api",
    "proposed_version": 4,
    "change": "新增 refresh_token 字段"
  },
  "requires_response": true
}
```

建议先实现以下七类消息：

| 类型                | 用途                   |
| ------------------- | ---------------------- |
| `task.assigned`     | 任务分配               |
| `task.completed`    | 任务完成               |
| `contract.updated`  | 接口契约更新           |
| `context.changed`   | 共享上下文变更         |
| `review.requested`  | 请求其他 Agent 审查    |
| `conflict.detected` | 资源或代码冲突         |
| `agent.blocked`     | 遇到无法自行处理的问题 |

传输层可以采用 Redis Streams 或 PostgreSQL Outbox + 事件消费者。第一版不需要 Kafka。

事件应该具备唯一 ID、幂等处理、可重放日志和消费确认。对关键变更，不能仅依靠不持久化的 Pub/Sub 广播。

### 运行时同步方式

Agent 不应在每次收到普通事件时立刻打断生成过程，而应在固定的安全点同步：

- 开始一个新的开发子步骤之前。

- 访问接口契约或者共享代码之前。

- 执行测试和提交代码之前。

- 收到影响当前任务的破坏性变更时。

其中最后一种情况由调度器设置任务失效标志，Agent 在下一次可控检查点停止或重新规划。

## 七、核心机制五：Agent Runtime 与角色分工

框架应当支持多模型、多 Agent 驱动，而不是将整个系统绑定到某一家模型的对话接口。

### 7.1 Agent 角色

| Agent      | 职责                             | 权限                   |
| ---------- | -------------------------------- | ---------------------- |
| Planner    | 理解需求、规划架构、拆分任务     | 提议架构及创建任务     |
| Architect  | 审查全局设计、接口规范和重大变更 | 批准或拒绝核心契约变更 |
| Developer  | 编码、调试、单元测试             | 修改分配范围内的代码   |
| Reviewer   | 静态审查、安全检查、代码质量评估 | 提出修改意见           |
| Tester     | 自动生成和执行测试               | 测试环境读写           |
| Integrator | 合并分支、运行集成测试           | 受控操作集成分支       |

这些是角色，而不是必须同时启动的六个模型实例。

例如某个小型修复任务，只需要一个 Developer 和自动化测试；只有发生跨模块修改时才需要 Architect 介入。

### 7.2 统一 Agent Adapter

框架将模型和编程工具封装为统一接口：

```
class AgentAdapter:
    async def start(self, task, context, workspace):
        ...

    async def send(self, message):
        ...

    async def checkpoint(self):
        ...

    async def cancel(self):
        ...

    async def get_status(self):
        ...
```

具体实现可以提供：

```
AgentAdapter
├── CodexAdapter
├── ClaudeCodeAdapter
├── OpenAIAgentsAdapter
├── CustomLLMAdapter
└── LocalModelAdapter
```

这些是计划设计的适配器接口，并非现成统一 SDK。不同 Coding Agent 的会话恢复、命令执行、工具授权和工作区支持存在差异，适配器需要逐一实现。

这样可以做到：

- 使用较强的推理模型执行架构设计与复杂审查。

- 使用编程模型处理各自负责的代码。

- 使用成本较低的模型执行文档整理、变更摘要等任务。

- 在某个 Agent 失败时，允许另一个兼容的 Agent 接管任务。

任务接管依赖结构化检查点，而不是简单复制失败 Agent 的聊天记录。

### 7.3 Agent 生命周期

```
CREATED
   ↓
INITIALIZING
   ↓
READY
   ↓
RUNNING
   ├── BLOCKED
   ├── NEEDS_REPLAN
   ├── CHECKPOINTED
   └── FAILED
   ↓
SUBMITTED
   ↓
VALIDATING
   ├── REWORK
   └── ACCEPTED
```

其中任务结束和 Agent 进程结束是两个不同概念。一个 Agent 进程可以连续处理多个任务，但每个任务必须有独立的执行记录和验收状态。

出现模型超时、机器重启或 Token 上限时，系统可以从最近一个检查点恢复；对于已经执行的非幂等操作，必须先核查其结果，不能盲目重试。

## 八、核心机制六：自动集成与验证

多个 Agent 完成代码后，不应该直接向 `main` 分支合并。

我建议增加一个独立的 Integration Gate，采用受控集成分支进行验证。

1. 任务完成

Agent 提交 Commit，记录基线版本及上下文版本

2. 预检查

检查写入范围、契约版本、依赖状态与权限

3. 合并候选

在临时集成分支上按依赖顺序合并

4. 自动验证

编译、静态检查、单元测试、集成测试

5. 代码审查

检查安全、业务逻辑、接口和架构一致性

6. 正式合并

全部通过后更新目标分支及项目状态

如果验证失败，Integration Gate 生成带有失败原因、相关文件、日志位置和复现命令的报告，并将其作为新的修复任务返回给对应 Agent。

这里需要特别控制三个问题：

第一，合并检查必须针对最新目标分支。 不能只在 Agent 开始任务时的旧基线上执行测试。

第二，合并过程必须串行提交。 多个已验证候选可以并行进行预检查，但更新目标分支的最终步骤应由一个受控队列负责，必要时重新验证。

第三，测试失败不能无限自动修复。 每个任务需要配置最大重试次数、费用预算和升级条件，超过阈值后进入人工审查。

## 九、技术选型

目前已经存在一些成熟组件，不需要从零实现所有 Agent 调用和状态管理能力。

| 现有技术                                                                                                        | 已具备能力                                  | 在本方案中的定位                 |
| --------------------------------------------------------------------------------------------------------------- | ------------------------------------------- | -------------------------------- |
| [OpenAI Agents SDK](https://openai.github.io/openai-agents-python/?utm_source=chatgpt.com)                      | Agent 定义、工具、Handoff、Session、Tracing | Agent 调用和工具执行层           |
| [LangGraph](https://docs.langchain.com/oss/python/langgraph/overview?utm_source=chatgpt.com)                    | 状态图、持久化、执行控制                    | 可选的任务编排引擎               |
| [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/overview/?utm_source=chatgpt.com) | 多 Agent 工作流、运行状态、可观测性         | 替代编排方案                     |
| [Claude Code Agent Teams](https://code.claude.com/docs/en/agent-teams?utm_source=chatgpt.com)                   | 独立会话、共享任务列表、Agent 消息          | 参考现有协作能力                 |
| [Temporal](https://docs.temporal.io/?utm_source=chatgpt.com)                                                    | 持久工作流、恢复、重试                      | 生产级长任务调度                 |
| [MCP](https://modelcontextprotocol.io/?utm_source=chatgpt.com)                                                  | 标准化工具和资源访问                        | 对 Coding Agent 暴露 Context Hub |

OpenAI Agents SDK 已支持 Agent 编排、Handoff 与运行追踪；Microsoft Agent Framework 提供显式工作流与运行状态管理。

OpenAI Agents SDK+1

Temporal 也已提供与 Agent SDK 集成的持久工作流方案。

Temporal Documentation

Claude Code 的 Agent Teams 已具备部分相似特征，包括共享任务列表和 Agent 直接消息通信，但官方仍将其标为实验性能力，且提示存在协调、恢复方面的限制。

Claude Code Docs

### 推荐落地技术栈

我更倾向于以下组合：

| 层级       | MVP 方案                       | 生产扩展                 |
| ---------- | ------------------------------ | ------------------------ |
| 控制服务   | Python + FastAPI               | 保持                     |
| Agent 编排 | 自定义 DAG Scheduler + asyncio | Temporal 或 LangGraph    |
| Agent 执行 | SDK / CLI Adapter              | 多模型、多运行环境       |
| 核心状态   | PostgreSQL + JSONB             | PostgreSQL + 向量索引    |
| 事件系统   | PostgreSQL Outbox              | Redis Streams / 消息队列 |
| 代码隔离   | Git Worktree                   | Worktree + 容器沙箱      |
| 上下文访问 | 内部 API                       | MCP Server               |
| 测试与集成 | Git + 本地 CI 脚本             | 独立 CI Worker           |
| 管理界面   | CLI                            | React + WebSocket        |

选择这套技术栈的原因，是它能够先将共享上下文与协作协议做好，而不是在第一版就引入过于复杂的分布式基础设施。

MCP 可以用作不同 Agent 读取核心上下文、查询任务、提交变更提案的统一工具接口，但它本身不是一致性数据库，也不负责 Git 冲突解决。相关事务、鉴权和版本控制仍应由 Context Hub 服务实现。

## 十、工程实现：推荐项目目录

```
sharedcore/
├── apps/
│   ├── server/              # HTTP API / WebSocket
│   ├── cli/                 # 命令行工具
│   └── dashboard/           # 可视化控制台
│
├── core/
│   ├── context/
│   │   ├── store.py         # 上下文持久化
│   │   ├── assembler.py     # 动态上下文组装
│   │   ├── versioning.py    # 版本与一致性
│   │   └── retrieval.py     # 相关内容检索
│   │
│   ├── scheduler/
│   │   ├── dag.py           # 任务依赖图
│   │   ├── planner.py       # 任务分解
│   │   └── dispatcher.py    # 并行任务调度
│   │
│   ├── agents/
│   │   ├── runtime.py
│   │   ├── adapter.py
│   │   └── registry.py
│   │
│   ├── workspace/
│   │   ├── worktree.py
│   │   └── ownership.py
│   │
│   ├── events/
│   │   ├── bus.py
│   │   └── handlers.py
│   │
│   └── integration/
│       ├── merger.py
│       ├── validator.py
│       └── reviewer.py
│
├── protocols/
│   ├── task.schema.json
│   ├── context.schema.json
│   └── event.schema.json
│
├── adapters/
│   ├── codex.py
│   └── claude.py
│
└── tests/
```

### 10.1 Context Hub API

建议设计以下接口，未来可同时提供 REST 和 MCP 两种访问方式。

| 接口                     | 功能                       |
| ------------------------ | -------------------------- |
| `context.get_core`       | 读取核心项目规范           |
| `context.get_slice`      | 根据任务读取相关上下文     |
| `context.get_changes`    | 获取指定版本之后的增量变更 |
| `context.propose_change` | 提交上下文修改提案         |
| `context.commit_change`  | 经授权审批后提交           |
| `tasks.list_ready`       | 查询当前可运行任务         |
| `tasks.claim`            | 原子领取任务               |
| `tasks.report`           | 报告任务进展与结果         |
| `events.publish`         | 发布结构化事件             |
| `integration.submit`     | 提交代码集成申请           |

其中 `context.get_slice` 是最需要优化的接口。

它不应该返回全部文档，而应该按以下逻辑组装上下文：

\[
C_i=C_{\text{core}}+
C_{\text{domain}}(T_i)+
C_{\text{dependencies}}(T_i)+
C_{\text{recent}}
\]

这里的加号表示上下文组合，不是简单字符串拼接。

组装过程还要进行相关性排序、去重、Token 预算裁剪，以及版本和来源标注。核心规范不能被低可信度的检索结果覆盖。

### 10.2 任务调度器核心伪代码

```
async def scheduler_loop():

    while project.is_active():

        tasks = await task_store.list_pending()

        ready = [
            task for task in tasks
            if dependencies_satisfied(task)
            and context_is_valid(task)
            and not resource_conflict(task)
        ]

        ready = prioritize(ready)

        slots = capacity.available_slots()

        for task in ready[:slots]:
            claimed = await task_store.try_claim(task.id)

            if not claimed:
                continue

            workspace = await workspace_manager.create(
                task_id=task.id
            )

            context = await context_hub.assemble(
                task=task
            )

            await agent_runtime.launch(
                task=task,
                workspace=workspace,
                context=context
            )

        await scheduler_events.wait_for_change()
```

这只是调度算法骨架，完整实现还需要任务租约、超时、心跳、任务恢复、执行结果处理和原子资源预留。

实际系统中，`try_claim` 必须在数据库事务中检查任务状态及资源占用，不能只在 Python 内存中完成冲突判断。

## 十一、MVP 实施路线

我建议分三个阶段实现，而不是一次性构建完整的分布式 Agent 平台。

阶段一：最小可运行系统

实现 1 个 Orchestrator、3 个 Developer Agent、共享核心上下文、任务 DAG、Git Worktree 和自动集成。

验收目标：输入一个需求，系统能够拆分成三个独立任务，并行编码、测试和合并。

阶段二：真正的共享上下文与协作

增加 Context Hub 版本控制、结构化事件、接口变更传播、文件所有权和 Agent 协作请求。

验收目标：一个 Agent 改变接口规范，相关 Agent 能够收到通知并调整工作，而无关 Agent 不受影响。

阶段三：长期稳定执行

增加断点恢复、多模型适配、沙箱隔离、权限控制、预算限制、全链路追踪和管理界面。

验收目标：Agent 中断后可以恢复；多任务连续执行时，仍能保证上下文一致性和代码质量。

### 可视化管理面板的主要内容

SharedCore / Project Overview

项目开发状态 · 示意数据

运行中

## 3 / 4

活动 Agent

## 12

上下文版本

## 8 / 15

完成任务

Agent 状态

Agent A · 身份认证模块

编码中

Agent B · 课程资源上传

测试中

Agent C · 知识图谱组件

编码中

实时事件

T-100 · API 契约已通过审查

Agent B · 收到接口版本更新通知

Agent A · 已提交新的集成候选

示意界面，数值不代表实际任务运行结果。

## 十二、需要特别防范的技术风险

| 风险                   | 对策                               |
| ---------------------- | ---------------------------------- |
| Agent 使用过时上下文   | 快照版本绑定、依赖失效通知         |
| 多 Agent 循环讨论      | 消息次数限制，设定最终决策者       |
| 上下文无限膨胀         | 分层检索、结构化摘要、Token 预算   |
| Agent 意外修改公共代码 | Worktree 隔离、写入权限校验        |
| 合并成功但功能冲突     | 契约测试、业务集成测试             |
| 重试造成重复操作       | 幂等键、持久化操作日志             |
| Agent 自行执行危险命令 | 沙箱、权限最小化、人工审批         |
| 并行成本高于收益       | 根据任务依赖和协调开销动态控制并发 |

安全方面，不能仅依靠 Prompt 告诉 Agent 不要执行危险操作。工作区边界、文件写入范围、网络权限、密钥访问以及生产环境部署权限，都需要通过执行环境和服务端策略强制实现。

尤其是从代码文件、网页或日志中检索出的内容，不应被当作高优先级系统指令。

## 十三、建议的最终方案

我建议把框架定位为：

SharedCore — Context-Centric Multi-Agent Development Framework

即：以共享项目上下文为中心，而不是以 Agent 之间的对话为中心的并行开发框架。

它与普通多 Agent 编排系统的关键区别应该体现在三个技术能力：

1. Context as State：上下文是具备版本、来源、事务与权限的共享项目状态，而不只是 Prompt。

2. Task as Contract：每个开发任务都具有明确输入、代码范围、依赖、输出与验收标准，具备可验证的并行执行条件。

3. Integration as Transaction：Agent 生成的代码只是候选产物，必须经过兼容性检查、集成验证与受控提交，才成为正式项目状态。

第一版最值得优先完成的不是复杂的 Agent 自治通信，而是下面这条完整链路：

`需求输入 → 任务 DAG → 核心上下文快照 → 并行 Coding Agent → 独立 Git Worktree → 自动集成测试 → 上下文更新`

只要这条链路跑通，框架就具备了实际的开发价值。之后再引入实时事件同步、语义冲突检测、复杂多模型调度等能力，系统会更容易保持稳定。

最终判断框架是否成功，也不应只看启动了多少个 Agent，而要看端到端交付时间、任务返工率、合并冲突率、测试通过率和单位合格交付成本是否优于单 Agent 开发。
