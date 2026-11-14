# 输入方案插件：设置、词库与快捷键

设置中心顶部右侧的“选择要配置的输入方案”只控制设置页面，和 TSF 当前使用的方案、启用列表及轮换顺序相互独立。候选与输入、学习与排序、快捷键、词库管理器按配置目标切换，页标题旁显示“配置方案”。通用设置、插件管理和全局“切换输入方案”快捷键仍由 OwO 管理。

## 插件包约定 v1

安装并明确授权 `input.scheme` 后，宿主只针对实际发现的 `<plugin_id>/<local_scheme_id>` 读取同一活动版本内的 `input-schemes/<local_scheme_id>/settings.json`。可附带同目录的 `shortcuts.conf` 和 `typing.conf`。它们是包内资产，随包内容校验；不执行插件 XAML、脚本或任意命令。

```json
{
  "version": 1,
  "scheme": "table",
  "candidateFields": ["show_prefix_candidates"],
  "learningFields": [],
  "lexiconExtension": ".tsv",
  "sources": []
}
```

- 两个字段列表引用插件 `config_schema` 的已声明字段，不能跨页重复。用户值写入插件的 `data/<plugin_id>/settings.json`，只合并该页的字段；插件自行读取并执行语义。多个方案需要独立配置时，应声明不同的键。
- `lexiconExtension` 为 `.tsv`、`.json` 等小写扩展名，或 `null`。宿主只做容量与路径限制、显式导入/导出和原子写入；文件内部格式、编码、校验和候选逻辑由插件实现。用户文件位于 `data/<plugin_id>/input-schemes/<local_scheme_id>/user-lexicon<扩展名>`，每个文件最多 16 MiB，不进入宿主拼音词库。
- 可选 `learnedLexiconFile` 必须是 `learned<扩展名>`，用于为该方案自己的学习记录提供单独导出/清除入口；学习算法和写入仍由插件负责。可选 `lexiconHelp` 是最多 512 字符的纯文本格式说明。
- `sources` 每项包含 `id`、`title`、`license`、`url`、`bytes`、`sha256`。v1 只接受 `https://raw.githubusercontent.com/<owner>/<repo>/<40 位提交哈希>/<文件路径>`，拒绝重定向，不使用浮动分支、账户凭据或本地网络地址。由用户明确点击后下载，大小和 SHA-256 均匹配才写入 `data/<plugin_id>/input-schemes/<local_scheme_id>/dictionaries/<来源 id><扩展名>`。声明来源不授予插件联网权限。
- 本协议不自动转换词库、执行导入脚本、编辑插件私有学习数据库或允许插件访问其他方案目录。旧包无声明时不显示拼音设置，仍可在插件管理器访问其已有通用配置。

### 方案快捷键

`shortcuts.conf` 为 ASCII，首行 `OWO_SCHEME_SHORTCUTS_V1`，以下为 `动作=键1;键2`，空值关闭。允许的组合动作：`raw`、`cursor_left`、`cursor_right`、`previous_page`、`next_page`、`expanded_previous_page`、`expanded_next_page`。未声明的动作不继承全拼绑定；旧包没有此文件时走原兼容行为。

用户覆盖在对应方案目录的 `shortcuts.conf`。宿主校验标准键名、重复绑定、全局切换冲突，并保留复制、粘贴、撤销等常用应用快捷键。只在该插件方案的合法组合态下生效，不注册全局热键。数字选词、取消与删除仍是组合协议的基本操作。

默认空格选择候选。附带仅一行 ASCII `OWO_SCHEME_WORD_TYPING_V1` 的 `typing.conf` 时，宿主进入拉丁单词输入策略：ASCII 字母开启组合，词内接受撇号和连字符，空格及普通可打印分隔符提交原始输入并保留分隔符。方案应声明相应字母和连字符编码；该策略不添加读写权限，也不允许包执行任意按键命令。

### 方案图标

可在活动包的 `input-schemes/<local_scheme_id>/icon.mask` 声明 16×16 单色图标。文件为 ASCII：首行 `OWO_SCHEME_ICON_MASK_V1`，其后恰好 16 行，每行恰好 16 个 `0`（透明）或 `1`（前景色），至少有一个前景像素。支持 LF 和 CRLF。参考[英文输入的图标](../../plugins/english/input-schemes/english/icon.mask)与[五笔的图标](../../plugins/wubi86/input-schemes/wubi86/icon.mask)。

已启用且明确授权的版本提供该图标，宿主按主题为其着色，用于输入方案设置列表、切换浮窗和 OwO 的 TSF 任务栏指示器图像。缺失、格式不合法或重解析路径一律使用宿主回退标识；无额外权限，不读取用户目录中的同名替换文件，也不加载图片解码器或插件代码。更新图标属于修改包内容，应提升插件版本。

### 共用候选显示设置

在 `input-schemes/<local_scheme_id>/presentation.conf` 放置以下 ASCII 声明可启用宿主的设置卡：

```ini
OWO_SCHEME_PRESENTATION_V1
preview=input
page_size=7
wrap_length=24
```

`preview` 可为 `input`（输入位置）或 `candidate`（候选栏）；`page_size` 为 1–9，`wrap_length` 为 4–64。用户覆盖写入同名的方案私有文件，不会改变全拼配置。旧包没有此文件时延续原有宿主布局；目标应用不支持 TSF 内联组合时回退到候选栏。纯净键盘不生成候选，因此不提供这些设置。插件仍拥有词库和候选生成逻辑，不应在候选响应中注入 UI 操作。

## 示例与边界

五笔 86 示例包声明了前缀候选开关、`.tsv` 用户词库及独立按键。其词库使用包内 `main.tsv` 同样的 `OWO-WUBI86` v1 格式。新提供者会话读取方案数据，忽略不合法词库；用户词库和下载文件只参与五笔候选，学习事务日志仍在五笔自己原有的数据文件中。修改资产后可构建 `owo_wubi86_package` 或 `owo_english_package` 重新暂存，即使插件二进制没有改动。

现有内置全拼保留原有词库、设置和格式，内置精确编码仍使用宿主兼容设置，纯净键盘不提供词库。它们不是可卸载的外部包；本协议为第三方包提供独立实现路径，不迁移或覆盖现有全拼数据。[英文输入插件](../../plugins/english/README.md) 使用独立词频与词库，并通过 `typing.conf` 声明单词输入。
