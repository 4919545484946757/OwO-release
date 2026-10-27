# OwO Agent SDK 单页产品落地页 —— 产品简报（Product Brief）

> 面向角色：designer（视觉/交互）、后续 frontend 实现者（index.html / styles.css / node:test）
> 证据来源：当前工作区为空（`list_dir .` 返回 0 条目），本简报基于用户目标与通用无障碍/响应式最佳实践推导；所有推断项已标注「待确认」。
> 本文件不包含 HTML/CSS 代码，仅提供可直接指导设计与实现的产品决策。

---

## 1. 目标用户（Primary / Secondary Audience）

| 优先级 | 受众 | 特征 | 他们最想知道的问题 |
|---|---|---|---|
| P0 | 独立开发者 / 全栈工程师 | 想把 LLM Agent 能力接入自己的产品，怕上手成本高 | 「多久能跑通第一个 Agent？」「要装什么、写几行代码？」 |
| P0 | 技术负责人 / 架构师 | 关注可维护性、可观测性、供应商可替换性 | 「它支持哪些模型/工具？能否自托管？」 |
| P1 | 开源贡献者 / 社区成员 | 需要文档、协议、贡献方式 | 「怎么参与？文档在哪？」 |
| P2 | 非技术决策者（产品/业务） | 不读代码，只看能力与案例 | 「能解决什么业务问题？」 |

**反受众（明确不服务）**：只想看企业采购报价单者（无 pricing 表）、需要完整 API 参考手册者（落地页不做全量文档，仅外链）。

---

## 2. 核心信息（Value Proposition & Key Messages）

**一句话定位（Hero H1 方向）**
> Build agents, not infrastructure. —— 用几行代码把可靠、可观测、可替换的 AI Agent 接进你的产品。

**三条支撑信息（按优先级，出现在 Hero 副标题与 Features 区）**
1. **少写胶水代码**：统一的 Agent 循环、工具调用、流式输出，开箱即用。
2. **可观测、可测试**：内置 trace / 事件日志，测试可重复（与 `node:test` 的验证故事一致）。
3. **模型与工具无关**：Provider 可插拔，工具/函数注册简单，不锁定单一供应商。

**主 CTA**：`Get started`（滚动到 Quick start 区或外链文档）。
**次级 CTA**：`View on GitHub` / `Read the docs`（次要视觉权重）。

**语调**：简洁、具体、少形容词；每句卖点必须能被一个事实（代码片段、数字、特性条目）支撑，避免「颠覆」「革命性」等空洞词。

---

## 3. 页面层级（Section Outline，自上而下单页）

必须包含以下 **8 个区块**，顺序固定（便于后续测试断言语义结构）：

1. **Header / 导航栏（sticky）**
   - 左侧：品牌标识（文字 logo「OwO Agent SDK」+ 可选 SVG 图标，图标须 `aria-hidden="true"`）。
   - 中部/右侧：锚点导航 —— Features / Quick start / FAQ。
   - 右侧 CTA：`Get started` 按钮。
   - 移动端（< 768px）：折叠为汉堡按钮 + 可展开菜单；按钮需 `aria-expanded` / `aria-controls`，并支持 Esc 关闭与焦点回归。
2. **Hero**
   - H1（唯一 `<h1>`）、一段副标题（1–2 句，≤ 140 字符）、主 CTA + 次 CTA。
   - 一个「代码/终端」视觉预览（装饰性截图或静态代码块，用 `<figure>` + `<figcaption>` 说明；代码块不使用 `tabindex` 之外的可交互性）。
3. **Feature Highlights（特征网格）**
   - 3–6 个特性卡片，每卡：图标（`aria-hidden`）+ H3 标题 + 1–2 句说明。
   - 推荐 6 卡：Easy setup / Tool calling / Streaming / Observability / Model-agnostic / TypeScript-first。
4. **How it works（三步流程）**
   - 有序列表 `<ol>`：Install → Define a tool/agent → Run & observe。
   - 三步必须语义化为有序列表，屏幕阅读器可读出顺序。
5. **Quick start（代码示例区）**
   - 一段可复制的安装命令（`npm i owo-agent-sdk`）+ 一段最小可运行示例。
   - 代码块需在窄屏可横向滚动而不破坏页面（`overflow-x: auto`，容器不得溢出视口）。
6. **Social proof / 采用证据（可选但推荐）**
   - 3 条用户引言或 3 个使用场景（卡形式），必须是真实/标注为示例的占位文本，不得伪造具体公司背书。
7. **FAQ（可折叠）**
   - 4–6 条问答；优先使用原生 `<details>/<summary>`（零 JS、天然无障碍），或自实现时配 `aria-expanded` + `aria-controls` + `button` 语义。
   - 内容：是否免费/开源自托管？支持哪些模型？需要 Node 版本？如何做测试？
8. **Footer**
   - 版权行、文档 / GitHub / 社区链接、`Back to top` 锚点链接。
   - 导航地标：`<footer>` 内含 `<nav aria-label="Footer">`，不得出现多个无标签 nav 冲突。

**地标要求**：全页恰好一个 `<header>`、一个 `<main>`、一个 `<footer>`；所有 `<nav>` 必须有唯一 `aria-label`。

---

## 4. 关键交互（Interaction Spec）

| # | 交互 | 触发 | 行为与无障碍要求 |
|---|---|---|---|
| I1 | 锚点平滑滚动 | 点击 nav 链接 | `scroll-behavior: smooth`（并尊重 `prefers-reduced-motion: reduce` 回退为瞬时跳转）；目标区块需可获焦或加 `scroll-margin-top` 避开 sticky header |
| I2 | 移动端菜单开合 | 汉堡按钮 | `aria-expanded` 同步；打开后焦点移入首个菜单项；Esc 关闭并把焦点还给按钮 |
| I3 | FAQ 折叠 | 点击 summary/按钮 | 使用 `<details>` 时为原生行为；自实现则同步 `aria-expanded` 并保留键盘 Enter/Space 支持 |
| I4 | 复制代码（可选增强） | 「Copy」按钮 | 提供 `aria-live="polite"` 状态提示「Copied」；无 JS 时按钮隐藏或用 `hidden` 属性，保证基础可用 |
| I5 | 可见焦点 | Tab 键导航 | 全局 `:focus-visible` 明显轮廓，禁止 `outline: none` 无替代；跳过链接（skip link）作为首个可聚焦元素指向 `#main` |
| I6 | 悬停微交互 | hover/focus | 卡片/CTA 的位移/阴影仅用 transform/opacity；在 `prefers-reduced-motion` 下禁用 |

**无 JS 降级**：页面在禁用 JS 时仍完整可读；导航链接为原生锚点，FAQ 用 `<details>`，复制按钮可缺失。

---

## 5. 视觉方向（Visual Direction，供 designer 落地）

- **气质**：现代开发者工具感 —— 深色优先（深墨蓝 / 近黑底 + 高对比文本），可同时提供浅色（`prefers-color-scheme`）方案；干净留白、克制的霓虹强调色（如电光青 #22D3EE 或紫蓝 #6366F1）。
- **字体**：正文用系统 UI 无衬线栈（含中文回退）；代码用等宽栈（ui-monospace, SFMono-Regular, Menlo, Consolas）。字号不得使用小于 16px 的正文字号。
- **排版层级**：H1 约 clamp(2rem, 5vw, 3.5rem)；正文 line-height ≥ 1.5；行宽控制在 60–75ch。
- **栅格与间距**：使用 4/8 的倍数间距刻度；容器最大宽约 1120px，左右安全边距 16–24px。
- **卡片**：轻边框 + 柔和背景，弱阴影；强调色只用于 CTA 与关键数字，避免大面积高饱和。
- **响应式断点**：≥ 1024px 三列特性网格 / 768–1023px 两列 / < 768px 单列；Hero 由左右双栏转为上下堆叠；导航折叠为汉堡菜单。
- **图像**：优先 SVG/CSS 绘制，避免大图；任何信息性图片必须有 `alt`，装饰性图 `alt=""` + `aria-hidden`。
- **禁止**：自动播放视频、闪烁动画、仅靠颜色传达状态（需配图标或文字）。

### 无障碍与性能硬指标（设计即可验证）
- 文本对比度 ≥ 4.5:1（大字号 ≥ 3:1），CTA 与背景同样适用。
- 全部功能可用键盘完成；焦点顺序与视觉顺序一致。
- 语义化 HTML：单一 `<h1>`，标题层级不跳级（h1→h2→h3）。
- 支持 200% 缩放与 320px 宽度下无横向滚动（代码块容器内部滚动除外）。
- 触控目标 ≥ 44×44px。
- 无阻塞主内容渲染的第三方脚本；首屏无布局抖动（为图片/代码块预留尺寸）。

---

## 6. 验收标准（Acceptance Criteria，可被 node:test 校验）

实现方需提供 `index.html`、`styles.css` 与基于 `node:test` 的测试（推荐仅用 `node:fs` 读取文件做静态断言，不引入重型 DOM 依赖；若引入 DOM 解析器需在 `package.json` 声明）。

**A. 文件与结构**
1. 仓库根存在 `index.html`、`styles.css`、测试文件（如 `test/page.test.mjs`），`package.json` 有 `test` 脚本执行 `node --test`。
2. `index.html` 引用了 `styles.css`（通过 `<link rel="stylesheet">`）。
3. 文档以 `<!DOCTYPE html>` 开头、含 `<html lang="en">`（或明确语言学值）、`<meta charset="utf-8">`、`<title>` 非空。

**B. 语义与无障碍（静态断言可覆盖）**
4. 恰好一个 `<h1>`，且包含产品名或核心定位关键词（如 "OwO" 或 "Agent"）。
5. 存在 `<header>`、`<main id="main">`、`<footer>` 各一个。
6. 每个 `<nav>` 都有非空 `aria-label`。
7. 每个 `<img>` 都有 `alt` 属性（允许空值但属性必须存在）。
8. 存在跳转链接（`href="#main"` 且文案含 skip 或 jump）。
9. 至少一个 `<section>`，且每个 `<section>` 有可读标题（`aria-labelledby` 或直接含 h2）。
10. 必须包含规定区块标识（建议用稳定的 `id`，便于测试与锚点）：`#features`、`#quickstart`、`#faq`。
11. CTA 存在且为可交互元素（`<a>` 或 `<button>`），文案包含 "Get started"（大小写不敏感）。
12. FAQ 使用 `<details>`/`<summary>`（或带 `aria-expanded` 的按钮）。
13. `styles.css` 中存在 `:focus-visible` 规则，且不存在 `outline: none`（无 `outline-style: none` 裸用）。
14. `styles.css` 包含 `@media` 响应式规则与 `prefers-reduced-motion` 处理。

**C. 内容质量**
15. Hero 副标题 ≤ 200 字符且不含占位符文本（"Lorem ipsum"、"TODO"）。
16. Feature 卡数量 ≥ 3。
17. Quick start 含可复制的安装命令（含 `npm` 或同类命令）。

**D. 手工/浏览器验收（记录在测试输出或 README 说明中）**
18. 320px / 768px / 1280px 三档视口无水平滚动条。
19. 仅键盘可完成：跳过链接 → 导航 → CTA → FAQ 展开/收起。
20. Lighthouse 无障碍分数 ≥ 95（若无法运行则注明未验证，不得声称通过）。

**测试实现约束**
- 测试只能在读取文件后做断言，**不得修改** `index.html` / `styles.css`。
- 每个断言都应有清晰失败消息，指明缺失的具体元素或规则。
- `npm test`（即 `node --test`）必须全绿；任何手工验收项若未执行，需在测试输出或 README 中明确标注「未验证」。

---

## 7. 非目标（Non-goals）

- 不构建多页面站点、博客、文档站或 API 参考手册（仅外链）。
- 不实现真实后端、鉴权、订阅支付或联系表单提交。
- 不引入前端框架或构建工具链（无 React/Vite/Tailwind 编译步骤）；保持原生 HTML + CSS（+ 极少量无依赖 JS，可选）。
- 不做国际化多语言切换（仅保证文本易于替换）。
- 不做暗色/亮色主题切换控件（跟随 `prefers-color-scheme` 即可）。
- 不追求视觉动效炫技；不做视频、3D、粒子背景等重资源效果。
- 不接入分析/追踪脚本与 Cookie 弹窗。
- 不做 SEO 深度优化（保留基础 title/description 即可）。

---

## 8. 给 designer 的交付清单（Handoff）

1. 页面线框（8 区块顺序 + 断点表现），标注 sticky header 高度与 `scroll-margin-top` 数值建议。
2. 色板（深/浅两套，含对比度实测值）、字体栈与字号阶梯（clamp 值）。
3. 组件规格：按钮（主要/次要/hover/focus/disabled）、卡片、代码块、FAQ 项、汉堡菜单开合状态。
4. 焦点态与 `prefers-reduced-motion` 下的静默样式。
5. 每个区块的最终文案（中/英一致，英文优先）与 `id` 命名对照表，确保与验收标准第 10、11 条一致。
6. 未决问题清单（如品牌 logo 是否已有、示例代码是否需要真实可运行、社交证据素材来源）需显式列给产品/负责人确认，不得默认编造。

---

## 9. 待确认 / 风险（Open Questions）

- 品牌官方名称与写法（"OwO Agent SDK" 是否包含版本号/公司名）。
- 是否已有官方 GitHub 与文档 URL（当前为占位 `#`，测试不应断言外链可达）。
- 社交证据区块若无可信素材，建议整体移除以避免伪造背书（downstream 需决策）。
- 工作区当前为空，无既有代码与风格约束；若后续出现既有设计系统，以既有约束优先。
