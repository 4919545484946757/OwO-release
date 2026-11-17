# Agent Plugins 1.0.0 兼容层

> 状态：已实现（`owo-agent-plugins::agent_plugins`，2026-11-18）。
> 权威规范：<https://agent-plugins.org/specification>（v1.0.0，TSC：Amazon / Cursor /
> Microsoft / OpenAI / Vercel / Google）。
> 事实优先级：源码与测试 > 本文。

## 1. 为什么是这一层

行业在 2026-08 发布了 **Agent Plugins 1.0.0**：`plugin.json` 清单 + `skills/` 技能目录 +
`mcp.json` 服务器声明，三件固定位置的东西就是一个可移植插件包。它把「Agent Skills」与
「MCP 服务器」这两类已经标准化的组件装进一个包，解决的是**分发格式**问题，而不是再造
一套运行时。

这正好接上 OwO 的插件理念（“一切皆插件、能力可声明可审批”）：

| 关注点 | 谁负责 |
|---|---|
| 包格式、组件位置、清单校验 | **Agent Plugins 1.0.0**（本兼容层实现） |
| 生命周期（安装/更新/回滚/吊销）、签名、风险扫描、市场 | OwO `PluginManager`（历史格式，治理更重） |
| 执行授权、沙箱、审批、审计 | `owo-agent-tool-safety` / Tool Host（**不因插件格式而放宽**） |

所以本层只做「读 + 校验 + 映射」，不执行、不联网、不落盘。

## 2. 实现范围

| 规范条款 | 实现 | 证据 |
|---|---|---|
| §4.1 包边界与路径包含（含 symlink/junction） | ✅ 清单/组件/`SKILL.md`/`cwd` 逐级做规范路径包含检查 | `path_is_within`、`resolve_within` |
| §4.1 窄失败边界（拒绝插件 / 废组件 / 跳过技能 / 跳过服务器） | ✅ 五级边界分别实现 | `load_agent_plugin` + 各 `discover_*` |
| §5.2 清单闭模式 | ✅ 未知**顶层**字段：报告后忽略（非致命）；其它 schema 违规：拒绝 | `AgentPluginManifest::from_value` |
| §5.2 `$schema` 选择校验规则、**不联网取 schema** | ✅ 只认 1.0.0 规范标识，其它版本直接拒绝 | `manifest_unsupported_schema` |
| §5.3 必填 `$schema` / `name` | ✅ | — |
| §5.5 插件名约束（1–64、`[a-z0-9.-]`、首尾字母数字、禁 `--` `..`） | ✅ | `validate_plugin_name` |
| §5.4 `author` 闭字段、元数据只按 JSON 类型校验 | ✅ | `optional_author` |
| §6 固定位置、缺失不报错、错误类型只废该组件 | ✅ | `discover_skills` / `load_mcp_servers` |
| §7.1 技能只找 `skills/` 直接子目录的 `SKILL.md`，不递归 | ✅ | `skills_are_discovered_only_in_direct_child_directories` |
| §7.1 技能不符合 Agent Skills 规范则跳过 | ✅ 校验 `name` / `description`（长度与字符集） | `parse_skill_frontmatter` |
| §7.2 `mcp.json` 顶层闭模式、`$schema` 与清单同版本 | ✅ 不一致即禁用该插件的 MCP | `mcp_schema_must_match_the_manifest_version` |
| §7.2.1 stdio 闭变体（`type/command/args/env/cwd`） | ✅ | `parse_stdio_server` |
| §7.2.1 `command` 单 token（裸名或 `./` 包内路径） | ✅ 绝对路径/`../` 一律拒绝 | — |
| §7.2.1 `env` 禁保留变量、值做占位符展开 | ✅ `PLUGIN_ROOT`/`PLUGIN_DATA` 不可被声明 | `reserved_env_variables_are_rejected` |
| §7.2.1 `cwd` 三种形态 + 展开 + 包含校验；缺省 = 插件根 | ✅ | `resolve_cwd` |
| §7.2.1 HTTP 变体 `streamable-http` + 头校验 | ✅ 大小写不敏感去重、控制字符拒绝 | `parse_http_server` / `validate_header` |
| §7.2.1 HTTP 变体 `sse` | ⛔ 明确跳过（§7.2.2 第 4 条）：本客户端 HTTP 传输是 Streamable HTTP，不是 2024-11-05 的 GET /sse | `mcp_server_skipped` 诊断 |
| §7.2.1 URL：绝对 http(s)、无 userinfo/fragment、非回环必须 HTTPS | ✅ | `validate_mcp_url` |
| §7.2.2 条目级失败只跳过自己 | ✅ | `invalid_server_is_skipped_without_losing_siblings` |
| §8 扩展命名空间 | ✅ 原样保留在 `manifest.extensions`，不解释内容 | `extension_namespaces_are_preserved_verbatim` |
| 客户端扩展顶层目录（`com.example.client/`） | ⛔ 不赋予语义（规范允许客户端只做清单侧） | — |

**未实现（明确取舍）**：

- OAuth / 凭据引用：规范本身不定义，留给客户端；当前 HTTP 服务器不做授权发现。
- `extensions` 里**非对象**的成员值按 §5.2 判为 schema 违规（致命）；只有「`extensions`
  整体不是对象」是非致命的。
- 规范包不参与 `PluginManager` 的签名/市场流程。需要签名的分发仍用历史 `manifest.json`
  格式；这是刻意的：规范包定位是「可移植」，签名与吊销是 OwO 自己的治理增强。

## 3. 包长什么样

```text
owo-demo-kit/
├── plugin.json          # 必填：$schema + name
├── skills/
│   └── demo-kit/
│       ├── SKILL.md     # 必填：YAML frontmatter（name/description）
│       ├── scripts/     # 可选
│       └── references/  # 可选
└── mcp.json             # 可选：stdio / streamable-http / sse
```

随包示例：`plugins/owo-demo-kit/`（一个技能 + 一个 stdio 服务器，Python 标准库实现）。

## 4. 加载流程

```text
plugins/<pkg>/plugin.json 存在
    → load_agent_plugin()
        1. 规范化包根；读 plugin.json
        2. 清单校验（闭模式 / $schema / name / author / extensions）
        3. 发现 skills/（直接子目录，逐个校验，越界跳过）
        4. 读 mcp.json（顶层闭模式、版本一致）
             → 逐服务器校验并映射为 McpServerConfig
             → 条目级失败只跳过自己
        5. 产出 LoadedAgentPlugin { manifest, skills, servers, diagnostics }
    → agent_plugin_to_manifest() 转成宿主内部 PluginManifest
    → 既有链路：信任门 → 合并 MCP 配置 → 沙箱门卫 → 连接/注册工具
```

服务器名带上插件名（`<plugin>/<server>`），因此两个插件声明同名 server 不会互相顶替工具
命名空间（`mcp__<sanitized_server>__<tool>`）。

诊断码是稳定契约，日志与测试都按码断言：
| code | 级别 | 含义 |
|---|---|---|
| `plugin_loaded` | info | 加载完成（带组件计数） |
| `manifest_unknown_field` | warning | 未知顶层字段已忽略 |
| `manifest_extensions_not_object` | warning | `extensions` 非对象已忽略 |
| `skill_invalid` / `skill_escape` / `skill_unreadable` | warning | 该技能被跳过 |
| `mcp_disabled` | warning | 该插件的 MCP 组件整体禁用（配置级问题） |
| `mcp_server_skipped` | warning | 该服务器条目被跳过（条目级问题） |
| `manifest_*` / `plugin_name_invalid` | error | 插件被拒绝（`AgentPluginError.code`） |

## 5. 验收

```powershell
# 规范层契约测试（28 条；含随包示例的持续回归）
cd agent-sdk
$env:RUSTUP_TOOLCHAIN='1.97.1-x86_64-pc-windows-msvc'
cargo test -p owo-agent-plugins --locked -j 2 -- --test-threads=2

# 手工校验一个规范包（不启动 Daemon）
cargo run -p owo-agent-cli -- plugin validate plugins/owo-demo-kit

# 目录清单会标出包格式与服务器数量（注意参数顺序：--workspace 在子命令之前）
cargo run -p owo-agent-cli -- plugin --workspace . catalog
```

`plugin check/verify` 遇到 `plugin.json` 会自动走规范校验；`plugin validate` 是该入口的
显式别名。

## 6. 与历史格式的关系

| | 历史 `manifest.json` | Agent Plugins `plugin.json` |
|---|---|---|
| 清单 | 扁平字段（id/name/version/permissions/mcp/signature） | 闭模式元数据 + `extensions` 命名空间 |
| MCP | 单个 `mcp` | `mcp.json` 多服务器 |
| 技能 | 独立 `skills/` 目录（宿主技能库） | 包内 `skills/`（随插件分发） |
| 签名/市场/吊销 | ✅ | ⛔（按未签名包处理） |
| 安装/更新/回滚 | ✅ `PluginManager` | 目录发现（放入 `plugins/` 即生效） |

两种格式可以并存：同一目录里 `plugin.json` 优先；不同目录互不影响。宿主侧统一用
`PluginManifest::all_mcp_servers()` / `plugin_mcp_configs()` 取服务器，调用方不必区分格式。

## 7. 安全边界

- 规范包**不绕过**任何既有闸门：工作区未信任不加载插件；MCP 子进程仍经 SandboxManager
  授权并挂 Job 对象；工具调用仍走 Tool Host 的权限判定与审计。
- `env` 是凭据通道（不进命令行、不进审计明文）；`plugin_api` 的展示字段**不含**
  `env`/`headers`。
- HTTP `headers` 只发给配置的 origin；规范禁止把凭据放进 `headers`，客户端也不跨重定向
  转发配置头。
- 所有包内路径在读取前做规范化包含检查；符号链接只有在解析后仍在包根内时才接受。
- 静态扫描（`scan_agent_plugin_for_risks`）只读**可执行内容**：技能 `scripts/`、
  `servers/` 与常见脚本后缀（单文件 ≤256 KB、总量 ≤1 MB）。清单里的 `$schema`、
  `homepage`、`repository` 是元数据，不参与网络域判定——否则每个合规包都会因为
  `$schema` 被误报成「域外网络」。
