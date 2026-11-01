# 五笔字根图解（wubi86-viewer）

86 版五笔学习站：输入汉字、词语或五笔编码，田字格按字根分色高亮，并自动讲解取码规则（键名字、成字字根、末笔字型识别码、简码、词组规则）。

线上地址：<https://wubi.sdcb.ai>

[![查询「胜」：田字格字根分色高亮与取码讲解](docs/screenshot-char.png)](https://wubi.sdcb.ai/?q=胜)

[![查询「今天吃什么」：推荐打法与优化打法](docs/screenshot-sentence.png)](https://wubi.sdcb.ai/?q=今天吃什么)

## 功能

- 汉字 / 词语 → 微软五笔的全部编码（含简码），候选顺序与系统一致
- 句子 / 未收录词组 → 推荐打法：按微软词库动态规划切分，给出免选重的推荐打法，以及允许数字选重、更省键的优化打法
- 编码 → 汉字 / 词语（支持前缀「继续输入」，第 3、4 位可用 Z 万能键）
- 田字格字根高亮：悬停编码字母即高亮对应笔画，支持拆解演示与笔顺演示
- 字根键盘表、取码规则速记
- 查询记录保存在 localStorage：易忘标记、删除、清空、导入导出、复习卡片

## 数据

| 文件 | 来源 |
| --- | --- |
| `data-src/ms-wubi.tsv` | 本机 `C:\Windows\InputMethod\CHS\ChsWubiNew.lex`（`npm run extract` 导出） |
| `data-src/data-wubi-v86.tsv` 等 | [sdcb/search-wubi](https://github.com/sdcb/search-wubi) 字根拆解、识别码 |
| `data-src/hanzi-writer-data/` | [Make Me a Hanzi](https://github.com/skishore/makemeahanzi) 笔画矢量（Arphic Public License） |
| `data-src/黑体字根.ttf` | 字根字形 |
| `scripts/root-names.json` | 非成字字根的中文名称（如“青字头”） |

候选顺序：词库每条记录带有权重，同一编码下按权重升序（与输入法候选顺序一致）。

### 字根笔画自动对齐

search-wubi 只为约 2,500 字提供了“笔画 → 字根”的对应关系。`scripts/lib/roots.mjs` 从这些样本中学习每个字根的笔画序列，
再对其余字做回溯搜索（允许包围结构拆成两段、允许 >4 根字中间不取码的笔画）；仍失败时依次尝试：人工分段中字根变形的情况（如「延」中的「止」）、
字根与书写顺序交错的任意顺序搜索（如「兆」「非」）、编辑距离近似匹配。

若 search-wubi 的笔画序列与 Make Me a Hanzi 笔画数不一致（多见于繁体字，如 4 画的「艹」），`scripts/lib/strokes.mjs`
会用 kNN 根据笔画中线推断每一笔的类型，并从这些字中自动学习繁体字根的笔画变体（如「艹」「糹」「釒」）。
近似结果会在页面上提示“自动推断”。

## 开发

```bash
npm install
npm run extract   # 仅在 Windows 上、词库更新时需要
npm run data      # 生成 public/data 分片 JSON
npm run dev
```

推送到 `main` 后由 GitHub Actions 生成数据、构建并发布到 Cloudflare Pages（需要 `CLOUDFLARE_ACCOUNT_ID`、`CLOUDFLARE_API_TOKEN`、`CLOUDFLARE_PAGES_PROJECT` 三个 secret）。

## 许可

本项目源代码以 [GNU AGPL-3.0](LICENSE) 许可开源。`data-src/` 中的第三方数据各自遵循其原始许可（见上表）。
