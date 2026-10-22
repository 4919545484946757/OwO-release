# 开发 CLI：隔离调试与测试

入口是项目根目录 `owo-dev.cmd`，可从任意工作目录调用。原生程序位于 `build/windows-developer/Release/owo_dev_cli.exe`。Windows 11 x64 为当前验证平台；开发者需要 MSVC/CMake，不需要先注册 TSF 或安装 OwO。

## 开发模式与数据边界

- 只有显式开启 `OWO_BUILD_DEVELOPER_TOOLS=ON` 的 `windows-developer` 预设编译此程序；默认发行构建不产生它，发行包文件白名单也不包含它。
- 这里的开发模式指 OwO CLI 的构建/会话模式，不会开启或关闭 Windows 系统的“开发人员模式”。`doctor` 只读核对会话配置、插件安装记录，并报告进程位数、原生架构、配置/协议版本和候选页大小；不扫描应用输入记录。
- 除 `help/version/build/test` 外，必须显式传 `--dev --root <绝对目录>`。首次 `init` 只接受新的或空目录，创建版本标记；后续命令只接受该标记的目录。未知参数、重复参数、越界数值均拒绝。
- 不接受日常 OwO 数据、安装目录及其祖先作为根目录；禁止盘符根、重解析点与受保护目录。每个会话持有独占锁，并发操作同目录会失败，而不是覆盖学习数据。
- 私有配置、插件商店、插件数据、可选学习记录均留在该目录。内置输入调试启动本 CLI 进程内的私有 Core，使用唯一命名管道，退出时只关闭自己的服务。不连接、重启、注册或替换正在使用的 OwO。
- 插件仍走原安装快照、精确版本/摘要绑定、授权、AppContainer 或明确授权的完整信任、调用超时与取消规则。开发模式不授予权限，也不跳过 UAC。`--dev` 和目录标记是防误操作机制，不是抵御同账户恶意程序的身份认证边界。
- Windows AppContainer 配置按插件 ID 共享。安装、启动、启停、撤权和卸载前，只读检查日常商店是否存在同名 ID；存在则拒绝，请为测试包使用唯一开发 ID。不会读取日常插件设置内容。不同开发根目录也应使用不同 ID；不要同时在日常商店安装正在调试的同名包。
- 所有“上屏”仅是模拟结果字符串，不调用 SendInput、剪贴板、应用窗口或真实 TSF 提交接口；不收集焦点应用、屏幕、密码框或周边文字。`--sensitive` 用于验证方案拒绝敏感请求。

高权限插件仍可能在用户授权的权限下访问外部系统。`--run-plugin` 是启动代码的显式确认，不代替安装授权；标准敏感服务还需 `--user-action`，且 Host 会再次核对包权限。它不是高风险系统操作的一揽子确认。不要在开发目录放真实密码、凭据或聊天记录。

## 快速开始

```powershell
# 编译专用 CLI；不会编译/注册到日常输入法部署目录
D:\0program\OwO\owo-dev.cmd build

# init 只执行一次；以后复用目录时使用 doctor
D:\0program\OwO\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual init
D:\0program\OwO\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual doctor

D:\0program\OwO\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual input parse --text chnag
D:\0program\OwO\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual input query --text nihao
```

不指定 `--lexicon` 时，私有 Core 只使用小型示例词典（你好、世界、西安等），不能用它评价真实词库覆盖率。真实候选测试应明确指定只读词典：

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual input query `
  --text rangxuexijilu --lexicon D:\0program\OwO\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx
```

根目录必须是绝对路径；其他显式文件参数可为相对于调用者当前目录的路径。带空格的路径应加引号。原生程序与入口脚本支持中文路径、UTF-8 输出、Windows PowerShell 5.1；脚本源码使用 ASCII，避免旧 PowerShell 的无 BOM 中文解析问题。

## 内置输入链路

| 命令 | 内容 |
| --- | --- |
| `input parse --text ...` | 规范化、所有解析路径、路径类型、编辑次数、原始输入 begin/end 字节范围、音节完整性、分阶段微秒耗时 |
| `input query --text ...` | 复用真实 Core 的生成、分区、排序、缓存和协议校验，返回原始候选响应与消耗字节数 |
| `input repl` | 从标准输入交互式读取回放动作 |
| `input replay --file ...` | UTF-8 动作文件，支持期望断言；错误即非零退出 |
| `input bench --text ... --samples 20` | 私有 Core 首次连接/响应与同结果缓存热往返，分别统计；不含 UI 渲染 |
| `config show` | 读取会话配置（返回 `contents` 字符串） |
| `config import --file ...` | 使用真实配置解析器校验后原子保存到会话配置；不修改传入文件 |

解析支持 `--correction on|off`、`--paths 1..128`。查询支持 `--page 0..100`、`--expanded`、`--correction`、`--context`、`--previous`、`--lexicon`、`--timeout-ms`。展开请求必须从第 0 页开始。

内置拼音/编码测试限 256 字节；插件方案使用其自身编码及最多 16384 字节的契约，不被转换为拼音。`input parse` 专门检查全拼；插件方案用 `scheme` 命令。

这里不接入日常 ModelHost：基础候选/用户频率可复现，真实 libime 排序在独立 `model rank` 命令中检查。`--context` 只使用开发者显式给出的合成文本，不读取应用。

### 回放脚本

仓库提供 `tests/data/developer-basic.owotest`：

```text
query nihaoshijie
expect 你好世界
select 2
expect-input shijie
expect 世界
select 1
expect-commit 你好世界
```

该片段假定从空模拟提交记录开始、使用默认小词典与默认分区配置。`select` 是当前显示数组中从 1 开始的索引（包含展开后的数组），按 Core 返回的消耗字节数删除输入前缀，并对余串重新请求；不是按候选汉字数量推算拼音。

支持的动作：

| 动作 | 含义 |
| --- | --- |
| `query <输入>` | 替换输入、回到第 0 页并请求 |
| `type <字符>` | 在输入尾部追加后请求 |
| `backspace` | 删除最后一个 UTF-8 字符后请求 |
| `page <0起始页码>` / `expand` | 普通分页 / 展开 |
| `select <1起始索引>` | 模拟提交并消耗对应输入 |
| `correct on` / `correct off` | 切换当前纠错状态并重新请求 |
| `clear` | 清空当前组合态，不清除模拟已提交内容 |
| `expect <首候选>` | 检查首候选 |
| `expect-input <剩余输入>` | 检查剩余输入，允许空值 |
| `expect-commit <累计文本>` | 检查累计模拟提交内容 |
| `quit` | 结束；以 `#` 开头的整行为注释 |

每行是“动作 + 一个空格 + 原样 UTF-8 值”，不是 shell；不执行管道、脚本、转义或变量展开。最多 1 MiB、10000 步，每行最多 16384 字节。`repl` 同样读取这些动作，不实现终端内文字编辑器或真实 TSF 按键事件模拟。

默认不写学习反馈。仅在 `replay/repl --learn` 时，把选择产生的学习反馈写入会话 `user-frequency.owuf`，不会写入日常用户词库。后续会话查询可读取这里的学习记录。

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual input replay `
  --file .\tests\data\developer-basic.owotest
```

## 插件生命周期和服务调用

1. `plugin inspect --source <文件夹/.zip/.owopkg>`：只读检查包，输出身份、权限、风险需求与 `inventory`。
2. `plugin install --source ...`：受信包走正常安装；不受信或高权限包必须提供与刚才预览一致的摘要和单独同意。
3. `plugin activate --id ... --version ...`：启用精确版本；Agent 插件另需 `--accept-agent-privacy I_ACCEPT_AGENT_IPC_PRIVACY_V1`。
4. `plugin invoke ... --run-plugin`：经真实 Host 启动/握手、调用、超时/取消、退出。不能选择任意未安装 EXE，也不能伪造授权记录。
5. `plugin deactivate/revoke/uninstall --id ... --version ...`：分别停用、撤销、卸载；卸载仍须先停用，保留插件数据。

高风险安装示例（用你实际检查到的摘要及权限替换占位符，不要照抄省略号）：

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual plugin inspect --source D:\MyPlugin
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual plugin install --source D:\MyPlugin `
  --inventory <预览返回的完整SHA256> --accept-risk I_ACCEPT_PLUGIN_RISK_V1 `
  --accept-untrusted --permissions input.scheme
```

只有清单确实要求完整信任且你理解风险时，才增加 `--allow-full-trust` 并在 `--permissions` 中明确列出 `system.full_trust` 及实际需要的权限；不自动授权清单中所有权限。

`plugin list` 列举开发商店；`plugin audit` 只读报告恢复项，不自动清理。`plugin schema --id ...` 读取活动版本声明的配置结构；`plugin settings-read --id ...` 读取开发目录内的插件自身设置。CLI 不执行第三方设置 UI。进程插件的配置写入可通过其实现的 `owo.settings.write.v1` 服务调试（仍需声明/授权 `config.write` 和显式 `--user-action`）；未实现该服务时应返回插件错误，不伪装成功。资源插件设置可在开发目录手工编辑后用资源校验命令检查。

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual plugin invoke `
  --id com.example.plugin --service example.echo.v1 --payload-file .\request.json `
  --run-plugin --timeout-ms 5000 --cancel-after-ms 100
```

payload 文件按原始字节读取，最多 256 KiB。结果 `payload_hex` 始终保留精确字节；有效 UTF-8 时同时提供 `payload`，否则为 `null`，不会把二进制错误输出成无效 JSON。`status`、`forced_termination`、`diagnostic` 可区分拒绝、取消、超时、协议错误与崩溃。注意：代码执行后，Host 终止进程不能撤销插件已执行的外部操作。

## 插件化输入方案

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual scheme list `
  --id owo.plugin.input-method-example --run-plugin
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual scheme query `
  --id owo.plugin.input-method-example --scheme owo.plugin.input-method-example/table `
  --text a --page-size 2 --page 0 --run-plugin
```

示例插件须先自行构建、按清单组装文件夹、检查、安装和授予 `input.scheme`；见[输入方案文档](plugins/input-schemes.md)。这里不会自动安装或授权。

`scheme query` 支持 `--caret-byte`、`--select`、`--page`、`--page-size`、`--cancel-after-ms`、`--sensitive`；选择与查询位于同一受保护会话，候选 ID 来自真实响应。结果包含预览、页码、状态和每条候选原编码的半开 UTF-8 字节消费范围。撤权、跨包身份、非法字符、越界光标等由原 Router/Host 校验。`--sensitive` 应被拒绝且不返回可用候选。

## 资源、Agent 和协议

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual resource validate `
  --kind theme --file .\plugins\candidate-glass\resources\candidate.theme
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual resource validate `
  --kind palette --file .\plugins\unicode-palette\resources\unicode.palette
```

主题可加 `--settings <插件设置JSON>`，复用实际主题加载器；符号资源复用实际分类/码点解析器。这里只检查资源与数值，不绘制窗口，不证明材质、阴影和滚动视觉正确。

`protocol validate --kind core|plugin|agent-request|agent-response --file ...` 直接使用生产严格解码器。Core/Agent 文件为 UTF-8 文本；PluginHost 文件为原始二进制消息（不含管道的额外帧头），未知字段、版本、超限、截断和身份错误不会被宽松 JSON 解析掩盖。

`agent sample-request --text 测试 --pinyin ceshi` 生成可供第三方联调的 v3 请求：使用本 CLI 的合成应用标识、随机 session/request ID、空真实上下文，先过滤显式文本，不连接 Agent、不执行任何指令。`agent filter --file ...` 可单独检查文本脱敏；这不是完备的 DLP，仍须人工审查测试数据。现有模拟服务入口仍为 `start-agent-mock.cmd`，接入规范见[Agent 文档](plugins/agent-ipc-integration.md)。

## 模型独立诊断

候选文件每行一个 UTF-8 候选，最多 256 条、每条 4096 字节：

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual model rank `
  --backend mock --text ceshi --candidates-file .\candidates.txt `
  --mock-latency-ms 200 --timeout-ms 100 --cancel-after-ms 30

.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-manual model rank `
  --backend libime --run-model --bridge D:\TrustedRuntime\owo_libime_bridge.dll `
  --model D:\TrustedModel\zh_CN.lm --text wang --context 狂 --candidates-file .\candidates.txt
```

Mock 只用于确定性排序和超时/取消验证，不代表中文智能排序。libime 会将明确指定的原生桥接代码加载到此开发 CLI 进程，因此必须显式 `--run-model`，只用可信文件；不进入日常 Core/TSF，也不下载模型。结果报告状态、顺序和排序调用耗时（不含模型加载）。

## 自动化、输出和测试

- 管理结果与回放事件为 JSON Lines（`schema_version: 1`）；`input query` 忠实输出 Core 当前协议，`agent sample-request` 忠实输出 Agent v3，不能按同一外壳解析。
- `--trace` 显式打开库级结构化诊断到 stderr；默认不持久写日志。stdout 包含开发者显式提供的输入、候选或 payload，保存 CI 日志前自行检查敏感数据。
- 退出码：0 成功；1 断言/解析/资源/协议验证失败；2 参数或文件输入错误；3 开发门禁/路径/确认拒绝或会话忙；4 私有 Core/词典/IPC 故障；5 插件/输入方案/模型失败或取消；6 未处理的工具内部故障。
- `status` 沿用对应接口枚举：插件为 0成功/1非法请求/2权限拒绝/3版本不支持/4超时/5取消/6插件错误；输入方案为 0成功/1取消/2不可用/3错误；模型为 0成功/1取消/2超时/3后端错误。不能跨接口混用编号。
- 有状态命令串行使用同一根目录。CI 为每个作业创建独立空目录。失败不会自动清空开发目录，避免丢失复现数据。

```powershell
.\owo-dev.cmd test --suite cli
.\owo-dev.cmd test --suite engine
.\owo-dev.cmd test --suite plugin
.\owo-dev.cmd test --suite scheme
.\owo-dev.cmd test --suite agent
```

测试命令只在开发构建树内构建/运行选定 CTest 组，没有隐式全量自检。CLI 合同使用随机插件 ID、一次性目录及真实参考插件，验证门禁、中文/空格路径、无副作用拒绝、分段消费、回放断言、模型超时/取消、二进制调用、崩溃、方案分页/选择、敏感请求和撤权。它不会覆盖真实宿主 TSF 焦点、快捷键钩子、窗口层级、亚克力渲染或所有 ARM64/x86 环境；这些仍需对应专项及手动验收。
