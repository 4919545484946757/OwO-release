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
| §8 客户端扩展：**hooks 贡献点** | ✅ `com.owo.agent` 命名空间（清单内 `hooks` 数组与 `com.owo.agent/hooks.json` 两个来源） | `hooks_are_loaded_from_the_extension_namespace` |
| 客户端扩展顶层目录（其他命名空间） | ⛔ 不赋予语义（规范允许客户端只做清单侧） | — |

**未实现（明确取舍）**：

- OAuth / 凭据引用：规范本身不定义，留给客户端；当前 HTTP 服务器不做授权发现。
- `extensions` 里**非对象**的成员值按 §5.2 判为 schema 违规（致命）；只有「`extensions`
  整体不是对象」是非致命的。
- 规范包**可经市场安装与更新**（`/plugins/market/install|update|verify|scan` 识别
  `plugin.json`，走同一套前置风险扫描、安装审计与旧版备份）；**吊销列表对规范包同样
  生效**（按包名 + 版本，`*` = 全部版本）。规范包另有**可选签名层**（见 §3）：存在则
  必须通过，缺失按未签名包处理（审计留痕），签名是 OwO 自己的治理增强。

## 3. 包长什么样
```text
owo-demo-kit/
├── plugin.json          # 必填：$schema + name
├── plugin.sig.json      # 可选：ed25519 签名（覆盖包内容，OwO 扩展）
├── skills/
│   └── demo-kit/
│       ├── SKILL.md     # 必填：YAML frontmatter（name/description）
│       ├── scripts/     # 可选
│       └── references/  # 可选
└── mcp.json             # 可选：stdio / streamable-http / sse
```

**可选签名**：`plugin.sig.json` = `{"algorithm":"ed25519","public_key":"<b64>","signature":"<b64>"}`，
签名对象是**包摘要**——按相对路径排序，对「路径 \0 文件 sha256 \n」整体再做 sha256，
排除签名文件本身、隐藏目录与符号链接。签名存在即必须通过，缺失按未签名包处理。

回滚：owo-agent plugin rollback <id>（默认取最新备份；备份由 update 自动产生，目录名记录被替换的版本，如 demo-1.0.0）。CLI 与市场共用签名开关 OWO_PLUGIN_REQUIRE_SIGNATURE=0（默认要求签名）。

作者侧签名：`owo-agent plugin sign <包目录> --key <私钥文件>`（私钥支持 raw 32 字节 /
hex / base64，只读入内存、不落盘）；`owo-agent plugin check <包目录>` 会显示
「签名：ed25519 校验通过」或「签名：校验失败」。

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
        5. 读 com.owo.agent 扩展命名空间（清单内 hooks + 扩展目录 hooks.json）
        6. 产出 LoadedAgentPlugin { manifest, skills, servers, hooks, diagnostics }
    → agent_plugin_to_manifest() 转成宿主内部 PluginManifest
    → 既有链路：信任门 → 合并 MCP 配置 → 沙箱门卫 → 连接/注册工具
    → hooks：与 settings hooks 合并（AppState 分开保存两半）
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
| `plugin_hook_skipped` | warning | 该 hook 条目被跳过（裸命令/越界/未知事件/坏参数） |
| `plugin_hooks_truncated` | warning | hook 数超过上限，超出部分忽略 |
| `plugin_hooks_not_array` / `plugin_hooks_file_invalid` | warning | hooks 数据形状不对，已忽略 |
| `manifest_*` / `plugin_name_invalid` | error | 插件被拒绝（`AgentPluginError.code`） |

## 5. 插件贡献 hooks（客户端扩展）

规范 §8 允许客户端用反向域名命名空间放自己的扩展数据。OwO 用 `com.owo.agent` 让
插件包声明**生命周期钩子**——这是"插件不仅能加工具，还能加行为"的落点：

```json
{
  "extensions": {
    "com.owo.agent": {
      "hooks": [
        { "event": "pre_tool_use", "matcher": "desktop_*", "command": "./hooks/guard.cmd", "args": ["--strict"], "fail_mode": "closed" },
        { "event": "user_prompt_submit", "command": "${PLUGIN_ROOT}/hooks/guard.cmd" }
      ]
    }
  }
}
```

等价写法：包根 `com.owo.agent/hooks.json`（同样是一个数组），两个来源会合并。
`plugins/owo-demo-kit/com.owo.agent/hooks.example.json` 是**不会被加载**的示例（文件名不是
`hooks.json`），需要时改名即可。

### 字段与约束

| 字段 | 必填 | 约束 |
|---|---|---|
| `event` | ✅ | `pre_tool_use` / `post_tool_use` / `user_prompt_submit` / `stop` / `pre_compact`（与 core `HookEvent` 由测试锁死同步） |
| `command` | ✅ | **必须是包内脚本**：`./x` 或 `${PLUGIN_ROOT}/x`；裸命令与越界路径一律拒绝（不能借 PATH 执行包外程序） |
| `args` | ❌ | 字符串数组；`${PLUGIN_ROOT}`/`${PLUGIN_DATA}` 会展开；含引号/换行的参数拒绝 |
| `matcher` | ❌ | 工具名通配（`*` / `desktop_*`）；无工具上下文的事件恒匹配 |
| `fail_mode` | ❌ | `open`（缺省）/ `closed`（失败即阻断） |

脚本按扩展名选解释器：`.ps1` → `powershell -NoProfile -ExecutionPolicy Bypass -File`，
`.py` → `python`，`.js/.mjs/.cjs` → `node`，其余按可执行文件直接运行。单包最多
`MAX_PLUGIN_HOOKS`（32）条，超出部分忽略并记 `plugin_hooks_truncated`。

### 与宿主 hooks 的关系

- 与 `settings.json` 的 hooks **同一执行器**：stdin 收事件 JSON、exit 2 = 阻断、10s 超时。
- 来源可追溯：插件 hook 的 `source` 为 `plugin:<包名>`，阻断消息里带该标签；设置页的
  hook 来源为 `settings`。
- 两半分开存放（`AppState::HookSources`）：设置页热重灌只替换 settings 那一半，
  **不会静默清掉插件 hooks**；撤销工作区信任会同时清空两半。
- 信任边界不放松：工作区未信任时插件根本不加载；插件 hooks 与插件 MCP 子进程同级，
  都属于"配置即代码"，需要显式信任。
- 已知边界：桌面端"禁用插件"目前只隐藏工具前缀，**已装载的 hooks 要重启 Daemon 才失效**
  （与插件 MCP 子进程同一生命周期策略：不在运行中的进程里热拔代码）。

## 6. 验收

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

## 7. 与历史格式的关系

### 7.1 安装与治理

规范包与历史格式共用同一套安装生命周期（`PluginManager`）：

```powershell
owo-agent plugin install <包目录>     # 校验 → 拷贝到 <data>/plugins/<name> → 激活
owo-agent plugin update <name> <目录> # 备份旧版 → 校验新版 → 替换；失败自动回滚
owo-agent plugin uninstall <name>
```

规范包在 `verify_plugin_dir` 里走独立分支（`verify_agent_plugin_dir`），审计逐项留痕：

| 校验项 | 规范包的行为 |
|---|---|
| 清单 | `load_agent_plugin` 规范校验；组件级降级写进审计 |
| 吊销 | 与历史格式同口径（id + version，`*` 匹配全部） |
| 路径 | 与历史格式同口径：拒绝 zip-slip 变体与符号链接 |
| 签名 | 规范包**没有签名字段**：不参与签名策略，审计明确记「按未签名包处理」，不谎报已验签 |
| 最低版本 | 规范没有该字段，跳过 |
| 静态扫描 | 只扫可执行内容（技能 `scripts/`、`servers/`、脚本后缀） |
| 沙箱门卫 | 每个 MCP 服务器都要过（与历史格式同一出口） |

| | 历史 `manifest.json` | Agent Plugins `plugin.json` |
|---|---|---|
| 清单 | 扁平字段（id/name/version/permissions/mcp/signature） | 闭模式元数据 + `extensions` 命名空间 |
| MCP | 单个 `mcp` | `mcp.json` 多服务器 |
| 技能 | 独立 `skills/` 目录（宿主技能库） | 包内 `skills/`（随插件分发） |
| 签名/市场/吊销 | ✅ | ⛔（按未签名包处理） |
| 安装/更新/回滚 | ✅ `PluginManager` | ✅ 同一生命周期：`plugin install/update/uninstall`（规范包走 `verify_agent_plugin_dir`） |

两种格式可以并存：同一目录里 `plugin.json` 优先；不同目录互不影响。宿主侧统一用
`PluginManifest::all_mcp_servers()` / `plugin_mcp_configs()` 取服务器，调用方不必区分格式。

## 8. 安全边界

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
