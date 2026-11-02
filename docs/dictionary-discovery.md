# 在线发现词库

入口：设置中心 → 词库管理器 → 顶部“已安装 / 发现词库”标签。发现词库分为两级：第一级以卡片选择上游来源，第二级展示该来源的具体词库。第一级不联网，点击来源后自动获取目录；第二级提供返回来源按钮、来源名称、搜索、原有词库卡片和下载进度，选取上限与来源说明仍收纳在可展开的选项栏。

本地、来源选择和在线词库共用响应式卡片尺寸规则（理想宽度 210、高度 228），窄窗口自动减少列数并缩小卡片，不使用水平滚动。具体词库卡片保留图标、标题、摘要、底部操作栏、右上角菜单；点击正文或菜单查看完整详情及许可，底部操作为下载预览。切换标签保留所在层级、来源、搜索及已获取目录；被取消或失败的目录在重新进入时重试，未变化的在线卡片实例增量复用。

## 五个来源

- [雾凇拼音](https://github.com/iDvel/rime-ice)：获取 cn_dicts 内 Rime 文本，GPL-3.0-only；无注音的数据保留为待补编码，可能与基础词典重复。
- [维基词库](https://github.com/felixonmars/fcitx5-pinyin-zhwiki)：获取正式发行的 .dict.yaml；生成数据遵循 Wikimedia 许可，不能用生成器的 Unlicense 替代。
- [萌娘词库](https://github.com/outloudvi/mw2fcitx)：获取正式发行的 .dict.yaml；数据许可见上游，不能用生成器 MIT 替代。
- [THUOCL](https://github.com/thunlp/THUOCL)：获取 data 内文本，MIT；原始数据没有拼音，不猜测多音字。补编码后才会进入候选。
- [CC-CEDICT](https://www.mdbg.net/chinese/dictionary?page=cedict)：官方 UTF-8 gzip，CC BY-SA 4.0；取简体及拼音，去声调、u:/ü 转 v，不导入英文释义。

只处理数据，不运行生成器、Lua、Python 或安装脚本，不引入 Rime 引擎。使用带拼音的文本版本而非 .dict 二进制。

## 安全、容量与质量

选择来源后自动获取目录；不显示原刷新和取消按钮。搜索仅在本地目录进行，不上传输入、应用上下文、个人词库或配置。返回来源页会取消未完成的目录请求，再选来源重新获取；旧请求不能覆盖新来源。切回“已安装”或卸载页面也会取消未完成的工作。下载和确认导入期间暂时禁用层级返回，避免交互跨页；目录加载期间始终可以返回。失败后可返回来源重新进入重试。

下载先确认来源和许可，再转换、预览、确认导入，默认不启用。预览显示上游有效行数、本次实际选取数、待补编码数和来源摘要；选取按文件顺序，默认最多 50000 条，最高 500000，并非质量排名。未选取的数据不会自动追加。

容量现为：在线下载及解压各 256 MiB，本地单文件和 Gboard 解压文件 128 MiB，独立词库合计 50 万条，JSON 目录与 Core 启用快照各 256 MiB。超过大小的条目仍显示但不可下载。容量不足明确拒绝并保留既有词库；设置中心和 Core 上限同步，不以多文件绕过总量限制。大目录校验、序列化和启用索引生成在设置中心后台执行。

运行时编码仍限 256 字符，原编码元数据独立限 4096 字符且禁止控制字符。
上游注音过长时，词条正文和原编码完整保留为“待补编码”，不会截断、丢弃或阻断整份导入；
待处理词条不写入启用快照，不绕过运行时限制。JSON/TSV 导入导出和再次打开均保留原编码。
旧版本仅接受 256 字符原编码，回退前应先导出备份并处理这些超长待补项。

本次是有界扩容，不是内存映射大词库重构。首次启用大型词库仍可能增加内存与加载时间，建议按需选取；旧版本无法读取超过其原有限制的数据，回退前应导出备份并减少词库规模。

上游词频不可直接相互比较，转换为中性权重 1，不计为个人学习。不会覆盖用户编辑；同一内容和选取规模拒绝重复导入，较大范围可在删除旧导入后重新选取。已安装项目继续允许导出备份。

GitHub Contents 文件核对 Git blob SHA-1；发行文件核对 API SHA-256，同时计算本地 SHA-256。摘要不符拒绝并提示刷新。Git 对象 SHA-1 仅用于内容匹配，不是商业签名。CC-CEDICT 此入口没有独立上游校验和，只保障 HTTPS 与本地摘要，界面明确告知。

限定固定仓库、官方 MDBG 文件与 GitHub CDN 的 HTTPS 地址，每次重定向重新检查。元数据限时 40 秒，下载转换整体限时 15 分钟，可取消；匿名 API 可能限流，可重试。原始数据仅在设置中心的有界缓冲中转换，不持久保存整套上游文件，也不在 TSF/Core 输入线程执行。

## 定向测试

```powershell
dotnet run --project tests/settings_center/dictionary_sources_tests.csproj -c Release
dotnet run --project tests/settings_center/dictionary_sources_tests.csproj -c Release -- --live

# 对已下载的萌娘 Rime 文件做完整离线转换；不安装，不写日常词库
dotnet run --project tests/settings_center/dictionary_sources_tests.csproj -c Release -- --moegirl-file D:\Tests\moegirl.dict.yaml
dotnet run --project tests/settings_center/dictionary_manager_tests.csproj -c Release
dotnet run --project tests/settings_center/dictionary_capacity_tests.csproj -c Release
```

--live 会读取五个真实来源并抽样下载转换，不安装。离线测试包含来源拒绝、格式转换、拼音规范化、选取计数、中性权重、摘要错误及取消。联网成功不代表全量词条已人工校对或完成许可证法律审查。

### 萌娘超长注音回归

已对上游 [20261024 发行数据](https://github.com/outloudvi/mw2fcitx/releases/tag/20261024)
的 `moegirl.dict.yaml` 完整离线验证，SHA-256 为
`f7ed0e76e6bab529c43ff1fb59252e003267c4a339d83972b5b112d025e1b314`。
原文件第 44677 行含 78 字正文、272 字符原注音；在线转换去除头部并添加 TSV 表头后，
旧版会在第 44673 行因原编码元数据超过 256 字符而中止，并非下载损坏。

修复后完整转换 130504 条、待补编码 1 条；默认选取 50000 条亦可正常预览，
本地 Rime 导入取得相同的完整条数与待补数。测试未安装或启用该词库。
合成测试另覆盖元数据上限、控制字符拒绝、正常条目继续导入、原文/编码不截断、
JSON/TSV 往返、快照保存与重新加载，以及待补项不会进入活动编码快照。
