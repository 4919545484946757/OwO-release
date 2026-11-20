# OwO-release 输入法全量整合（2026-10-27）

本次将独立仓库 `4919545484946757/OwO-release` 的 master（`0c30b077b7fe5d883ac109e6157690ebf61d9f0c`）通过真实 subtree merge 合入主仓库 master（基线 `c0c2e538edffeb6870d04c41d369e4d79e0db494`）的 `ime/`。共同基线为 `e1c05370c9906fc621f5399c44d359dba8394c2b`，包含独立仓库此后全部 48 个提交、165 个净变更文件，保留其历史 ancestry。

## 合并边界

保留 master 的连接器认证、智能/表情/术语候选、语料学习、搭配续写、风格档案和语气草稿/反馈。输入法保持独立 CMake 工程，不加入 Agent SDK Rust workspace。本次不合并 agent-frame 或其他 Agent SDK 分支，不包含其他本地工作树的未提交修改。

独立仓库的拼音搜索与纠错、候选分区/展开分页、组合学习隔离与衰减、用户词库编辑、模块化设置和导入导出、开发 CLI、图标与安装流程等均保留。冲突处理以两边功能并存为准，不以整目录覆盖代替合并。

## 原生配置 v21

两条历史分支独立占用了 v16/v17，不能仅按版本号解释字段：

| 来源 | 历史版本 | 识别和升级 |
| --- | --- | --- |
| 共同输入法 | v1–v15 | 继续原有逐版迁移 |
| 主仓库 | v16–v20 | 保留智能候选、语料、预测、风格和语气字段；补齐独立仓库新增选项 |
| 独立仓库 | v16/v17 | 由单字优先/展开快捷键字段识别；补齐主仓库字段，语料学习及语气云端模型默认关闭 |
| 整合后 | v21 | 完整包含两边全部字段；缺失、重复、混合或未知字段仍判定无效 |

读取旧格式不写回文件；显式保存时写 v21。Up/Down 从旧普通翻页组迁移至展开页选择，保留禁用状态与其他动作的自定义键位。设置中心、Core 和 TSF 必须配套更新，旧程序不能读取新 schema。沿用原生配置损坏时备份/默认值恢复机制；严格文件导入通过独立 JSON 验证，不能依赖该恢复机制接受错误配置。

`owo_config_shell set-all` 保留主仓库已有位置参数，末尾追加单字优先和展开页上下操作的 5 个参数；新的设置中心使用完整参数集合。独立仓库旧设置中心必须同时升级，不能和新 shell 混用。

## 设置与候选的交叉行为

- 设置模块覆盖 11 个模块、52 个界面字段；语料、风格、语气接入同一加载、保存、观察和按页重置流程。插件方案仍保留不适用字段，但禁用未接线能力。
- 配置导出格式升至 v3，包含语料/风格/语气设置。可读取独立仓库 v1/v2，新增采集/云端选项默认关闭；拒绝在旧格式中注入新增字段。导入确认显示语料学习和云端模型开关。
- 展开候选的数字和空格选择按选中页映射，同时遵守主仓库“续写候选不编号、不由数字或空格提交”的约束。只有续写候选的尾页不会误提交。
- 保留主仓库注册脚本的用户输入法列表保护，并加入独立仓库 TSF 缓存刷新。

## 验证入口

新增 `owo.config.schema_lineages` 回归测试，覆盖两条配置历史的非默认值迁移、只读加载、保存升级及无效混合格式。设置导入导出测试增加 v2 迁移、隐私开关默认值和主仓库全部新增设置的原生往返。展开候选 TSF 分派测试补充续写尾页保护。

```powershell
cmake -S ime -B <build> -G "Visual Studio 17 2022" -A x64 -DBUILD_TESTING=ON -DOWO_BUILD_DEVELOPER_TOOLS=ON
cmake --build <build> --config Release --parallel 1
ctest --test-dir <build> -C Release --output-on-failure -j 1
dotnet run --project ime/tests/settings_center/settings_modules_tests.csproj -c Release -- <build>/Release/owo_config_shell.exe
dotnet run --project ime/tests/settings_center/configuration_transfer_tests.csproj -c Release -- <build>/Release/owo_config_shell.exe
dotnet run --project ime/tests/settings_center/preferences_tests.csproj -c Release
dotnet run --project ime/tests/settings_center/user_lexicon_tests.csproj -c Release
```

构建和契约测试不等于已安装验收；真实应用中的 TSF 交互、主题外观及安装升级仍需对应环境验收。本次不替换日常运行中的输入法或用户配置。

## 本轮验证结果

2026-10-27，Windows x64 / MSVC 19.44 / Windows SDK 10.0.26100：

- 原生 Release 完整构建通过；最终 CTest 105/105 通过（33.54 秒）。
- 设置模块 4,552 项检查、配置导入导出 36 项检查、偏好/安装载荷、用户词库 JSON/TSV 四组 C# 测试通过。
- 设置中心与独立安装器编译均为 0 警告、0 错误。安装器使用 SDK 10.0.302 和锁定依赖，嵌入合成测试载荷，仅验证编译，不作为可安装发布包。
- Windows PowerShell 5.1 的安装前置条件、目录重试/保护、安装包完整性夹具通过；未执行安装或注册。
- 共享 IPC 23 个样例与 Agent workspace 边界检查通过；master 独有 75 个文件全部保留且内容未改动；本次变更均位于 `ime/`。
- 旧的 Core 契约中，等字数已学习词的分组/优先级断言按独立仓库现有联合分区规则更新；大写拉丁测试保留字面候选首位断言，同时允许扩充词典产生的合法拼音前缀。没有为通过测试修改候选生产排序。
- WinHTTP 回环、隔离桌面、命名管道与插件子进程测试需要正常本机权限；沙箱中失败/超时后，在本机串行执行最终完整回归通过。

未进行 x86 构建、实际应用的可见交互验收、正式发布打包或安装升级。源码整合与自动化回归的结果不能替代这些验收。
