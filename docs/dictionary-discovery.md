# 在线发现词库

入口：设置中心 → 词库管理器 → 顶部“已安装 / 发现词库”标签，与插件管理器使用相同的 SelectorBar 切换。已安装词库继续提供编辑、开关、导出和删除；发现页提供来源选择、搜索、下载进度及取消，选取上限与来源说明收纳在可展开的选项栏。

本地和在线词库共用响应式卡片尺寸规则（理想宽度 210、高度 228），窄窗口自动减少列数并缩小卡片，不使用水平滚动。卡片统一采用图标、标题、摘要、底部操作栏、右上角菜单；在线卡片点击正文或菜单查看完整详情及许可，底部操作为下载预览。切换标签保留来源、搜索及已获取目录，不自动联网；未变化的在线卡片实例增量复用，避免刷新时整屏重建。

## 五个来源

- [雾凇拼音](https://github.com/iDvel/rime-ice)：获取 cn_dicts 内 Rime 文本，GPL-3.0-only；无注音的数据保留为待补编码，可能与基础词典重复。
- [维基词库](https://github.com/felixonmars/fcitx5-pinyin-zhwiki)：获取正式发行的 .dict.yaml；生成数据遵循 Wikimedia 许可，不能用生成器的 Unlicense 替代。
- [萌娘词库](https://github.com/outloudvi/mw2fcitx)：获取正式发行的 .dict.yaml；数据许可见上游，不能用生成器 MIT 替代。
- [THUOCL](https://github.com/thunlp/THUOCL)：获取 data 内文本，MIT；原始数据没有拼音，不猜测多音字。补编码后才会进入候选。
- [CC-CEDICT](https://www.mdbg.net/chinese/dictionary?page=cedict)：官方 UTF-8 gzip，CC BY-SA 4.0；取简体及拼音，去声调、u:/ü 转 v，不导入英文释义。

只处理数据，不运行生成器、Lua、Python 或安装脚本，不引入 Rime 引擎。使用带拼音的文本版本而非 .dict 二进制。

## 安全、容量与质量

目录仅在显式刷新时联网；搜索仅在本地目录进行，不上传输入、应用上下文、个人词库或配置。切换来源会清空旧目录。

下载先确认来源和许可，再转换、预览、确认导入，默认不启用。预览显示上游有效行数、本次实际选取数、待补编码数和来源摘要；选取按文件顺序，默认最多 10000 条，最高 100000，并非质量排名。未选取的数据不会自动追加。

上游文件下载和解压各受 128 MiB 限制，更大的条目显示但不可下载。原本地文件导入仍限制 32 MiB；独立词库合计仍限制 10 万条和 32 MiB 序列化数据。容量不足明确拒绝并保留既有词库，不能以拆分多个文件绕过。首版不提供大型词库的全量只读存储。

上游词频不可直接相互比较，转换为中性权重 1，不计为个人学习。不会覆盖用户编辑；同一内容和选取规模拒绝重复导入，较大范围可在删除旧导入后重新选取。已安装项目继续允许导出备份。

GitHub Contents 文件核对 Git blob SHA-1；发行文件核对 API SHA-256，同时计算本地 SHA-256。摘要不符拒绝并提示刷新。Git 对象 SHA-1 仅用于内容匹配，不是商业签名。CC-CEDICT 此入口没有独立上游校验和，只保障 HTTPS 与本地摘要，界面明确告知。

限定固定仓库、官方 MDBG 文件与 GitHub CDN 的 HTTPS 地址，每次重定向重新检查。元数据限时 40 秒，下载转换整体限时 15 分钟，可取消；匿名 API 可能限流，可重试。原始数据仅在设置中心的有界缓冲中转换，不持久保存整套上游文件，也不在 TSF/Core 输入线程执行。

## 定向测试

```powershell
dotnet run --project tests/settings_center/dictionary_sources_tests.csproj -c Release
dotnet run --project tests/settings_center/dictionary_sources_tests.csproj -c Release -- --live
dotnet run --project tests/settings_center/dictionary_manager_tests.csproj -c Release
```

--live 会读取五个真实来源并抽样下载转换，不安装。离线测试包含来源拒绝、格式转换、拼音规范化、选取计数、中性权重、摘要错误及取消。联网成功不代表全量词条已人工校对或完成许可证法律审查。
