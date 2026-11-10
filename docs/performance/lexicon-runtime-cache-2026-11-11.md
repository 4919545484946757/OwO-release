# 词库运行时格式高优先优化

## 已实现

导入词库保留 `catalog.json` 和原始词条、注音边界及来源信息，用于编辑、补码和导出。新增只读的 `runtime-v1/phonetic.owolx`（v3）和独立快捷短语 `aliases.tsv`。

设置中心保存时，先调用随设置中心发布的 `owo_config_shell --dictionary-compile <snapshot-directory>`。原生后端使用与原来相同的拼音推导、去重规则，预生成整数音节表、词条表、首字母索引及混拼索引。编译成功后，才原子替换 `current` 指针。编译失败、超时不发布新目录、启用状态或候选；旧快照不删除。

Core 通过稳定的 Lexicon 接口引用只读映射。不再在按键线程读取导入 TSV、推导注音或排序建索引。只有一个快照准备任务可以运行，不是每次按键创建线程。准备结果在 Core 线程一次性替换；结果所属快照与当前指针不同则丢弃，避免旧任务覆盖新配置。普通刷新仅非阻塞检查任务就绪；显式 `refresh(true)` 保留为离线测试屏障。

旧的、只有 TSV 的快照自动在后台迁移，完成后原子发布缓存目录；后续启动直接读取缓存。迁移线程在 Windows 降低 CPU 优先级，但不降低映射页的内存/IO 优先级。迁移期间保留上一份有效快照；首次启动尚无有效导入快照时，导入候选要等后台准备完成，在下一次请求生效。这是有意的非阻塞行为，不等于“导入词库永远立即可用”。

保留导入索引的原有同频排序，以及混拼最多 16384 个去重访问词条的查询预算。未改变学习记录格式、分区算法、评分系数或模型 ABI。快捷短语仍用有界 TSV 在后台载入，不宣称所有文本分配都已消除。其他输入方案的原始数据仍保留，拼音编译仅处理启用的 `full_pinyin` 数据。

## 基础词库 v3 接入

现有基础词库本来已经支持 mmap，未重复改造。新增 `scripts/prepare-runtime-lexicon.ps1`，用当前 writer 从 v2 离线重新生成 v3，重排同读音词条并重建所有词条位置索引，保留 v2 原文件。输出另存为 `rime-ice-cn-2026.06.30-v3.owolx`，不会仅改版本字段冒充 v3。

开发启动默认优先选用已有 v3；缺少 v3 时仍兼容 v2。发行打包先准备并校验 v3，再操作发行暂存目录。安装脚本接受 v2/v3，验证旁文件写入实际版本，不再硬编码为 2。没有重新打包或替换既有 0.3.0 安装包。

空导入词库现可生成并载入合法空二进制文件；无音节但包含非空词条/索引的非法结构仍会拒绝。链接/重解析点、大小超限、坏头、校验失败和非法快捷短语不会作为有效候选发布。

## 诊断与验证

Core 在准备结果应用时记录数值型 `dictionary_runtime_ready` 事件，包含 `prepare_us`、`migrated`、`phonetic_entries`、`alias_entries`、`success`，不记录用户文本或词条。它是快照准备时间，不是每个按键耗时；失败时的 0 不表示有效准备过程耗时为零。

本次执行定向验证，未运行全量测试：

- 三项原生合同（导入词库、二进制词库、Core 导入词库）各重复三次通过；覆盖完整/不完整/简拼/混拼、纠错、显式注音边界、两内置源停用、跨词/跨源组合、分页与展开、旧快照迁移、缓存重载、过期任务和坏缓存拒绝。
- 另外四项候选快照、配置热加载、计时和符号兼容 IPC 合同通过。
- 设置中心词库管理 56 项检查通过，包含编译失败不发布和编译发生在指针切换之前；另有实际原生后端集成检查通过。
- 编码补全 46 项通过；100001 条及大于 24 MiB 活跃数据的容量合同通过。
- 设置中心 Release 编译发布成功，0 警告、0 错误。
- 基础词库共 1885739 条，抽样 461 组读音比较 v2/v3，词条文本、权重、最大词频和最长读音长度保持一致。

查最大词频微基准（461 组 × 100 次，本机单轮热缓存）：v2 为 18752 µs，v3 为 18764 µs，基本持平。样本大多为小同音桶，不能据此宣称整体候选生成加速；主要确定收益是导入词库解析/建索引移出按键线程，以及运行时不展开全部拼音词条对象。

## 复现

```powershell
cmake --build --preset windows-release --target owo_lexicon_merge owo_binary_lexicon_tests owo_managed_dictionary_tests owo_managed_dictionary_contract owo_config_shell
.\scripts\prepare-runtime-lexicon.ps1
.\build\windows-release\Release\owo_binary_lexicon_tests.exe `
  D:/0program/OwO/build/windows-release/rime-ice-cn-2026.06.30-v2.owolx `
  D:/0program/OwO/build/windows-release/rime-ice-cn-2026.06.30-v3.owolx
ctest --test-dir build/windows-release -C Release -R '^owo\.(engine.managed_dictionary|engine.binary_lexicon|ipc.managed_dictionary_contract)$' --output-on-failure
.\build\dependencies\dotnet-10.0.403\dotnet.exe run --project tests/settings_center/dictionary_manager_tests.csproj -c Release -- --native-compiler D:/0program/OwO/build/windows-release/Release/owo_config_shell.exe
```

## 仍有边界

编译会增加词库保存时间和磁盘占用，原始管理文件不会移除；编译峰值内存尚未做流式化。只改显示名称或提示偏好也会生成新快照，复用相同活跃内容的编译产物可作为下一阶段优化。首次旧格式迁移仍需一次后台编译；不支持写入旧快照目录时会返回诊断，而不是静默丢失词条。该缓存仅适配已有 Windows mmap 读端，不能宣称已完成 Linux/macOS 格式落地。
