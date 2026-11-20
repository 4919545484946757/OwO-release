# 展开候选列表的页选择

## 操作

- 点击“展开”：高亮折叠模式下当前候选页，并把它滚动到可见范围。
- 默认 `↓` / `↑`：自动展开，选中下一页 / 上一页；第一页向上不越界。第一次按下也执行一次页移动。
- 数字键 `1`–`每页候选数`：选择高亮页内相应候选；每行编号从 1 开始，非高亮行编号弱化显示。空格选择高亮页的第一个候选，尾页没有对应编号时不选词。
- 点击某页的空白或候选之间的间隔：只选中该页，不上屏。点击具体候选仍直接选词。
- 滚轮滚动视口；如果原高亮页离开可见范围，选中最近的可见页，数字键始终对应可见的高亮页。
- 收起后回到高亮页，不强制跳回第一页。继续输入、删除、取消或切换输入位置后按原组合态规则重置，不保留旧页身份。

`[` / `]` 保留普通翻页功能；已展开时切换高亮页。符号列表打开时，上/下对应其分页，不会意外打开拼音候选列表。

## 配置与兼容

设置中心 → 快捷键新增“展开并选中上一候选页”和“展开并选中下一候选页”。分别默认 `Up` 和 `Down`，可独立关闭、改成组合键或添加多组绑定。不与 `Shift+方向键` 的文本区光标功能混用；未输入编码时不会拦截普通上/下方向键。

独立仓库原生配置 Schema v17 新增四项；主仓库整合后统一写 v21，兼容两条历史版本，见[整合记录](ime-release-integration-20261027.md)：

```text
expanded_previous_page_shortcut_enabled=true
expanded_previous_page_shortcut=Up
expanded_next_page_shortcut_enabled=true
expanded_next_page_shortcut=Down
```

独立仓库旧 Schema 1–16 及主仓库 v16–v20 继续读取：原翻页组里的纯 `Up` / `Down` 移到新操作，其他键位保留；原操作关闭时不擅自重新开启。若方向键已被其他已启用动作使用，则保留用户绑定，禁用冲突的新操作。迁移不写回原配置，直到用户保存；仍使用既有 `.bak` 备份机制。设置中心、Core 与 x64/x86 TSF 应配套更新，旧程序不认识 v21 配置时不要混用。

配置导出格式现为 v3，完整保存新快捷键及语料/风格/语气设置；可读取并迁移独立仓库 v1/v2 导出文件。旧设置中心会拒绝新格式，而不是丢掉字段。插件授权和词库数据不属于本次迁移。

## 状态与渲染

`ExpandedCandidatePages` 独立保存选中页和视口起始页。数字索引根据 `选中页 × 每页项数 + 行内编号` 计算，不能用视口顶部或全局编号代替。

选中页在当前视口内移动时只请求重绘，不创建候选窗口、不隐藏/重新显示窗口，也不向 Core 请求候选。首次展开需要异步获取完整列表：保留当前画面，累计页移动；数字选择绑定目标页与行内编号，结果返回后再校验，过期或缩短后的无效目标不能提交其他候选。

插件方案沿用隔离协议和候选 ID，上/下选择只作用于当前批次；跨批次通过原方案分页请求。每批请求按完整候选页对齐且不超过提供者上限，折叠与展开页码通过批内页号映射，不改变权限及安全上下文门禁。Agent 高风险指令仍由原安全检查拒绝直接执行。

## 定向验证

```powershell
cmake --build --preset windows-developer --target owo_expanded_candidate_pages_tests owo_expanded_candidate_service_tests owo_expanded_page_shortcuts_tests owo_config_store_tests owo_config_shell
ctest --test-dir build/windows-developer -C Release -R 'owo\.(tsf\.expanded_candidate|config\.(expanded_page_shortcuts|store))' --output-on-failure
dotnet run --project tests/settings_center/settings_modules_tests.csproj -c Release -- build/windows-developer/Release/owo_config_shell.exe
dotnet run --project tests/settings_center/configuration_transfer_tests.csproj -c Release -- build/windows-developer/Release/owo_config_shell.exe
```

状态机测试遍历每页 1–9 项、总数 0–256、首尾页、短尾页、视口跟随、整数边界和插件批大小。真实 TSF 分派测试只在测试可执行文件开放内部访问，不激活日常 TSF、不创建工作线程或连接日常 Core；覆盖异步展开期间连续按键、数字映射、关闭/重绑快捷键、选页点击、收起重开及过期响应拒绝。实际应用和主题下的高亮外观、IME 按键路由仍需手动验收。
