# OwO 插件 API v1 开发指南

本文说明 OwO 当前可安装的插件来源、运行时、标准服务和权限模型。API v1 支持 Windows 11 上的独立进程插件与声明式资源插件；插件代码不会加载进 TSF 或 Core Service 进程。

插件化仓颉、五笔、外文输入方案见[输入方案接入说明](input-schemes.md)。目前已开放实验性隔离进程通信、独立授权、异步候选、设置中心方案选择和 TSF 接线；只支持通过安全检查的原生输入宿主，完整词典与跨宿主兼容仍需后续验证。`input.scheme` 足以接收本方案编码并生成/选择候选，不等于周边上下文、系统操作或全盘访问权限；已授权方案的沙箱内存上限为 512 MiB。

## 1. 可安装来源

设置中心接受三种来源：

- `.owopkg`：推荐的发行格式，本质是受限制的 ZIP 容器。
- `.zip`：使用与 `.owopkg` 完全相同的检查规则，不能包含任意 ZIP 扩展特性。
- 文件夹：适合本地开发；选择后会先将普通文件复制到不可变内存快照，再进入与压缩包相同的清单、签名和授权流程。

所有来源都必须在根目录放置 `manifest.json`。压缩包和文件夹最多 1024 个文件，单文件不超过 64 MiB，未压缩总量不超过 256 MiB。绝对路径、`..`、Windows 设备名、大小写冲突路径、符号链接和重解析点会被拒绝。文件夹安装不会直接从原目录运行，也不会在检查后重新读取已变化的源文件。

未签名文件夹、第三方包以及申请高权限的包不会因为格式放宽而自动受信：设置中心仍要求风险与免责确认、逐包摘要确认和权限授权，安装后保持停用，需由用户再次启用。

## 2. 目录与清单

最小结构：

```text
my-plugin/
├─ manifest.json
├─ config.schema.json
└─ bin/
   └─ my-plugin.exe
```

`manifest.json` 使用封闭的 API v1 字段集合：

```json
{
  "id": "com.example.owo.demo",
  "name": "Demo Plugin",
  "version": "1.0.0",
  "api_version": 1,
  "runtime": "process",
  "entry": "bin/my-plugin.exe",
  "permissions": ["candidate.transform", "config.read"],
  "network": false,
  "config_schema": "config.schema.json"
}
```

`id` 必须是小写反向域名形式，`version` 必须是三段数字版本。进程插件使用
`runtime: "process"` 和包内 `bin/*.exe` 入口；纯资源插件使用
`runtime: "resource"` 和包内 `resources/*.theme` 或 `resources/*.palette` 入口。
资源插件不启动代码，只允许申请一个与入口类型对应的 `ui.candidate_theme` 或
`ui.unicode_palette` 权限。`network` 只有在同时声明 `network.client` 时才可设为
`true`；要求普通 Win32 用户令牌的能力还必须声明 `system.full_trust`。

### 声明式设置页

设置中心会为每个已安装插件建立独立侧栏入口，并以插件包内的
`config_schema` 生成原生 WinUI 3 设置页。页面不会加载插件提供的 DLL、XAML
或脚本，因此第三方插件不能借设置界面进入设置中心进程。

当前支持 JSON Schema 风格的封闭字段集合：顶层 `title`、`description`、
`type: "object"` 和 `properties`；属性支持 `boolean`、`string`、`integer`、
`number`，以及可选的 `title`、`description`、`default`、`minimum`、
`maximum`、`multipleOf` 和 `enum`。字符串还可使用 `format: "color"`，设置中心会
显示原生颜色选择器并保存 `#RRGGBB`。例如：

```json
{
  "title": "Demo Plugin 设置",
  "description": "这些选项只属于当前插件。",
  "type": "object",
  "properties": {
    "enabled": {
      "type": "boolean",
      "title": "启用增强候选",
      "description": "允许插件处理 Core 明确传入的候选页。",
      "default": true
    },
    "result_limit": {
      "type": "integer",
      "title": "最大结果数",
      "default": 8,
      "minimum": 1,
      "maximum": 32
    },
    "style": {
      "type": "string",
      "title": "显示样式",
      "default": "compact",
      "enum": ["compact", "comfortable"]
    }
  }
}
```

配置保存到插件专属数据目录的 `settings.json`。字段名最多 64 个 ASCII
字母、数字、点、下划线或连字符；单个结构和配置文件最大 256 KiB，最多
64 个字段、32 个枚举值。设置中心的全局搜索会索引插件名称、字段标题和说明。

## 3. 启动契约

OwO 使用以下参数启动插件：

```text
my-plugin.exe --owo-plugin-pipe <pipe> --owo-plugin-id <id> --owo-plugin-data <absolute-data-path>
```

插件只能将持久数据写入 `--owo-plugin-data` 指定的目录。沙箱进程还会收到同值的 `OWO_PLUGIN_DATA`、`TEMP` 和 `TMP`。获准完整信任时额外设置 `OWO_PLUGIN_FULL_TRUST=1`；这个标记不代表管理员权限，也不允许绕过 UAC。

插件连接命名管道后必须先发送 `hello_request`。Core 返回 `hello_response`，当前能力为 `cancel.v1` 和 `invoke.v1`。随后 Core 可发送 `invoke_request`、`cancel_request` 和 `shutdown_request`。插件分别返回 `invoke_response` 或 `acknowledgement`。同一插件同一时刻最多处理一个调用。

公共 C++ 协议定义位于：

- `include/owo/plugin/plugin_protocol.h`
- `include/owo/plugin/plugin_pipe.h`
- `include/owo/plugin/plugin_services.h`
- `include/owo/plugin/plugin_permissions.h`

`apps/example_process_plugin/main.cpp` 是可编译的握手、调用、取消与关闭参考实现。单条 payload 上限为 256 KiB，服务名必须带版本后缀（例如 `.v1`）。自定义接口应使用开发者前缀，例如 `com.example.search.v1`。

## 4. 标准服务

标准服务由 `plugin_services.h` 定义。Core 调用已知服务时会自动补入所需权限，PluginHost 会再次比对当前 manifest 和精确版本授权记录；插件或调用方无法通过漏报权限绕过检查。

| 服务 | 所需权限 | 用途 |
| --- | --- | --- |
| `owo.agent.exchange.v1` | `agent.ipc` | 交换经过宿主确认、限长和脱敏的 Agent 请求 |
| `owo.health.check.v1` | 无 | 检查插件是否就绪 |
| `owo.lifecycle.event.v1` | 无 | 接收启动、停用等生命周期事件 |
| `owo.command.execute.v1` | 无 | 执行插件声明的命令 |
| `owo.dictionary.lookup.v1` | 无 | 对显式查询返回词典结果 |
| `owo.candidate.transform.v1` | `candidate.transform` | 变换 Core 明确传入的一页候选词 |
| `owo.settings.schema.v1` | `config.read` | 返回声明式设置结构 |
| `owo.settings.read.v1` | `config.read` | 读取插件自身设置 |
| `owo.settings.write.v1` | `config.write` | 修改插件自身设置 |
| `owo.notification.show.v1` | `notification.show` | 显示用户通知 |
| `owo.ui.settings-page.v1` | `ui.settings_page` | 提供声明式设置页 |
| `owo.ui.candidate-theme.v1` | `ui.candidate_theme` | 提供声明式候选框主题资源 |
| `owo.ui.unicode-palette.v1` | `ui.unicode_palette` | 提供经过校验的分类 Unicode 码点列表 |
| `owo.ui.overlay.v1` | `ui.overlay` | 控制已授权的 UI 覆盖层 |
| `owo.ui.desktop-pet.v1` | `ui.desktop_pet` | 控制已授权的桌面宠物 |
| `owo.resource.dictionary.install.v1` | `resource.dictionary.install` | 提供字典资源安装操作 |
| `owo.resource.theme.install.v1` | `resource.theme.install` | 提供主题资源安装操作 |
| `owo.resource.material.install.v1` | `resource.material.install` | 提供材质资源安装操作 |
| `owo.resource.model.install.v1` | `resource.model.install` | 提供排序模型安装操作 |
| `owo.resource.sound.install.v1` | `resource.sound.install` | 提供声音资源安装操作 |

payload 在传输层是不透明 UTF-8 字节串。标准服务建议使用 UTF-8 JSON，并在顶层携带 `schema_version: 1`。插件必须拒绝未知字段或未知 schema；返回失败时使用 `invalid_request`、`permission_denied` 或 `plugin_error`，不要将敏感输入写入 `diagnostic`。

候选变换请求示例：

```json
{
  "schema_version": 1,
  "composition": "nihao",
  "candidates": [
    {"text": "你好", "consumed_syllables": 2},
    {"text": "拟好", "consumed_syllables": 2}
  ]
}
```

响应只返回变换后的显式候选数据，不应请求或推断未提供的窗口文本。Core 保留最终校验、去重、长度限制和上屏控制权。

## 5. 权限目录

除既有剪贴板、输入上下文/提交/替换、文件系统、网络、进程、屏幕、麦克风、桌宠、覆盖层、字典/主题/材质和完整信任权限外，API v1 现提供：

| 权限 | 风险 | 说明 |
| --- | --- | --- |
| `candidate.transform` | 需注意 | 处理 Core 明确传入的候选页 |
| `input.scheme` | 高 | 提供输入方案，接收该方案本次编码、光标、分页和候选选择；不授予周边上下文、直接上屏或系统操作权限 |
| `config.read` | 低 | 读取插件自己的配置 |
| `config.write` | 高 | 修改插件自己的配置 |
| `notification.show` | 需注意、完整信任 | 显示系统通知 |
| `resource.model.install` | 高、完整信任 | 安装排序模型资源 |
| `resource.sound.install` | 高、完整信任 | 安装声音资源 |
| `ui.settings_page` | 需注意 | 提供声明式设置页面 |
| `ui.candidate_theme` | 低 | 更改候选框语义颜色，不执行插件代码 |
| `ui.unicode_palette` | 低 | 在候选框中提供分类 Unicode 字符，不执行插件代码 |

任何需要完整信任的权限都必须同时声明 `system.full_trust`。权限在插件升级、manifest 变化、包摘要变化或发布者绑定变化后不会自动继承。TSF 输入线程不能直接调用插件；携带敏感权限的调用只接受 Core 内部标记的可信用户操作来源。

## 6. 构建与调试建议

1. 先复制 `apps/example_process_plugin`，保留握手和取消逻辑。
2. 使用文件夹安装进行本地迭代；每次内容变化都会形成新的清单摘要并重新授权。
3. 只声明实际使用的权限。开发阶段也不要依赖父进程环境变量或当前工作目录。
4. 服务处理必须支持超时与取消；不要在输入线程执行磁盘扫描、联网或模型初始化。
5. 发布时再制作 `.owopkg` 并生成覆盖规范化清单摘要的 `signature.json`。

本仓库的官方插件可统一构建与打包：

```powershell
cmake --build --preset windows-release
.\scripts\package_plugins.ps1
```

产物位于 `artifacts/plugins`。包名使用 `<plugin-id>-<version>.owopkg`。发布后不得在
版本号不变的情况下替换包内容；任何可见行为、二进制或包内文档变化都应提升三段式
版本，并使用户重新核对摘要和权限。Agent 连接器还可随参考协议与模拟服务一起打包：

```powershell
.\scripts\package-agent-sdk.ps1 -Version 1.3.0
```

### Agent 类插件的边界

API v1 提供的是受控 Agent IPC 桥接能力，不是内置的通用 Agent 运行时，也不等同于
Codex 的技能、MCP 或工具执行环境。官方 `org.owo.agent-ipc` 只完成必要上下文收集、
脱敏、结构化 IPC、状态展示和用户选择回传。第三方 Agent 自行负责模型、工具注册、
身份验证、审计以及高风险操作确认，不能把 OwO 的 `system.full_trust` 当作自身的系统
授权。完整协议见 `agent-ipc-integration.md`。

Agent 协议 v3 的 `tasks.slots` / `tasks.incremental` 能力允许插件返回任务草稿、槽位与
原拼音范围的映射，以及候选对应的最小槽位补丁。宿主按任务版本校验补丁，保护已锁定
槽位，并把未消耗范围留在组合态。插件不得把自然语言标签反向解析成槽位或执行参数。

## 7. 候选框主题资源

候选框皮肤应使用 `runtime: "resource"`，入口为 UTF-8 的 `resources/*.theme`。
资源采用封闭的 `schema_version=1` 键值格式，必须为以下七个语义角色同时声明
`color=#RRGGBB` 与 `opacity=0..1`：`background`、`border`、`divider`、
`pinyin_text`、`candidate_text`、`candidate_block`、`candidate_number`。

主题还可声明以下可选视觉效果：`background_blur.enabled`、
`background_blur.radius`（0..24）、`shadow.enabled`、`shadow.color`、
`shadow.opacity`、`shadow.blur_radius`（1..32）、`shadow.offset_x` 与
`shadow.offset_y`（-16..16）。背景模糊由本体采样候选框后方画面并执行本地高斯卷积；
阴影由独立、不可交互的分层窗口绘制，插件仍不获得屏幕读取或窗口控制接口。

设置页可用同名的 `<role>_color` 和 `<role>_opacity` 字段覆盖资源默认值。TSF 只读取
安装器已验证且已启用的资源快照，不加载插件代码；配置每秒最多检查一次，变化后原子
替换 Direct2D 画刷。多个候选主题同时启用时，按插件 ID 排序后使用第一个有效主题。
视觉效果对应的设置字段为 `background_blur_enabled`、`background_blur_radius`、
`shadow_enabled`、`shadow_color`、`shadow_opacity`、`shadow_blur_radius`、
`shadow_offset_x` 和 `shadow_offset_y`。

官方示例位于 `plugins/candidate-glass`，所有项目维护插件统一放在 `plugins/`，可用
`scripts/package_plugins.ps1` 批量生成 `.owopkg`。

## 8. Unicode 符号列表资源

Unicode 列表使用 `runtime: "resource"`、`ui.unicode_palette` 权限和 UTF-8 的
`resources/*.palette` 入口。文件是封闭的逐行格式：

```text
schema_version=1
category=math|数学符号
item=00B1|正负号
item=221E|无穷
```

分类 ID 只能使用小写 ASCII 字母、数字、`-` 和 `_`；分类标签和字符名称使用 UTF-8。
`item` 的第一段是一个 Unicode 标量的十六进制码点，代理项和大于 `U+10FFFF` 的值会被
拒绝。宿主最多接受 32 个分类、每类 512 项和 256 KiB 资源数据。插件不能提供脚本、
组合序列或上屏逻辑；候选框本体负责入口、分类切换、滚轮/按键翻页和点击上屏。

官方示例位于 `plugins/unicode-palette`。

插件格式的安全边界参见 `docs/security/plugin-baseline.md`，签名与库存摘要格式参见 `docs/adr/0008-plugin-package-and-trust-foundation.md`，授权和运行时隔离分别参见 ADR 0011 至 0017。
