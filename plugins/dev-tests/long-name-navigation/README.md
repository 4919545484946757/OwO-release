# 长名称导航测试插件

只用于设置中心的侧栏展示测试，安装后保持停用；无需启用即可打开设置页。
权限列表为空，不联网，不注册输入方案，也不提供候选框主题。

测试包入口复用仓库的 `owo_example_process_plugin.exe`，仅为满足有效进程插件包契约；
本测试不启动该入口、不执行插件服务。不要为了检查侧栏而启用插件。
设置页面由宿主读取声明式 schema 生成，不运行插件代码。

## 组装

创建独立的 `build/plugin-navigation-long-name` 暂存目录，复制本目录的
`manifest.json`、`config.schema.json` 和此说明，再将
`build/windows-release/Release/owo_example_process_plugin.exe` 复制为
暂存目录中的 `bin/owo-navigation-test.exe`。通过插件后端检查并按精确摘要安装，
保持停用。不能将空文件或虚假 EXE 作为入口绕过包校验。

## 检查

1. 展开“插件设置”，长名称应省略，侧栏宽度不应增加。
2. 悬浮这一项，完整名称在独立浮层中越过侧栏显示，点击可打开本测试设置页。
3. 移出、收起分组或调整窗口尺寸，浮层应关闭，导航栏不应反复伸缩。
4. 测试结束可在“插件”管理页删除 `org.owo.dev.long-name-navigation` 的 `1.0.0`。

此目录置于 `plugins/dev-tests/` 的第二级，不被官方插件一层目录的默认打包流程收集。
