# OwO 输入法

OwO 是面向 Windows 11 x64 的模块化中文输入法。全拼是默认输入方案；解析、候选服务和设置层允许切换或扩展其他编码方案。基础输入、词典查询和上屏均可离线运行；智能排序不可用时保留基础候选。明确选择的插件输入方案不可用时保留编码，不自动切回拼音。

当前版本：`0.2.3`

## 功能概览

- Windows TSF 输入链路，Direct2D、DirectWrite 候选窗口
- 可选输入方案：全拼、精确编码；已授权插件输入方案（实验性，见[接入说明](docs/plugins/input-schemes.md)）
- 全拼支持完整/不完整拼音、首字母缩写、混合缩写和可切换纠错
- 句子、字典词和单字候选分区；分页、展开、点击与滚轮操作
- 无模式混合输入：拼音与英文、数字、符号混排仍给出候选（`ni3hao3` 声调数字自动折叠且候选序号优先、`baidu.com` 保留后缀、`API` 按英文处理且大写输入的字面候选领先）；数字在紧跟字母且不占用候选序号时并入预编辑（`1-5` 为声调、其余数字为混合输入），不再漏进文档
- 智能候选：算式结果（`1+2*3` → `7`，表格习惯 `=1+2` → `=1+2=3`）、数字中文读法（`1234` → 一千二百三十四 / 壹仟贰佰叁拾肆）、中文数字转阿拉伯数字（`壹仟贰佰叁拾肆元整` → `1234`）、时长读法（`90min` → `1小时30分`）、日期本地化与星期（`2026-10-19` → `2026年10月19日 星期一`，`20261019` 紧凑日期同样展开）、日期区间本地化（`2026-10-19~2026-10-25` → `2026年10月19日 至 2026年10月25日`）、时间补零、符号归一化（`x!=y` → `x≠y`）、日期/标点/括号纠错、日期区间顺序、中英文自动空格、中文标点全角化、全角 ASCII 归一化、拆分/合并/重排/缺口占位候选
- 概念候选：用描述读音找术语（`chongfuzhixing` → 幂等；输入到 8 个字母起即可按描述前缀边打边提示），术语表本地可替换（`--concept-glossary <TSV>`、环境变量 `OWO_CONCEPT_GLOSSARY`，或放到 `%LOCALAPPDATA%\OwO\InputMethod\data\concept-glossary.tsv`；示例见 `docs/architecture/concept-glossary.example.tsv`）
- 表情与颜文字候选：25 组意图 / 53 条本地素材（如 `leisi` → 😩、`ganbei` → 🍻、`moyu` → 🐟），整段预编辑匹配，并可在可解码时给出文字+表情组合（`累死😩`），不上传任何内容
- 三类附加候选可在配置中单独关闭（schema v16）：`smart_candidates_enabled`、`expressive_candidates_enabled`、`concept_candidates_enabled`，默认开启；无模式统一编码（混合输入/声调数字）不受影响
- 本地词频学习与 libime 中文上下文排序
- 预编辑编码光标、候选暂存、中文标点及可配置的多组快捷键
- WinUI 3 设置中心，按输入方案、候选、学习、快捷键和插件分区，支持全局搜索
- WinUI 3 三步安装向导，将软件介绍、安装选项和安装进度分别呈现
- `.owopkg`、ZIP、文件夹插件安装；高权限插件逐版本告知并授权
- 官方候选主题、Unicode 符号列表和 Agent IPC 连接器

## 安装与使用

运行 `OwO-Input-Method-0.2.3-windows-x64-Setup.exe` 后会进入 WinUI 3 安装向导：先查看软件介绍，再选择安装位置、安装选项和快捷方式，最后在独立页面查看安装进度与可展开的详细信息。也可解压便携包后双击 `Install-OwO.cmd` 使用脚本入口。系统需要 Windows 11 x64，以及“中文（简体，中国）”语言和基本输入支持；不要求把中文设为 Windows 显示语言。

安装完成后按 `Win + Space` 选择 OwO Input Method。常用入口：

- 设置中心：开始菜单中的“OwO 输入法设置”，或 `Open-OwO-Settings.cmd`
- 重启服务：`Start-OwO.cmd`
- 卸载：开始菜单卸载项，或 `Uninstall-OwO.cmd`

用户配置、词频和日志位于 `%LOCALAPPDATA%\OwO\InputMethod`，卸载默认保留这些数据。

## 默认按键

| 操作 | 默认按键 |
| --- | --- |
| 选择首候选 | `Space` |
| 选择编号候选 | 数字键 |
| 上一页 / 下一页 | `[` / `]`、`↑` / `↓` |
| 移动预编辑光标 | `←` / `→` |
| 移动应用文本光标 | `Shift+方向键`，默认不扩展选区 |
| 切换中英文 | `Ctrl+Space` |
| 切换全拼纠错 | `Ctrl+Q` |
| 直接输出当前字母 | `Enter` |

快捷键可在设置中心启用、关闭、改键或添加多组绑定。

## Agent 模式

安装并明确授权官方 Agent IPC 连接器后，以小写 `v` 作为拼音串首字符进入 Agent 模式，例如 `vbangwozhaowenjian`。`v` 仅是模式标记；后续内容仍可生成并选择拼音候选，但不会作为普通文本立即上屏。请求完整消耗或用户确认后，连接器会将经过限长和脱敏的数据发送给用户配置的本机第三方 Agent。

Agent 返回的自然语言只用于显示，控制行为必须使用结构化命令 ID。协议支持把意图拆成带原拼音区间的槽位；候选选择可只锁定或修改一个槽位，不重新发送整句，也不直接上屏。删除文件、执行程序、账户、支付、凭据和系统设置等高风险操作不能由候选框直接授权。详见：

- [权限与隐私说明](docs/plugins/agent-ipc-privacy.md)
- [第三方 Agent 接入规范](docs/plugins/agent-ipc-integration.md)
- [模拟 Agent 联调指南](docs/plugins/agent-ipc-mock.md)

## 插件开发

插件 API v1 支持隔离的进程插件和不执行代码的声明式资源插件。插件必须声明权限，不能加载进 TSF 或 Core Service。入门与接口清单见 [插件开发指南](docs/plugins/development.md)，安全边界见 [插件安全基线](docs/security/plugin-baseline.md)。官方插件源码集中在 [`plugins`](plugins)。

## 无模式输入与智能候选

引擎把预编辑切分为拼音、英文、数字与符号片段：全部为小写拼音字母时仍由拼音引擎（含纠错与缩写）独占；出现声调数字或混合内容时，候选栏同时给出主拼音段的解码结果与「照原样保留」候选，无需切换输入模式。算式结果、日期本地化/纠错与区间顺序、数字读法、时长、符号归一化、中英文自动空格、拆分/合并/重排/缺口占位属于智能候选；概念候选用一句话的拼音描述查找术语（本地术语表，可用 TSV 替换）；表情与颜文字来自编译进引擎的本地表，只匹配整段预编辑，不联网、不读取上下文。设计与边界见 [无模式统一编码、智能候选与多模态输出](docs/architecture/modeless-unified-input.md)。

## 构建与打包

需要 Visual Studio 2022 Build Tools、MSVC x64、CMake 3.25+、.NET 10 SDK 和 Windows 11 SDK。

```powershell
cmake --preset windows-debug
cmake --build --preset windows-debug
ctest --preset windows-debug

cmake --preset windows-release
cmake --build --preset windows-release
.\scripts\build_settings_center.ps1 -Configuration Release
.\open-owo-installer.cmd
.\scripts\package_release.ps1 -Version 0.2.3
.\scripts\package_setup.ps1 -Version 0.2.3
.\scripts\package_plugins.ps1
.\scripts\package-agent-sdk.ps1 -Version 1.3.0
```

`open-owo-installer.cmd` 会重新发布设置中心、在 `build\installer-preview` 生成临时完整安装暂存目录，并打开 WinUI 3 安装界面；它不会自动执行安装。

安装向导的三个主页面可分别运行 `open-owo-installer-start.cmd`、`open-owo-installer-options.cmd` 和 `open-owo-installer-progress.cmd` 预览。检查中、选项就绪、安装中、成功和失败等状态仍可通过原有的 `open-owo-installer-checking.cmd`、`open-owo-installer-ready.cmd`、`open-owo-installer-installing.cmd`、`open-owo-installer-success.cmd` 和 `open-owo-installer-error.cmd` 查看。阶段预览会禁用所有真实安装操作。

## 许可与完整性

- OwO 源码：`GPL-3.0-only`
- libime / Fcitx5Utils：`LGPL-2.1-or-later`
- 第三方许可：发行包 `LICENSES` 目录

当前构建未进行商业代码签名。请只从可信来源获取，并核对发行目录中的 `SHA256SUMS.txt`。
