# OwO 输入法 0.1.0

面向 Windows 11 x64 的正式版。包含 TSF 输入法、Core Service、完整拼音词库、
本地词频学习、libime 中文上下文排序、候选窗口和 WinUI 3 设置中心。

## 系统要求

- Windows 11 x64
- 已安装“中文（简体，中国）”语言和基本输入支持；不要求将其设为 Windows 显示语言

如果缺少该语言，请先前往“设置 > 时间和语言 > 语言和区域”下载并安装，完成后再运行 OwO 安装程序。

## 安装

1. 解压整个 ZIP，不能只打开压缩包内的文件。
2. 双击 `Install-OwO.cmd`。
3. 安装完成后按 `Win + Space`，选择 `OwO Input Method (P1 Prototype)`。

图形安装向导会提供设置中心快捷入口选项：开始菜单默认勾选，桌面和任务栏引导默认
不勾选。选择任务栏引导后，安装完成会打开设置中心；请右键单击其任务栏图标并选择
“固定到任务栏”。Windows 11 不允许安装程序静默完成这一步。

安装范围仅限当前 Windows 用户；首次注册和卸载 TSF 时会显示 Windows UAC 管理员授权提示。
运行文件安装到
`%LOCALAPPDATA%\Programs\OwO\InputMethod\0.1.0`，用户配置、词频和日志保存在
`%LOCALAPPDATA%\OwO\InputMethod`。

## 使用与卸载

- `Start-OwO.cmd`：重新启动后台运行环境。
- `Open-OwO-Settings.cmd`：直接打开 WinUI 3 设置中心。
- 开始菜单中的“OwO 输入法设置”：打开设置中心。
- `Uninstall-OwO.cmd`：注销输入法并移除自动启动项，默认保留用户数据。

这是未进行商业代码签名的正式版，Windows 可能显示来源警告。不要从不可信来源下载
重新封装的版本；可用 `SHA256SUMS.txt` 核对发行文件。
