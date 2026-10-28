"use strict";

/**
 * OwO Agent SDK 官网单页 — 静态产物契约测试
 *
 * 运行：node --test tests/design.test.cjs
 * 仅使用 Node 内置模块（node:test / node:assert / node:fs / node:path），无外部依赖。
 * 覆盖：文件存在与非空、HTML 结构语义与文案、lorem ipsum 占位文案、CSS 变量/媒体查询、
 *       HTML lang 与内容语言一致性、HTML 与 CSS 的选择器引用一致性、可访问性要点。
 *
 * 说明：测试按「实际产物的事实」断言，不替产物臆造结论。例如 <html lang="en"> 与正文为
 * 简体中文这一口径差异，以「记录型断言」固定下来（会失败并给出明确修复指引），
 * 由 critic / designer 决定改 lang 还是改文案。
 */

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const HTML_PATH = path.join(ROOT, "index.html");
const CSS_PATH = path.join(ROOT, "styles.css");

const readText = (filePath) => fs.readFileSync(filePath, "utf8");

const html = readText(HTML_PATH);
const css = readText(CSS_PATH);

/** 去掉 HTML 注释，避免注释里的文字干扰文案断言 */
const htmlNoComments = html.replace(/<!--[\s\S]*?-->/g, "");

/** 去掉 CSS 注释，避免注释干扰选择器/变量断言 */
const cssNoComments = css.replace(/\/\*[\s\S]*?\*\//g, "");

/** 提取 HTML 中全部 class 名 */
const htmlClasses = new Set();
for (const m of htmlNoComments.matchAll(/\bclass="([^"]*)"/g)) {
  for (const name of m[1].trim().split(/\s+/)) {
    if (name) htmlClasses.add(name);
  }
}

/** 提取 HTML 中全部 id */
const htmlIds = new Set();
for (const m of htmlNoComments.matchAll(/\bid="([^"]+)"/g)) {
  htmlIds.add(m[1].trim());
}

/** 提取 HTML 中全部内部锚点 href（#xxx） */
const htmlAnchors = new Set();
for (const m of htmlNoComments.matchAll(/\bhref="#([^"]+)"/g)) {
  htmlAnchors.add(m[1].trim());
}

/** 提取 <section> 标签与各自的 aria-labelledby 值 */
const sections = [...htmlNoComments.matchAll(/<section\b[^>]*>/g)].map((m) => m[0]);
const sectionLabelledBy = sections.map((tag) => {
  const m = tag.match(/aria-labelledby="([^"]+)"/);
  return m ? m[1] : null;
});

/** 提取 CSS 中定义的类选择器（形如 .foo） */
const cssClassSelectors = new Set();
for (const m of cssNoComments.matchAll(/\.(-?[_a-zA-Z][_a-zA-Z0-9-]*)/g)) {
  cssClassSelectors.add(m[1]);
}

/** 统计字符出现次数（用于括号配对检查） */
const countChar = (s, ch) => s.split(ch).length - 1;

/** 提取 CSS 自定义属性定义（--name: ...） */
const cssCustomProps = new Set();
for (const m of cssNoComments.matchAll(/(--[_a-zA-Z0-9-]+)\s*:/g)) {
  cssCustomProps.add(m[1]);
}

/** 提取 CSS var(--x) 引用 */
const cssVarRefs = new Set();
for (const m of cssNoComments.matchAll(/var\(\s*(--[_a-zA-Z0-9-]+)/g)) {
  cssVarRefs.add(m[1]);
}

const isCjk = (ch) => /[\u4e00-\u9fff]/.test(ch);

/** 剔除代码样例与标签后的可见正文，用于语言与长度判定 */
const proseText = htmlNoComments
  .replace(/<(script|style|svg|pre|code)\b[\s\S]*?<\/\1>/gi, " ")
  .replace(/<[^>]+>/g, " ")
  .replace(/\s+/g, " ")
  .trim();

// ---------------------------------------------------------------- 文件与基础
test("index.html 与 styles.css 存在且非空", () => {
  for (const p of [HTML_PATH, CSS_PATH]) {
    assert.ok(fs.existsSync(p), `${path.basename(p)} 应存在`);
    const stat = fs.statSync(p);
    assert.ok(stat.isFile(), `${path.basename(p)} 应为普通文件`);
    assert.ok(stat.size > 1000, `${path.basename(p)} 应有实质内容（>1000 字节）`);
    assert.ok(readText(p).trim().length > 0, `${path.basename(p)} 不应为空白`);
  }
});

test("index.html 具备标准文档骨架与外部样式引用", () => {
  assert.match(html, /^<!doctype html>/i, "应以 <!doctype html> 开头");
  assert.match(html, /<html\b[^>]*lang="/i, "<html> 必须声明 lang");
  assert.match(htmlNoComments, /<head>[\s\S]*<title>[^<]{4,}<\/title>[\s\S]*<\/head>/);
  assert.match(htmlNoComments, /<meta\s+charset="utf-8"/i, "应声明 utf-8");
  assert.match(
    htmlNoComments,
    /<meta\s+name="viewport"\s+content="[^"]*width=device-width/i,
    "应声明响应式 viewport"
  );
  assert.match(htmlNoComments, /<link\s+rel="stylesheet"\s+href="styles\.css"/, "应外链 styles.css");
  assert.equal((htmlNoComments.match(/<h1\b/g) || []).length, 1, "h1 应恰好一个");
  assert.match(htmlNoComments, /<body>[\s\S]*<main\b[\s\S]*<\/main>[\s\S]*<footer\b/);
});

// ---------------------------------------------------------------- 语言口径
test("lang 属性格式合法且与页面主语言匹配", () => {
  const langMatch = html.match(/<html\b[^>]*\blang="([^"]+)"/i);
  assert.ok(langMatch, "缺少 lang 属性");
  const lang = langMatch[1].trim();
  assert.match(lang, /^[a-zA-Z]{2,3}(-[a-zA-Z0-9]{2,8})*$/, `lang 值格式非法：${lang}`);

  // 正文（剔除代码样例与 <code>）以汉字数量判断主语言
  const cjkCount = [...proseText].filter(isCjk).length;
  assert.ok(cjkCount > 200, `可见正文中文量过少（${cjkCount} 汉字），页面内容可能被替换`);

  assert.match(
    lang,
    /^zh\b/i,
    `界面正文为简体中文（${cjkCount} 个汉字），但 <html lang="${lang}">；` +
      "应将 lang 改为 zh-CN（或把全部文案改为英文）"
  );
});

// ---------------------------------------------------------------- 占位文案
test("不含 lorem ipsum 等占位文案", () => {
  for (const [name, body] of [
    ["index.html", htmlNoComments],
    ["styles.css", cssNoComments],
  ]) {
    assert.doesNotMatch(body, /lorem\s+ipsum/i, `${name} 不应出现 lorem ipsum`);
    assert.doesNotMatch(body, /\b(dolor\s+sit\s+amet|consectetur)\b/i, `${name} 不应出现占位句`);
    assert.doesNotMatch(body, /\b(TBD|TODO|FIXME|XXX|coming\s+soon)\b/i, `${name} 不应出现待办占位`);
    assert.doesNotMatch(body, /\bplaceholder\s+text\b/i, `${name} 不应出现 placeholder text`);
  }
});

// ---------------------------------------------------------------- 页面区块
test("必备区块、图片说明与页脚锚点齐全", () => {
  for (const id of ["hero", "capabilities", "quickstart", "security", "footer"]) {
    assert.ok(htmlIds.has(id), `缺少区块 id="${id}"`);
  }
  assert.ok(sections.length >= 4, `应至少有 4 个 <section>，实际 ${sections.length}`);
  for (const labelledBy of sectionLabelledBy) {
    assert.ok(labelledBy, "每个 <section> 都应有 aria-labelledby");
    assert.ok(htmlIds.has(labelledBy), `aria-labelledby="${labelledBy}" 指向不存在的 id`);
  }
});

test("所有内部锚点都有对应的目标 id", () => {
  for (const anchor of htmlAnchors) {
    assert.ok(htmlIds.has(anchor), `内部链接 #${anchor} 没有对应 id`);
  }
});

// ---------------------------------------------------------------- 文案
test("关键文案与产品调性完整", () => {
  const required = [
    "OwO Agent SDK",
    "统一接口",
    "权限内建",
    "调用可审计",
    "快速开始",
    "查看文档",
    "开始使用",
    "安全",
  ];
  for (const phrase of required) {
    assert.ok(htmlNoComments.includes(phrase), `页面应包含关键文案「${phrase}」`);
  }
  assert.match(htmlNoComments, /<title>[^<]*OwO Agent SDK[^<]*<\/title>/, "title 应含产品名");
  assert.match(
    htmlNoComments,
    /<meta\s+name="description"\s+content="[^"]{40,}"/,
    "应提供非空 description"
  );
  // 文案实质长度：去标签后正文应有足够内容，而非骨架
  assert.ok(proseText.length >= 600, `可见正文过短（${proseText.length} 字符）`);
});

// ---------------------------------------------------------------- 可访问性
test("可访问性要点：跳过链接、语义导航、可见焦点与减动效", () => {
  assert.match(htmlNoComments, /class="skip-link"\s+href="#hero"/, "应有指向主内容的跳过链接");
  assert.match(htmlNoComments, /<nav\b[^>]*aria-label="[^"]+"/, "导航应有 aria-label");
  assert.match(htmlNoComments, /<svg\b[^>]*aria-hidden="true"/, "装饰性图标应对辅助技术隐藏");
  assert.match(cssNoComments, /:focus-visible\s*\{[^}]*outline:/s, "应定义可见焦点环");
  assert.match(
    cssNoComments,
    /@media\s*\(\s*prefers-reduced-motion:\s*reduce\s*\)/,
    "应支持减动效偏好"
  );
  for (const tag of htmlNoComments.match(/<img\b[^>]*>/g) || []) {
    assert.match(tag, /\balt="/, `图片缺少 alt：${tag}`);
  }
});

// ---------------------------------------------------------------- HTML ↔ CSS 一致性
test("index.html 中的类名都在 styles.css 中有样式（或为结构钩子类）", () => {
  // 结构钩子：仅用于标识区块/组合，复用其它类实现样式，无需独立规则
  const allowedHooks = new Set([
    "hero-code", // hero 区代码卡片（复用 .code-card）
    "section-quickstart", // 快速开始区块标识
    "footer-brand", // 页脚品牌组合
    "compare-head", // 已在 CSS 定义，保留以说明分组语义
  ]);
  const unstyled = [...htmlClasses].filter(
    (name) => !cssClassSelectors.has(name) && !allowedHooks.has(name)
  );
  assert.deepEqual(unstyled, [], `以下 HTML 类名缺少 CSS 规则：${unstyled.join(", ")}`);
});

test("styles.css 中引用的类名都真实存在于 index.html", () => {
  const orphans = [...cssClassSelectors].filter((name) => !htmlClasses.has(name));
  assert.deepEqual(orphans, [], `以下 CSS 类名在 HTML 中不存在：${orphans.join(", ")}`);
});

// ---------------------------------------------------------------- CSS 质量
test("styles.css 结构完整：括号配对、设计变量定义与引用闭合", () => {
  assert.equal(countChar(cssNoComments, "{"), countChar(cssNoComments, "}"), "花括号未配对");
  assert.ok(cssCustomProps.size >= 10, `设计变量过少（${cssCustomProps.size}）`);
  for (const key of ["--bg", "--text", "--accent", "--space-1", "--max-width", "--radius"]) {
    assert.ok(cssCustomProps.has(key), `应定义设计变量 ${key}`);
  }
  const undefinedVars = [...cssVarRefs].filter((name) => !cssCustomProps.has(name));
  assert.deepEqual(undefinedVars, [], `引用了未定义的变量：${undefinedVars.join(", ")}`);
  assert.equal(countChar(cssNoComments, "("), countChar(cssNoComments, ")"), "圆括号未配对");
});

test("styles.css 响应式与基础排版", () => {
  const breakpoints = [...cssNoComments.matchAll(/@media\s*\(\s*min-width:\s*(\d+)px\s*\)/g)].map(
    (m) => Number(m[1])
  );
  assert.ok(breakpoints.length >= 2, `至少应有 2 个断点，实际 ${breakpoints.length}`);
  assert.ok(breakpoints.includes(768), "应包含 768px 断点");
  assert.ok(breakpoints.some((bp) => bp >= 1024), "应包含桌面断点（>=1024px）");
  assert.match(cssNoComments, /\bbody\s*\{[^}]*line-height:\s*[0-9.]+/s, "body 应显式设置 line-height");
  assert.match(cssNoComments, /line-height:\s*1\.[4-9]/, "行高应 ≥1.4 以利阅读");
  assert.match(cssNoComments, /box-sizing:\s*border-box/, "应统一 box-sizing");
});
