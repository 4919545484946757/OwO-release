// 内容可视化契约（对标 Codex / DSH 的内容显示方式）：
// 1) 代码块 token 着色 + 行号栏（core/markdown.js highlightCode）；
// 2) 行级着色 diff（app-domain.js buildDiffView / refreshDiff 源级契约）；
// 3) 用量趋势与预算进度（app.js usageTrendHtml / usageBudgetHtml / 打开用量页即拉取）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const read = (file) => readFileSync(new URL("../" + file, import.meta.url), "utf8");
const app = read("app.js");
const domain = read("app-domain.js");

// 与 markdown-rendering.test.mjs 相同的加载口径：core/markdown.js 是唯一渲染源。
const markdownSource = read("core/markdown.js");
const sandbox = { window: { location: { href: "http://localhost/" } }, URL };
vm.createContext(sandbox);
vm.runInContext(markdownSource, sandbox);
const OwoMarkdown = sandbox.window.OwoMarkdown;

// ---------- 1) 代码高亮 ----------

test("highlightCode 给 rust 的属性/宏/关键字/字符串/注释分色", () => {
  const html = OwoMarkdown.highlightCode(
    ['#[derive(Debug)]', "fn run() {", '    let s = "owo";', "    println!(s); // tail", "}"].join("\n"),
    "rust",
  );
  assert.match(html, /<span class="tk-m">#\[derive\(Debug\)\]<\/span>/);
  assert.match(html, /<span class="tk-k">fn<\/span>/);
  assert.match(html, /<span class="tk-f">run<\/span>/);
  assert.match(html, /<span class="tk-s">"owo"<\/span>/);
  assert.match(html, /<span class="tk-m">println!<\/span>/);
  assert.match(html, /<span class="tk-c">\/\/ tail<\/span>/);
});

test("highlightCode 把 json 属性键与值区分着色，数字与字面量各自归类", () => {
  const html = OwoMarkdown.highlightCode('{"a": 1, "b": [true, null]}', "json");
  assert.match(html, /<span class="tk-p">"a"<\/span>/);
  assert.match(html, /<span class="tk-n">1<\/span>/);
  assert.match(html, /<span class="tk-b">true<\/span>/);
  assert.match(html, /<span class="tk-b">null<\/span>/);
});

test("highlightCode 未知语言纯转义；已知语言内 HTML 仍被转义", () => {
  const plain = OwoMarkdown.highlightCode("<img src=x>", "brainfuck");
  assert.equal(plain, "&lt;img src=x&gt;");
  const html = OwoMarkdown.highlightCode('<a href="x">t</a>', "html");
  // 标签名/属性名/字符串都分色，且原文里的标签字符全部转义（无裸 <a、无 onerror 可执行片段）。
  assert.doesNotMatch(html, /<a href=/);
  assert.doesNotMatch(html, /onerror=/);
  assert.match(html, /<span class="tk-t">&lt;a<\/span>/);
  assert.match(html, /<span class="tk-p">href<\/span>/);
  assert.match(html, /<span class="tk-s">"x"<\/span>/);
  assert.match(html, /&lt;\/a/);
});

test("renderMarkdown 多行代码带行号栏，单行代码不带；复制载荷仍是完整原文", () => {
  const multi = OwoMarkdown.renderMarkdown('```rust\nlet a = 1;\nlet b = 2;\n```');
  assert.match(multi, /<span class="md-lines" aria-hidden="true">1\n2<\/span>/);
  assert.match(multi, /<span class="md-src">/);
  const single = OwoMarkdown.renderMarkdown('```rust\nlet a = 1;\n```');
  assert.doesNotMatch(single, /md-lines/);
  const code = "const ok = true;\nreturn ok;";
  const round = OwoMarkdown.renderMarkdown("```typescript\n" + code + "\n```");
  assert.match(round, new RegExp('data-code="' + encodeURIComponent(code) + '"'));
  assert.match(round, /<span>typescript<\/span>/);
});

// ---------- 2) 行级着色 diff ----------

// 轻量 DOM 桩：buildDiffView 只用 createElement/append/appendChild/setAttribute。
function domStub() {
  const make = (tag) => ({
    tag,
    className: "",
    textContent: "",
    children: [],
    append(...nodes) { this.children.push(...nodes); },
    appendChild(node) { this.children.push(node); return node; },
  });
  return { createElement: make };
}

function loadBuildDiffView() {
  const source = /function buildDiffView\(diff\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(source, "buildDiffView 必须存在");
  const context = { document: domStub() };
  vm.createContext(context);
  return vm.runInContext(source + "\nbuildDiffView;", context);
}

test("buildDiffView 按行配对着色：改动行成对、未变行归上下文、行号双侧推进", () => {
  const buildDiffView = loadBuildDiffView();
  // 服务端口径：before/after 是文件内容字符串（app-domain.js 按 \n 拆行）。
  const diff = {
    path: "a.txt",
    before: "keep\nold\ntail",
    after: "keep\nnew\ntail",
  };
  const view = buildDiffView(diff);
  const rows = view.rows.children;
  assert.equal(view.rows.className, "diff-rows");
  // keep（上下文）+ old（删）+ new（增）+ tail（上下文）= 4 行。
  assert.equal(rows.length, 4);
  const marks = rows.map((row) => row.children[2].textContent);
  assert.deepEqual(marks, [" ", "-", "+", " "]);
  const classes = rows.map((row) => row.className);
  assert.deepEqual(classes, ["dl dl-ctx", "dl dl-del", "dl dl-add", "dl dl-ctx"]);
  // 双侧行号：上下文 1|1，删除 2|空，新增 空|2，尾部 3|3。
  const nos = rows.map((row) => row.children.slice(0, 2).map((n) => n.textContent));
  assert.deepEqual(nos, [["1", "1"], ["2", ""], ["", "2"], ["3", "3"]]);
  // 统计徽标：+1 / −1。
  assert.equal(view.stat.children[0].textContent, "+1");
  assert.equal(view.stat.children[1].textContent, "-1");
  assert.equal(view.added, 1);
  assert.equal(view.removed, 1);
});

test("buildDiffView 纯新增/纯删除文件分别全绿/全红且不产生对侧行号", () => {
  const buildDiffView = loadBuildDiffView();
  const added = buildDiffView({ path: "new.txt", before: null, after: "one\ntwo" });
  assert.equal(added.rows.children.every((row) => row.className === "dl dl-add"), true);
  assert.equal(added.stat.children.length, 1);
  assert.equal(added.stat.children[0].textContent, "+2");
  const deleted = buildDiffView({ path: "gone.txt", before: "one", after: null });
  assert.equal(deleted.rows.children[0].className, "dl dl-del");
  assert.equal(deleted.stat.children[0].textContent, "-1");
  // 文本经 textContent 落 DOM，不做 HTML 拼接（标签字符不会进 innerHTML）。
  const hostile = buildDiffView({ path: "x", before: "<img onerror=alert(1)>", after: null });
  assert.equal(hostile.rows.children[0].children[3].textContent, "<img onerror=alert(1)>");
});

test("refreshDiff 保持会话代际守卫与 hunk 审阅接线（源级契约）", () => {
  const fn = /async function refreshDiff\(sessionId\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(fn);
  assert.match(fn, /sessionDiffRefreshGeneration/);
  assert.match(fn, /sessionId !== state\.sessionId/);
  assert.match(fn, /renderHunkActions\(sessionId, generation, list\)/);
  assert.match(fn, /没有文件改动/);
  assert.match(fn, /buildDiffView\(diff\)/);
  // 头部组装不再走 innerHTML 拼接（路径由 textContent 落 DOM）。
  assert.doesNotMatch(fn, /innerHTML = .*(changed|diff\.path)/);
});

// ---------- 3) 用量趋势与预算 ----------

test("usageTrendHtml：不足两天不出图，两天起按比例出柱且悬停带明细", () => {
  const trendFn = /function usageTrendHtml\(report\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(trendFn, "usageTrendHtml 必须存在");
  // esc() 的真身走 DOM 转义；沙箱里用等价的纯文本转义（数值/日期断言不依赖实体形态）。
  const esc = (text) =>
    String(text == null ? "" : text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const context = { esc };
  vm.createContext(context);
  const trend = vm.runInContext(trendFn + "\nusageTrendHtml;", context);
  assert.equal(trend(null), "");
  assert.equal(trend({ by_day: [{ date: "2026-11-18", total_tokens: 10 }] }), "");
  const report = {
    by_day: [
      { date: "2026-11-17", prompt_tokens: 100, completion_tokens: 50, calls: 3, total_tokens: 150 },
      { date: "2026-11-18", prompt_tokens: 50, completion_tokens: 0, calls: 1, total_tokens: 50 },
    ],
  };
  const html = trend(report);
  assert.match(html, /usage-trend/);
  assert.equal((html.match(/usage-col"/g) || []).length, 2);
  // 最大值 150：第一天 in=66.67% out=33.33%，第二天 in=33.33%。
  assert.match(html, /height:66\.67%/);
  assert.match(html, /height:33\.33%/);
  assert.match(html, /11\/17/);
  assert.match(html, /输入 100/);
});

test("usageBudgetHtml：未配置上限不出条；配置后按占比出条且超限标红", () => {
  const budgetFn = /function usageBudgetHtml\(data\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(budgetFn, "usageBudgetHtml 必须存在");
  const formatCount = /function formatCount\(value\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(formatCount);
  const esc = (text) =>
    String(text == null ? "" : text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const context = { esc };
  vm.createContext(context);
  const budget = vm.runInContext(formatCount + "\n" + budgetFn + "\nusageBudgetHtml;", context);
  // 无预算：不出条。
  assert.equal(budget({ usage: { total_tokens: 100 }, budget: {} }), "");
  // 只配置 token 上限：一条，占比 25%。
  const html = budget({ usage: { total_tokens: 250 }, budget: { token_cap: 1000 }, cost_usd: null });
  assert.equal((html.match(/usage-budget-row/g) || []).length, 1);
  assert.match(html, /width:25\.0%/);
  assert.doesNotMatch(html, /is-over/);
  // 超限：标红；token 300% 与成本 250% 两条都出。
  const over = budget({ usage: { total_tokens: 3000 }, budget: { token_cap: 1000, cost_cap_usd: 1 }, cost_usd: 2.5 });
  assert.match(over, /is-over/);
  assert.match(over, /300%/);
  assert.equal((over.match(/usage-budget-row/g) || []).length, 2);
});

test("打开用量设置页立即拉取一次（不再停在「加载中…」等待失效事件）", () => {
  const fn = /function setSettingsTab\(name\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(fn);
  assert.match(fn, /if \(name === "usage"\) refreshUsage\(\)/);
});

test("refreshUsage 并行拉取趋势报表且失败不影响卡片（源级契约）", () => {
  const fn = /async function refreshUsage\(\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(fn);
  assert.match(fn, /api\("\/usage"\)/);
  assert.match(fn, /api\("\/usage\/report\?days=14"\)/);
  assert.match(fn, /usageBudgetHtml\(data\)/);
  assert.match(fn, /usageTrendHtml\(report\)/);
});

test("流式回答按 120ms 合并重渲，终态立即冲刷（源级契约）", () => {
  // 节流句柄与冲刷函数必须成对出现在 sendPrompt 里。
  assert.match(domain, /let streamFlushTimer = null;/);
  assert.match(domain, /const flushStreamingNow = \(\) => \{/);
  assert.match(domain, /setTimeout\(\(\) => \{[\s\S]*?flushStreamingNow\(\);[\s\S]*?\}, 120\)/);
  const tokenCase = /case "token_delta": \{[\s\S]*?\n        \}/.exec(domain)?.[0];
  assert.ok(tokenCase, "token_delta 分支必须存在");
  assert.match(tokenCase, /scheduleStreamingFlush\(\)/);
  // 每个 delta 全量 innerHTML 重渲（O(n²)）不得回归。
  assert.doesNotMatch(tokenCase, /innerHTML = renderMarkdown/);
  const finalCase = /case "final": \{[\s\S]*?\n        \}/.exec(domain)?.[0];
  assert.ok(finalCase, "final 分支必须存在");
  assert.match(finalCase, /flushStreamingNow\(\)/);
  // 收尾兜底：中断/异常后仍挂着的节流立即落地。
  const cleanup = /if \(streamFlushTimer && streaming\) \{[\s\S]*?\n    \}/.exec(domain)?.[0];
  assert.ok(cleanup, "finally 收尾必须冲刷未落地的流式内容");
  assert.match(cleanup, /clearTimeout\(streamFlushTimer\)/);
});

test("流式更新只重写 .message-body（与历史回放同构，保住字号/行距样式）", () => {
  assert.match(domain, /let streamingBody = null;/);
  assert.match(domain, /streamingBody = streaming\.querySelector\("\.message-body"\)/);
  // 冲刷必须以 message-body 为目标：整气泡重写会抹掉包装器，实时消息字号与回放不一致。
  const flush = /const flushStreamingNow = \(\) => \{[\s\S]*?\n  \};/.exec(domain)?.[0];
  assert.ok(flush);
  assert.match(flush, /const target = streamingBody \|\| streaming;/);
  assert.match(flush, /target\.innerHTML = renderMarkdown\(assistantText\)/);
});

test("formatRelativeTime：相对时间分级 + 无效/未来时间回落短日期", () => {
  const fn = /function formatRelativeTime\(raw\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(fn, "formatRelativeTime 必须存在");
  const frozenNow = Date.parse("2026-11-18T21:00:00+08:00");
  const context = { Date: { parse: Date.parse, now: () => frozenNow } };
  vm.createContext(context);
  const format = vm.runInContext(fn + "\nformatRelativeTime;", context);
  assert.equal(format(""), "");
  assert.equal(format("not-a-timestamp!"), "-timestamp!"); // 解析失败回落原始截串
  assert.equal(format("2026-11-19T09:00:00+08:00"), "11-19 09:00"); // 未来时间回落
  assert.equal(format("2026-11-18T20:59:40+08:00"), "刚刚");
  assert.equal(format("2026-11-18T20:55:00+08:00"), "5 分钟前");
  assert.equal(format("2026-11-18T18:00:00+08:00"), "3 小时前");
  assert.equal(format("2026-11-16T21:00:00+08:00"), "2 天前");
  assert.equal(format("2026-11-08T21:00:00+08:00"), "11-08 21:00"); // 一周外回落
});
