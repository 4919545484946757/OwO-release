import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../core/markdown.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../experience.css", import.meta.url), "utf8");
const sandbox = { window: { location: { href: "http://localhost/" } }, URL };
vm.createContext(sandbox);
vm.runInContext(source, sandbox);
const md = sandbox.window.OwoMarkdown;
const render = md.renderMarkdown;
const renderMessage = md.renderMessage;

test("图片只在协议安全时渲染，并带懒加载", () => {
  const html = render("![图一](https://example.com/a.png)");
  assert.match(html, /<img class="md-img" src="https:\/\/example\.com\/a\.png" alt="图一" loading="lazy">/);
  // javascript: 被丢弃，只留 alt 文本。
  const unsafe = render("![坏图](javascript:alert(1))");
  assert.doesNotMatch(unsafe, /<img/);
  assert.match(unsafe, /坏图/);
});

test("粗斜体、删除线与裸链接", () => {
  const html = render("***强调*** ~~过时~~ 见 https://example.com/doc 结尾");
  assert.match(html, /<strong><em>强调<\/em><\/strong>/);
  assert.match(html, /<del>过时<\/del>/);
  assert.match(html, /<a href="https:\/\/example\.com\/doc" target="_blank" rel="noopener">https:\/\/example\.com\/doc<\/a>/);
  // 已经生成的 href 不能被二次包成链接。
  assert.doesNotMatch(html, /href="<a/);
});

test("任务列表与有序列表起始号", () => {
  const html = render("- [ ] 待办\n- [x] 完成\n\n3. 第三\n4. 第四");
  assert.match(html, /md-task" aria-hidden="true">○<\/span> 待办/);
  assert.match(html, /md-task is-done" aria-hidden="true">✓<\/span> 完成/);
  assert.match(html, /<ol class="md-list" start="3">/);
});

test("嵌套列表用层级类表达缩进", () => {
  const html = render("- 一级\n  - 二级\n    - 三级");
  assert.match(html, /<li class="md-li">一级<\/li>/);
  assert.match(html, /<li class="md-li md-li-depth-1">二级<\/li>/);
  assert.match(html, /<li class="md-li md-li-depth-2">三级<\/li>/);
});

test("来源卡片按外链去重，跳过相对链接", () => {
  const html = renderMessage(
    "[官方文档](https://example.com/docs) 与 [同一处](https://example.com/docs) 以及 [本地](/docs/x)"
  );
  assert.match(html, /<div class="md-sources">/, "缺少来源区");
  const cardCount = (html.match(/class="md-source"/g) || []).length;
  assert.equal(cardCount, 1, "同一 href 只应生成一张卡片");
  assert.match(html, /class="md-source-host">example\.com</);
  assert.match(html, /class="md-source-label">官方文档</);
  // 没有外链时不生成空壳。
  assert.doesNotMatch(renderMessage("只有正文"), /md-sources/);
});

test("renderMessage = Markdown + 来源卡片", () => {
  const html = renderMessage("结论见 [论文](https://arxiv.org/abs/1)。");
  assert.match(html, /<a href="https:\/\/arxiv\.org\/abs\/1"/);
  assert.match(html, /md-sources-title">来源</);
});

test("来源卡片标签被转义，不执行 HTML", () => {
  // 标签里塞 HTML：先转义渲染，来源卡片再剥标签 + 转义。
  const html = renderMessage("[<img src=x onerror=alert(1)>](https://x.test/a)");
  // 安全的判据是"没有真实标签"：文本里出现 onerror 字样不算注入。
  assert.doesNotMatch(html, /<img/i);
  assert.doesNotMatch(html, /<script/i);
  assert.match(html, /class="md-source"/, "应仍生成来源卡片");
  assert.match(html, /onerror=alert\(1\)/, "标签文本应作为纯文本可见");
  // 正文里裸写的 HTML 标签也不能变成真实标签。
  const raw = render("<img src=x onerror=alert(1)>");
  assert.doesNotMatch(raw, /<img src=x/);
});

test("连续图片合成图集，单图与图文混排保持原样", () => {
  const both = render(
    "![a](https://example.com/a.png)\n\n![b](https://example.com/b.png)"
  );
  assert.match(both, /<div class="md-img-grid">/);
  assert.equal((both.match(/class="md-img"/g) || []).length, 2);

  const single = render("![a](https://example.com/a.png)");
  assert.doesNotMatch(single, /md-img-grid/);

  const mixed = render("看图：![a](https://example.com/a.png) 就懂了");
  assert.doesNotMatch(mixed, /md-img-grid/);

  // 3 张以上必须合成一个网格：旧实现只配对「单图段落」，
  // 第三张会掉出网格，4 张则被拆成两个双图网格。
  const three = render(
    "![a](https://example.com/a.png)\n\n![b](https://example.com/b.png)\n\n![c](https://example.com/c.png)"
  );
  assert.equal((three.match(/class="md-img-grid"/g) || []).length, 1);
  assert.equal((three.match(/class="md-img"/g) || []).length, 3);
  const four = render(
    "![a](https://example.com/a.png)\n\n![b](https://example.com/b.png)\n\n" +
      "![c](https://example.com/c.png)\n\n![d](https://example.com/d.png)"
  );
  assert.equal((four.match(/class="md-img-grid"/g) || []).length, 1);
  assert.equal((four.match(/class="md-img"/g) || []).length, 4);
  // 正文隔开的图片仍不能误合并。
  const separated = render(
    "![a](https://example.com/a.png)\n\n中间有字\n\n![b](https://example.com/b.png)"
  );
  assert.doesNotMatch(separated, /md-img-grid/);
});

test("表格外包滚动容器，窄屏不撑破气泡", () => {
  const html = render("| 名称 | 值 |\n| --- | --- |\n| A | 1 |");
  assert.match(html, /<div class="md-table-wrap">/);
  assert.match(html, /<table class="md-table">/);
  const wrapped = html.slice(html.indexOf("md-table-wrap"));
  assert.match(wrapped, /<\/table><\/div>/);
});

test("正文 [n] 变成来源锚点，代码块与无来源时保持原样", () => {
  const sources = [
    { href: "https://a.example/x", label: "A 资料", host: "a.example" },
    { href: "https://b.example/y", label: "B 资料", host: "b.example" },
  ];
  const html = md.linkCitations(
    "<div class=\"md-p\">结论见 [1]，另见 [2] 与 [9]</div>" +
      "<pre><code>[1]</code></pre>" +
      "<div class=\"md-p\">已有链接 <a href=\"https://c.example\">[1]</a></div>",
    sources
  );
  assert.match(html, /<a class="md-cite" href="https:\/\/a\.example\/x"[^>]*>1<\/a>/);
  assert.match(html, /href="https:\/\/b\.example\/y"[^>]*>2<\/a>/);
  assert.match(html, /\[9\]/, "无对应来源时保持字面量");
  assert.match(html, /<pre><code>\[1\]<\/code><\/pre>/, "代码块内不改写");
  assert.match(html, /<a href="https:\/\/c\.example">\[1\]<\/a>/, "链接内不改写");
  assert.equal(md.linkCitations("<p>[1]</p>", []), "<p>[1]</p>");
});

test("renderMessage 同时产出引用锚点与来源卡片", () => {
  const html = renderMessage("参考 [1] 的结论。\n\n[资料](https://learn.example/page)");
  assert.match(html, /class="md-cite"/);
  assert.match(html, /class="md-sources"/);
  assert.match(html, /learn\.example/);
});

test("chart 围栏渲染条形图，数据不可解析时回退代码块", () => {
  const chart = render(["```chart", "季度份额", "Q1, 25", "Q2, 100", "```"].join("\n"));
  assert.match(chart, /class="md-chart"/);
  assert.match(chart, /class="md-chart-title">季度份额/);
  assert.match(chart, /Q2<\/span><span class="md-chart-track"><span class="md-chart-bar" style="width:100%"/);
  assert.match(chart, /Q1<\/span><span class="md-chart-track"><span class="md-chart-bar" style="width:25%"/);
  assert.doesNotMatch(chart, /md-code/);

  const fallback = render(["```chart", "只有一行文字", "```"].join("\n"));
  assert.doesNotMatch(fallback, /md-chart/);
  assert.match(fallback, /class="md-code"/);
});

test("回答里的图片可点击放大（事件委托 + Esc 关闭）", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const css = readFileSync(new URL("../experience.css", import.meta.url), "utf8");
  assert.match(app, /function openImageViewer\(src, alt\)/);
  assert.match(app, /closest\("\.message-body \.md-img"\)/);
  assert.match(app, /if \(viewer && viewer\.classList\.contains\("is-open"\)\) closeImageViewer\(\);/);
  assert.match(css, /\.image-viewer\.is-open \{ display: flex; \}/);
  assert.match(css, /\.msg\.assistant \.md-img \{ cursor: zoom-in; \}/);
  // 防盗链/失效外链图片退化成可读占位，不留破图。
  assert.match(app, /document\.addEventListener\("error", \(event\) => \{/);
  assert.match(app, /image\.tagName !== "IMG" \|\| !image\.classList\.contains\("md-img"\)/);
  assert.match(app, /fallback\.className = "md-img-broken"/);
  assert.match(app, /image\.replaceWith\(fallback\)/);
  assert.match(app, /\}, true\);/);
  assert.match(css, /\.msg\.assistant \.md-img-broken \{/);
});

test("pie 围栏渲染占比环图，数值全零时回退代码块", () => {
  const html = render(["```pie", "渠道占比", "自然流量, 60", "付费, 40", "```"].join("\n"));
  assert.match(html, /class="md-chart md-pie"/);
  assert.match(html, /conic-gradient\(#60a5fa 0\.00% 60\.00%,#818cf8 60\.00% 100\.00%\)/);
  assert.match(html, /自然流量/);
  assert.match(html, /<em>60%<\/em>/);
  assert.match(html, /<em>40%<\/em>/);

  const zero = render(["```pie", "A, 0", "B, 0", "```"].join("\n"));
  assert.doesNotMatch(zero, /md-pie/);
  assert.match(zero, /class="md-code"/);
});

test("引用块支持嵌套层级，空引用行不产出空段落", () => {
  const html = render("> 一级\n> > 二级\n> > > 三级\n>\n> 回到一级");
  assert.equal((html.match(/<blockquote class="md-quote">/g) || []).length, 3);
  assert.equal((html.match(/<\/blockquote>/g) || []).length, 3);
  assert.match(html, /一级/);
  assert.match(html, /二级/);
  assert.match(html, /三级/);
  assert.doesNotMatch(html, /<div class="md-p"><\/div>/);
  // 引用结束后所有层级都闭合，后续段落不被吞进引用。
  const after = render("> 引用\n\n普通段落");
  assert.match(after, /<\/blockquote>\n\n<div class="md-p">普通段落<\/div>/);
});

test("标题支持到六级，五级六级有独立样式", () => {
  const html = render("##### 五级\n###### 六级");
  assert.match(html, /<h5 class="md-h5">五级<\/h5>/);
  assert.match(html, /<h6 class="md-h6">六级<\/h6>/);
  const css = readFileSync(new URL("../experience.css", import.meta.url), "utf8");
  assert.match(css, /\.msg\.assistant \.md-h5 \{ font-size: 14px; \}/);
  assert.match(css, /\.msg\.assistant \.md-h6 \{ font-size: 13px;/);
});

test("嵌套引用有视觉层级差异", () => {
  const css = readFileSync(new URL("../experience.css", import.meta.url), "utf8");
  assert.match(css, /\.msg\.assistant \.md-quote \.md-quote \{/);
});

test("引用块内部按 Markdown 递归渲染：列表、代码、表格、图表", () => {
  const list = render("> - 一\n> - 二");
  assert.match(list, /<blockquote class="md-quote"><ul class="md-list">/);
  assert.equal((list.match(/<li class="md-li">/g) || []).length, 2);

  const code = render("> ```rust\n> fn main() {}\n> ```");
  assert.match(code, /<blockquote class="md-quote"><pre class="md-code">/);
  assert.match(code, /<span>rust<\/span>/);
  assert.match(code, /fn main\(\) \{\}/);
  assert.match(code, /class="md-copy"/);

  const table = render("> | a | b |\n> | --- | --- |\n> | 1 | 2 |");
  assert.match(table, /<blockquote class="md-quote"><div class="md-table-wrap">/);
  assert.match(table, /<td style="text-align:left">1<\/td>/);
  // 外层 post-process 不能把内层已有的滚动容器再包一层。
  assert.equal((table.match(/class="md-table-wrap"/g) || []).length, 1);

  const chart = render("> ```chart\n> 份额\n> A, 50\n> B, 100\n> ```");
  assert.match(chart, /<blockquote class="md-quote"><div class="md-chart"/);
  assert.match(chart, /class="md-chart-bar" style="width:100%"/);
});

test("引用内公式与外层公式互不干扰", () => {
  const html = render("外层 $E=mc^2$ 结束\n\n> 引用 $y=1$ 结束");
  assert.match(html, /外层 <code class="md-tex">E=mc²<\/code> 结束/);
  assert.match(html, /引用 <code class="md-tex">y=1<\/code> 结束/);
  assert.equal((html.match(/<code class="md-tex">/g) || []).length, 2);
});

test("公式的上下标/分数/根号转成可读文本，不留下半个字符", () => {
  assert.equal(md.texToReadable("E=mc^2"), "E=mc²");
  assert.equal(md.texToReadable("10^{23}"), "10²³");
  assert.equal(md.texToReadable("a_1 + b_{n}"), "a₁ + bₙ");
  assert.equal(md.texToReadable("\\frac{1}{2}"), "(1)/(2)");
  assert.equal(md.texToReadable("\\sqrt{x}"), "√(x)");
  // 没有对应 Unicode 的指数保留 ^，但花括号会被清掉；不能变成 undefined。
  assert.equal(md.texToReadable("x^{\\alpha}"), "x^α");
  assert.doesNotMatch(md.texToReadable("x^{\\alpha}"), /undefined/);
  const html = render("质能方程 $E=mc^2$");
  assert.match(html, /<code class="md-tex">E=mc²<\/code>/);
  // 公式不是代码：不能继续背灰底/边框。
  assert.match(css, /\.msg\.assistant code\.md-tex \{[^}]*background: transparent;[^}]*font-family: "Cambria Math"/);
});

test("回答内容占满阅读列，短回答不把图表/引用压窄", () => {
  assert.match(
    css,
    /\.session-view > \.msg\.assistant \{[^}]*align-self: stretch;/
  );
});

test("顶层表格只包一层滚动容器", () => {
  const html = render("| a | b |\n| --- | --- |\n| 1 | 2 |");
  assert.equal((html.match(/class="md-table-wrap"/g) || []).length, 1);
  assert.equal((html.match(/class="md-table"/g) || []).length, 1);
});

test("流式未闭合围栏也会先渲染已到达的代码", () => {
  const open = render("```rust\nfn main() {}\nlet x = 1;");
  assert.match(open, /class="md-code"/);
  assert.match(open, /fn main\(\) \{\}/);
  assert.match(open, /let x = 1;/);
  assert.match(open, /<span>rust<\/span>/);

  // 未闭合的 chart 围栏：数据已能解析就直接成图，不能吞掉内容。
  const pending = render("```chart\n标题\nA, 50");
  assert.match(pending, /class="md-chart"/);
  assert.match(pending, /A<\/span>/);
  assert.match(pending, /md-chart-value">50</);
});
