"use strict";

/**
 * index.html / styles.css 静态契约测试（零依赖，仅用 Node 内置 fs / path / assert / node:test）
 *
 * 运行：node --test tests/design.test.cjs
 *
 * 断言分组：
 *   1. 文件存在、为 UTF-8 文本且非空
 *   2. 无占位 / 示例 / 乱码文本（lorem ipsum、TODO、示例域名等）
 *   3. CSS 自定义属性定义与引用闭合（不引用未定义 token）
 *   4. CSS 括号 / 注释 / 引号配平 + 响应式与可达性基线 + 结构 class 覆盖率
 *   5. HTML lang 必填且合法，charset / viewport / title / description / styles.css 引用齐备
 *   6. 语义化结构与无障碍：唯一 h1、标题不跳级、main/header/footer/nav、SVG 与按钮可访问名、导航开关 aria
 *   7. 页内锚点闭合（每个 #fragment 都有对应 id）
 *   8. 可读性：明暗两套主题 token + 正文对比度 ≥ 7:1（AAA）
 *   9. 不依赖外部脚本 / 远程资源，内联脚本可解析
 */

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const ROOT = path.resolve(__dirname, "..");
const HTML_PATH = path.join(ROOT, "index.html");
const CSS_PATH = path.join(ROOT, "styles.css");

/* ------------------------------------------------------------------ helpers */

function readText(filePath) {
  return fs.readFileSync(filePath, "utf8").replace(/^\uFEFF/, "");
}

function lineOf(source, index) {
  return source.slice(0, index).split("\n").length;
}

/** 极简标签扫描：返回起始标签（已剔除注释），用于结构断言，不追求完整 HTML 解析。 */
function scanTags(html) {
  const stripped = html.replace(/<!--[\s\S]*?-->/g, "");
  const tags = [];
  const re = /<([a-zA-Z][a-zA-Z0-9-]*)((?:"[^"]*"|'[^']*'|[^>"'])*)>/g;
  let m;
  while ((m = re.exec(stripped)) !== null) {
    const attrs = {};
    const attrRe = /([a-zA-Z_:][-a-zA-Z0-9_:.]*)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'))?/g;
    let a;
    while ((a = attrRe.exec(m[2] || "")) !== null) {
      if (!a[1]) continue;
      attrs[a[1].toLowerCase()] = a[2] !== undefined ? a[2] : a[3] !== undefined ? a[3] : "";
    }
    tags.push({ name: m[1].toLowerCase(), raw: m[0], attrs, index: m.index, line: lineOf(stripped, m.index) });
  }
  return { tags, stripped };
}

/** 去掉标签后的可见文本 */
function visibleText(html) {
  return html
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<(script|style)\b[\s\S]*?<\/\1>/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/\s+/g, " ")
    .trim();
}

/** 去掉 CSS 注释，便于对“有效代码”做结构断言 */
function stripCssComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/** 从 index 起截取到第一个匹配的闭合标签，返回其内容（用于取元素内文本） */
function innerOf(source, tagName, index) {
  const end = source.indexOf(`</${tagName}>`, index);
  if (end === -1) return "";
  return source.slice(source.indexOf(">", index) + 1, end);
}

/** 花括号配平：返回结尾深度与出现过的最小深度（<0 表示提前闭合） */
function bracesBalanced(code) {
  let depth = 0;
  let min = 0;
  for (const ch of code) {
    if (ch === "{") depth += 1;
    else if (ch === "}") depth -= 1;
    if (depth < min) min = depth;
  }
  return { depth, min };
}

/** WCAG 2.1 相对亮度与对比度 */
function relativeLuminance(hex) {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h.slice(0, 6);
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(full.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRatio(a, b) {
  const [l1, l2] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

// 结构化 CSS 规则（去注释后按 "选择器 { 声明 }" 顺序收集，嵌套 @media 单独处理）
function collectRules(code) {
  const rules = [];
  const flat = code.replace(/@media[^{]+\{/g, "@media{");
  const re = /([^{}@]+)\{([^{}]*)\}/g;
  let m;
  while ((m = re.exec(flat)) !== null) {
    rules.push({ selector: m[1].trim(), body: m[2] });
  }
  return rules;
}

const html = readText(HTML_PATH);
const css = readText(CSS_PATH);
const { tags } = scanTags(html);
const cssCode = stripCssComments(css);
const ids = new Set(tags.filter((t) => t.attrs.id).map((t) => t.attrs.id));

/* ------------------------------------------------- 1. 文件存在且非空（基础） */

test("index.html 与 styles.css 存在、UTF-8 文本且非空", () => {
  for (const file of [HTML_PATH, CSS_PATH]) {
    const name = path.basename(file);
    assert.ok(fs.existsSync(file), `${name} 不存在`);
    const stat = fs.statSync(file);
    assert.ok(stat.isFile(), `${name} 不是普通文件`);
    assert.ok(stat.size > 2000, `${name} 体积过小（${stat.size}B），疑似未写完`);
    assert.ok(!/\uFFFD/.test(readText(file)), `${name} 含替换字符 U+FFFD，疑似编码损坏`);
  }
  assert.ok(html.trim().startsWith("<!DOCTYPE html>"), "index.html 缺少 <!DOCTYPE html> 声明");
});

/* ------------------------------------------------------- 2. 占位 / 乱码文本 */

test("不含占位与示例文本（lorem ipsum / TODO / 示例域名等）", () => {
  const placeholderPatterns = [
    /\blorem\b/i,
    /\bipsum\b/i,
    /\bdolor sit amet\b/i,
    /\bfoo bar\b/i,
    /\bTODO\b/,
    /\bFIXME\b/,
    /\bTBD\b/,
    /待补充/,
    /此处填写/,
    /占位/,
    /\bplaceholder text\b/i,
    /example\.com/i,
    /\bacme\b/i,
    /your-company/i,
    /公司名/,
    /xxx+/i,
    /\bNaN\b/,
  ];
  const offenders = [];
  for (const [name, source] of [["index.html", html], ["styles.css", css]]) {
    for (const pattern of placeholderPatterns) {
      const m = source.match(pattern);
      if (m) offenders.push(`${name} 命中占位文本：${JSON.stringify(m[0])}`);
    }
  }
  assert.deepEqual(offenders, [], offenders.join("\n"));
});

test("可见文本是真实中文内容，长度与语言占比达标", () => {
  const text = visibleText(html);
  assert.ok(text.length >= 600, `可见文本仅 ${text.length} 字，内容量不足`);
  const cjk = (text.match(/[\u4e00-\u9fff]/g) || []).length;
  const ratio = cjk / text.length;
  assert.ok(ratio >= 0.3, `中文占比仅 ${(ratio * 100).toFixed(1)}%，疑似未本地化或占位文本`);
});

/* --------------------------------------------------------------- 3. CSS token */

test("CSS 自定义属性定义与引用闭合", () => {
  const defined = new Set();
  for (const m of cssCode.matchAll(/(--[a-zA-Z0-9-]+)\s*:/g)) defined.add(m[1]);
  assert.ok(defined.size >= 15, `仅定义 ${defined.size} 个 CSS 变量，token 体系不完整`);

  const referenced = Array.from(new Set(Array.from(cssCode.matchAll(/var\(\s*(--[a-zA-Z0-9-]+)/g), (m) => m[1])));
  const missing = referenced.filter((name) => !defined.has(name));
  assert.deepEqual(missing, [], `引用了未定义的 CSS 变量：${missing.join(", ")}`);

  for (const key of ["--color-text", "--color-bg", "--color-accent", "--space-4", "--container-max", "--header-height"]) {
    assert.ok(defined.has(key), `缺少关键 token ${key}`);
  }
});

test("CSS 括号、注释与引号结构配平", () => {
  const { depth, min } = bracesBalanced(cssCode);
  assert.equal(depth, 0, `CSS 花括号不配平，结尾深度 ${depth}`);
  assert.equal(min, 0, "CSS 出现多余的 }（提前闭合）");
  assert.equal((css.match(/\/\*/g) || []).length, (css.match(/\*\//g) || []).length, "CSS 注释未闭合");
  assert.equal((cssCode.match(/"/g) || []).length % 2, 0, "CSS 双引号数量为奇数，疑似字符串未闭合");
  assert.ok(!/\{\s*\{/.test(cssCode), "CSS 存在非法嵌套规则块");
  assert.ok(!/;\s*;\s*}/.test(cssCode), "CSS 声明区存在连续分号");
});

test("CSS 覆盖关键视觉与响应式要求", () => {
  assert.match(cssCode, /:root\s*\{/, "缺少 :root 变量作用域");
  assert.match(cssCode, /box-sizing\s*:\s*border-box/, "缺少 box-sizing 重置");
  assert.match(cssCode, /@media\s*\(min-width:\s*\d+px\)/, "缺少 min-width 断点（移动端优先策略）");
  assert.match(cssCode, /:focus-visible/, "缺少 :focus-visible 焦点样式，键盘可达性不达标");
  assert.match(cssCode, /prefers-reduced-motion/, "缺少 prefers-reduced-motion 降级");
  assert.match(cssCode, /min-height:\s*44px/, "缺少 44px 最小触达尺寸");
  assert.match(cssCode, /line-height:\s*1\.\d/, "缺少行高设置");
  // 仅校验 BEM 块级骨架类：元素级修饰类允许由父级布局继承，避免误报
  const skeletonClasses = [
    "skip-link",
    "container",
    "section",
    "btn",
    "site-header",
    "header__inner",
    "brand",
    "nav-toggle",
    "primary-nav",
    "nav-list",
    "hero",
    "hero__inner",
    "hero__copy",
    "hero__visual",
    "topology",
    "card-grid",
    "card",
    "card__code",
    "steps",
    "codeblock",
    "codeblock__pre",
    "metrics",
    "metric",
    "metric__value",
    "reasons",
    "final-cta",
    "site-footer",
    "footer__inner",
    "footer__list",
  ];
  const htmlClasses = new Set(
    tags.flatMap((t) => (t.attrs.class ? t.attrs.class.split(/\s+/).filter(Boolean) : []))
  );
  const declaredButUnstyled = skeletonClasses.filter((cls) => htmlClasses.has(cls) && !new RegExp(`\\.${cls}(?![a-zA-Z0-9_-])`).test(cssCode));
  assert.deepEqual(declaredButUnstyled, [], `以下结构 class 在 CSS 中无样式：${declaredButUnstyled.join(", ")}`);
  const skeletonMissingInHtml = skeletonClasses.filter((cls) => !htmlClasses.has(cls));
  assert.deepEqual(skeletonMissingInHtml, [], `以下结构 class 在 HTML 中缺失：${skeletonMissingInHtml.join(", ")}`);

  // 关键 class 必须有实际声明体（而非只在注释/选择器列表里出现）
  const rules = collectRules(cssCode);
  for (const cls of ["btn", "card", "container", "site-header", "site-footer"]) {
    const hit = rules.some((r) => new RegExp(`\\.${cls}(?![a-zA-Z0-9_-])`).test(r.selector) && r.body.trim().length > 0);
    assert.ok(hit, `.${cls} 缺少实际样式声明`);
  }
});

/* ------------------------------------------------------------------ 4. HTML 头 */

test("HTML lang 必填且为合法语言标签", () => {
  const htmlTag = tags.find((t) => t.name === "html");
  assert.ok(htmlTag, "缺少 <html> 标签");
  const lang = (htmlTag.attrs.lang || "").trim();
  assert.notEqual(lang, "", "<html> 缺少 lang 属性");
  assert.match(lang, /^[a-zA-Z]{2,3}(-[a-zA-Z0-9]{2,8})*$/, `lang="${lang}" 不是合法语言标签`);
  assert.match(lang, /^zh(-|$)/i, `lang="${lang}" 与中文内容不符（期望 zh-CN 一族）`);
});

test("文档元信息与样式表引用完整", () => {
  assert.ok(tags.some((t) => t.name === "head"), "缺少 <head>");
  assert.ok(
    tags.some((t) => t.name === "meta" && (t.attrs.charset || "").toLowerCase() === "utf-8"),
    "缺少 <meta charset=\"utf-8\">"
  );
  assert.ok(
    tags.some((t) => t.name === "meta" && /width=device-width/i.test(t.attrs.content || "")),
    "缺少移动端 viewport 设置"
  );
  const titleText = (html.match(/<title>([\s\S]*?)<\/title>/i) || [, ""])[1].trim();
  assert.ok(titleText.length >= 6, "页面 <title> 缺失或过短");
  const desc = tags.find((t) => t.name === "meta" && (t.attrs.name || "").toLowerCase() === "description");
  assert.ok(desc, "缺少 meta description");
  assert.ok((desc.attrs.content || "").trim().length >= 30, "meta description 过短");

  const links = tags.filter((t) => t.name === "link" && /stylesheet/i.test(t.attrs.rel || ""));
  assert.equal(links.length, 1, `应只有一个样式表引用，实际 ${links.length} 个`);
  const href = links[0].attrs.href;
  assert.equal(href, "styles.css", `样式表应为同目录相对路径 styles.css，实际 ${href}`);
  assert.ok(fs.existsSync(path.join(ROOT, href)), `样式表 ${href} 不存在`);

  const external = tags.filter(
    (t) =>
      (t.name === "link" && /^(https?:)?\/\//i.test(t.attrs.href || "")) ||
      (t.name === "script" && /^(https?:)?\/\//i.test(t.attrs.src || "")) ||
      (t.name === "img" && /^(https?:)?\/\//i.test(t.attrs.src || ""))
  );
  assert.deepEqual(external.map((t) => t.raw), [], "存在外部 CDN 资源引用，脱离离线可打开要求");
  assert.ok(!/@import\s+url\(\s*["']?https?:/i.test(cssCode), "CSS 中存在 @import 远程资源");
  assert.ok(!/url\(\s*["']?https?:\/\//i.test(cssCode), "CSS 中存在远程资源引用");
});

/* ------------------------------------------------------- 5. 语义与无障碍 */

test("语义化骨架存在且数量正确（header 允许出现在 section 内）", () => {
  const count = (name) => tags.filter((t) => t.name === name).length;
  assert.equal(count("main"), 1, `<main> 应唯一，实际 ${count("main")}`);
  assert.equal(count("footer"), 1, `<footer> 应唯一，实际 ${count("footer")}`);
  assert.ok(count("header") >= 1, "缺少 <header>");
  assert.ok(count("nav") >= 2, `导航区不足（${count("nav")} 个）：需主导航与页尾导航`);
  assert.ok(count("section") >= 4, `区块过少（${count("section")} 个），页面结构不完整`);
  assert.ok(count("h2") >= 3, `h2 数量不足（${count("h2")} 个）`);
  assert.ok(count("h3") >= 3, `h3 数量不足（${count("h3")} 个）`);

  const main = tags.find((t) => t.name === "main");
  assert.ok(main.attrs.id, "<main> 缺少 id，skip link 无法定位");

  const bodyStart = html.indexOf("<body");
  const firstAfterBody = tags.find((t) => t.index > bodyStart);
  assert.ok(firstAfterBody, "<body> 内无元素");
  assert.equal(firstAfterBody.name, "a", "body 首个子元素应为跳转到主内容的 skip link");
  assert.equal(firstAfterBody.attrs.href, `#${main.attrs.id}`, "skip link 应指向 <main> 的 id");
  assert.ok(/skip-link/.test(firstAfterBody.attrs.class || ""), "skip link 缺少 skip-link class");

  const headerIndex = tags.find((t) => t.name === "header").index;
  assert.ok(headerIndex < main.index, "站点 header 应位于 <main> 之前");
});

test("标题层级：唯一 h1，且不跳级", () => {
  const headings = tags.filter((t) => /^h[1-6]$/.test(t.name));
  assert.ok(headings.length >= 6, `标题数量过少（${headings.length}）`);
  const h1s = headings.filter((t) => t.name === "h1");
  assert.equal(h1s.length, 1, `应只有一个 h1，实际 ${h1s.length} 个`);
  let previous = 1;
  for (const heading of headings.slice(1)) {
    const level = Number(heading.name[1]);
    assert.ok(level - previous <= 1, `第 ${heading.line} 行标题由 h${previous} 跳到 h${level}，层级断裂`);
    previous = level;
  }
  const heroStart = tags.find((t) => t.name === "section" && t.attrs.id === "hero");
  assert.ok(heroStart, "缺少 id=hero 的首屏区块");
  assert.ok(h1s[0].index > heroStart.index, "h1 不在首屏 hero 区块内");
  assert.ok(h1s[0].attrs.id, "h1 缺少 id，无法作为 aria-labelledby 目标");
});

test("区块与图形具备可访问名", () => {
  for (const section of tags.filter((t) => t.name === "section")) {
    const label = section.attrs["aria-labelledby"] || section.attrs["aria-label"];
    assert.ok(label, `第 ${section.line} 行的 <section> 缺少 aria-labelledby / aria-label`);
    if (section.attrs["aria-labelledby"]) {
      assert.ok(ids.has(section.attrs["aria-labelledby"]), `aria-labelledby="${section.attrs["aria-labelledby"]}" 指向的 id 不存在`);
    }
  }
  const svgs = tags.filter((t) => t.name === "svg");
  assert.ok(svgs.length >= 2, `内联 SVG 过少（${svgs.length}）`);
  for (const svg of svgs) {
    const labelled = svg.attrs["aria-labelledby"] || svg.attrs["aria-label"] || svg.attrs["aria-hidden"] === "true";
    assert.ok(labelled, `第 ${svg.line} 行的 <svg> 既无 aria 名称也未 aria-hidden`);
    if (svg.attrs["aria-labelledby"]) {
      assert.ok(ids.has(svg.attrs["aria-labelledby"]), `SVG aria-labelledby="${svg.attrs["aria-labelledby"]}" 未找到对应 <title id>`);
    }
  }
  assert.ok(svgs.some((t) => t.attrs["aria-labelledby"]), "应至少有一个 SVG 通过 aria-labelledby 提供文字描述");

  const buttons = tags.filter((t) => t.name === "button");
  assert.ok(buttons.length >= 2, `按钮过少（${buttons.length}）`);
  for (const btn of buttons) {
    const innerText = visibleText(innerOf(html, "button", btn.index));
    const named = innerText || btn.attrs["aria-label"] || btn.attrs["aria-labelledby"];
    assert.ok(named, `第 ${btn.line} 行的 <button> 无可见文本也无 aria-label`);
    assert.equal(btn.attrs.type, "button", `第 ${btn.line} 行的 <button> 应显式 type="button"`);
  }
  const toggle = tags.find((t) => t.name === "button" && "aria-expanded" in t.attrs);
  assert.ok(toggle, "缺少带 aria-expanded 的移动端导航开关");
  assert.equal(toggle.attrs["aria-expanded"], "false", "导航开关初始 aria-expanded 应为 false");
  assert.ok(toggle.attrs["aria-controls"], "导航开关缺少 aria-controls");
  assert.ok(ids.has(toggle.attrs["aria-controls"]), "aria-controls 指向的导航 id 不存在");

  assert.match(html, /aria-live="(polite|assertive)"/, "缺少 aria-live 状态播报区域");
});

test("链接均有可读文本且 href 有效", () => {
  const links = tags.filter((t) => t.name === "a");
  assert.ok(links.length >= 6, `链接过少（${links.length}）`);
  for (const link of links) {
    const href = (link.attrs.href || "").trim();
    assert.ok(href !== "", `第 ${link.line} 行的 <a> 缺少 href`);
    assert.ok(!/^javascript:/i.test(href), `第 ${link.line} 行的 <a> 使用 javascript: 伪协议`);
    assert.ok(href !== "#", `第 ${link.line} 行的 <a> 使用空锚点 "#"`);
    const innerText = visibleText(innerOf(html, "a", link.index));
    assert.ok(innerText || link.attrs["aria-label"], `第 ${link.line} 行的 <a> 无可访问文本（图标链接需 aria-label）`);
  }
});

/* ------------------------------------------------------------ 6. 锚点闭合 */

test("页内锚点全部闭合（无断链）", () => {
  const anchors = Array.from(
    new Set(
      tags
        .filter((t) => t.name === "a" && /^#.+/.test(t.attrs.href || ""))
        .map((t) => (t.attrs.href || "").slice(1))
    )
  );
  assert.ok(anchors.length >= 4, "页内导航锚点过少");
  const missing = anchors.filter((frag) => !ids.has(frag));
  assert.deepEqual(missing, [], `以下锚点没有对应 id：${missing.map((f) => "#" + f).join(", ")}`);
});

/* -------------------------------------------------------- 7. 对比度 / 主题 */

test("可读性 token：明暗两套主题都定义了正文与背景色，对比度达 AAA", () => {
  const tokensOf = (block) => new Set(Array.from(block.matchAll(/(--[a-zA-Z0-9-]+)\s*:/g), (m) => m[1]));
  const lightIndex = cssCode.search(/@media\s*\(\s*prefers-color-scheme:\s*light\s*\)/);
  assert.ok(lightIndex >= 0, "缺少 prefers-color-scheme: light 浅色回退");

  const darkBlock = cssCode.slice(0, lightIndex);
  const lightBlock = cssCode.slice(lightIndex);

  for (const key of ["--color-bg", "--color-text", "--color-text-muted", "--color-accent"]) {
    assert.ok(tokensOf(darkBlock).has(key), `深色主题缺少 ${key}`);
  }
  for (const key of ["--color-bg", "--color-text", "--color-accent"]) {
    assert.ok(tokensOf(lightBlock).has(key), `浅色主题缺少 ${key}`);
  }

  const darkText = darkBlock.match(/--color-text:\s*(#[0-9a-fA-F]{3,8})/);
  const darkBg = darkBlock.match(/--color-bg:\s*(#[0-9a-fA-F]{3,8})/);
  assert.ok(darkText && darkBg, "深色主题正文色与背景色应为十六进制色值");
  const darkContrast = contrastRatio(darkText[1], darkBg[1]);
  assert.ok(darkContrast >= 7, `深色主题正文对比度仅 ${darkContrast.toFixed(2)}:1，低于 AAA 7:1`);

  const lightText = lightBlock.match(/--color-text:\s*(#[0-9a-fA-F]{3,8})/);
  const lightBg = lightBlock.match(/--color-bg:\s*(#[0-9a-fA-F]{3,8})/);
  assert.ok(lightText && lightBg, "浅色主题正文色与背景色应为十六进制色值");
  const lightContrast = contrastRatio(lightText[1], lightBg[1]);
  assert.ok(lightContrast >= 7, `浅色主题正文对比度仅 ${lightContrast.toFixed(2)}:1，低于 AAA 7:1`);
});

/* --------------------------------------------------------------- 8. 内联脚本 */

test("页面不依赖外部脚本，且内联脚本可解析", () => {
  const scripts = tags.filter((t) => t.name === "script");
  assert.deepEqual(scripts.filter((t) => t.attrs.src).map((t) => t.raw), [], "存在外部 <script src>，离线打开会失效");
  for (const script of scripts) {
    if (script.attrs.type && !/javascript|module/i.test(script.attrs.type)) continue;
    const body = innerOf(html, "script", script.index);
    assert.doesNotThrow(() => new vm.Script(body), `第 ${script.line} 行的内联脚本存在语法错误`);
  }
  assert.ok(!/document\.write\s*\(/.test(html), "使用了 document.write，破坏解析与可访问性");
});
