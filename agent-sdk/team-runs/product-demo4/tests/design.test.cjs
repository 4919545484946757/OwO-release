/**
 * OwO Agent SDK 落地页 — 设计与结构回归测试
 *
 * 运行：node --test tests/design.test.cjs
 * 依赖：仅 Node 内置 node:test / node:assert / node:fs，无第三方包、无网络。
 * 范围：静态校验 index.html 与 styles.css（不启动浏览器，不做真实渲染测量）。
 */
"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const HTML_PATH = path.join(ROOT, "index.html");
const CSS_PATH = path.join(ROOT, "styles.css");

const readUtf8 = (file) => fs.readFileSync(file, "utf8");

const html = fs.existsSync(HTML_PATH) ? readUtf8(HTML_PATH) : null;
const css = fs.existsSync(CSS_PATH) ? readUtf8(CSS_PATH) : null;

/** 去掉 HTML 注释，避免注释内容干扰断言 */
const stripHtmlComments = (src) => src.replace(/<!--[\s\S]*?-->/g, "");
const visibleHtml = html === null ? null : stripHtmlComments(html);

/** 去掉 CSS 注释 */
const stripCssComments = (src) => src.replace(/\/\*[\s\S]*?\*\//g, "");
const bareCss = css === null ? null : stripCssComments(css);

/** 去掉代码块 / 预格式化块内容（代码里的 #fff、<T> 之类不应参与样式与标签断言） */
const stripCodeBlocks = (src) =>
  src
    .replace(/<pre[\s\S]*?<\/pre>/gi, "<pre></pre>")
    .replace(/<code[\s\S]*?<\/code>/gi, "<code></code>");

const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// ---------------------------------------------------------------------------
// 0. 文件存在与非空
// ---------------------------------------------------------------------------

test("产物文件存在且非空：index.html 与 styles.css", () => {
  assert.ok(html !== null, "index.html 必须存在");
  assert.ok(css !== null, "styles.css 必须存在");
  assert.ok(html.trim().length > 1000, "index.html 内容不应为空或占位");
  assert.ok(css.trim().length > 1000, "styles.css 内容不应为空或占位");
});

// ---------------------------------------------------------------------------
// 1. HTML 文档基础
// ---------------------------------------------------------------------------

test("HTML 声明 zh-CN 语言且字符集/视口正确", () => {
  assert.match(html, /^<!DOCTYPE html>/i, "应以 <!DOCTYPE html> 开头");
  assert.match(html, /<html[^>]*\blang\s*=\s*"zh-CN"/i, "根元素需 lang=\"zh-CN\"");
  assert.match(html, /<meta[^>]*charset\s*=\s*"utf-8"/i, "需声明 UTF-8 字符集");
  assert.match(
    html,
    /<meta[^>]*name\s*=\s*"viewport"[^>]*content\s*=\s*"[^"]*width=device-width/i,
    "需声明响应式 viewport",
  );
});

test("HTML 通过相对路径 styles.css 引入样式", () => {
  assert.match(
    html,
    /<link[^>]*rel\s*=\s*"stylesheet"[^>]*href\s*=\s*"styles\.css"/i,
    "需以 <link rel=\"stylesheet\" href=\"styles.css\"> 引入样式",
  );
});

test("HTML 区块顺序与结构完整（header → main → footer）", () => {
  const headerIdx = html.indexOf("<header");
  const mainIdx = html.indexOf("<main");
  const footerIdx = html.indexOf("<footer");
  assert.ok(headerIdx > -1, "缺少 <header>");
  assert.ok(mainIdx > -1, "缺少 <main>");
  assert.ok(footerIdx > -1, "缺少 <footer>");
  assert.ok(headerIdx < mainIdx && mainIdx < footerIdx, "结构顺序应为 header → main → footer");
  assert.ok(html.includes("</footer>"), "footer 未闭合");
  assert.ok(html.includes("</html>"), "文档未闭合");
});

test("导航、Hero、全部内容区块按简报顺序出现且锚点可跳转", () => {
  assert.match(
    html,
    /<nav[^>]*class\s*=\s*"nav"[^>]*aria-label\s*=\s*"主导航"/i,
    "需有带 aria-label 的主导航",
  );
  const expectedSections = [
    "top",
    "capabilities",
    "quickstart",
    "how-it-works",
    "use-cases",
    "faq",
  ];
  const positions = expectedSections.map((id) => {
    const idx = html.indexOf(`id="${id}"`);
    assert.ok(idx > -1, `缺少区块 id="${id}"`);
    return idx;
  });
  for (let i = 1; i < positions.length; i += 1) {
    assert.ok(
      positions[i - 1] < positions[i],
      `区块顺序错误：${expectedSections[i - 1]} 应在 ${expectedSections[i]} 之前`,
    );
  }
});

test("页内锚点链接（#xxx）均有对应目标", () => {
  const links = [...html.matchAll(/href\s*=\s*"(#[^"]*)"/gi)].map((m) => m[1]);
  assert.ok(links.length > 0, "应存在页内锚点链接");
  const ids = new Set([...html.matchAll(/\bid\s*=\s*"([^"]+)"/gi)].map((m) => m[1]));
  const missing = [...new Set(links)].filter((href) => !ids.has(href.slice(1)));
  assert.deepEqual(missing, [], `存在无目标的锚点链接：${missing.join(", ")}`);
});

// ---------------------------------------------------------------------------
// 2. 语义与可访问性
// ---------------------------------------------------------------------------

test("全页仅一个 <h1>，且包含产品名", () => {
  const h1s = [...html.matchAll(/<h1\b[^>]*>([\s\S]*?)<\/h1>/gi)].map((m) => m[1]);
  assert.equal(h1s.length, 1, `应有且仅有 1 个 <h1>，实际 ${h1s.length} 个`);
  assert.match(h1s[0], /OwO Agent SDK/, "h1 文案需包含 \"OwO Agent SDK\"");
});

test("标题层级不跳级（h1 → h2 → h3）", () => {
  const levels = [...html.matchAll(/<h([1-6])\b/gi)].map((m) => Number(m[1]));
  assert.ok(levels.length > 5, "标题数量过少，疑似结构缺失");
  assert.equal(levels[0], 1, "首个标题应为 h1");
  for (let i = 1; i < levels.length; i += 1) {
    assert.ok(
      levels[i] - levels[i - 1] <= 1,
      `标题层级跳级：h${levels[i - 1]} 之后出现 h${levels[i]}（第 ${i + 1} 个标题）`,
    );
  }
});

test("存在跳到正文的 skip-link，图片均有替代文本（本页无 <img>）", () => {
  assert.match(
    html,
    /<a[^>]*class\s*=\s*"skip-link"[^>]*href\s*=\s*"#main"/i,
    "需提供 skip-link 且指向 #main",
  );
  const imgs = [...html.matchAll(/<img\b[^>]*>/gi)].map((m) => m[0]);
  for (const img of imgs) {
    assert.match(img, /\balt\s*=\s*"[^"]*"/i, `<img> 缺少 alt：${img}`);
  }
});

test("代码块为可聚焦区域并带 aria-label（无键盘陷阱）", () => {
  const pres = [...html.matchAll(/<pre\b[^>]*>/gi)].map((m) => m[0]);
  assert.ok(pres.length >= 2, "至少应有 Hero 与快速开始两个代码块");
  for (const pre of pres) {
    assert.match(pre, /\btabindex\s*=\s*"0"/i, "代码块需 tabindex=\"0\" 以支持键盘横向滚动");
    assert.match(pre, /\brole\s*=\s*"region"/i, "代码块需 role=\"region\"");
    assert.match(pre, /\baria-label\s*=\s*"[^"]+"/i, "代码块需 aria-label");
  }
});

test("FAQ 使用原生 <details>/<summary>，且仅第一条默认展开", () => {
  const detailsCount = (html.match(/<details\b/gi) || []).length;
  const summaryCount = (html.match(/<summary\b/gi) || []).length;
  assert.ok(detailsCount >= 5, `FAQ 条目过少：${detailsCount}`);
  assert.equal(summaryCount, detailsCount, "每个 <details> 都应配一个 <summary>");
  const openCount = (html.match(/<details[^>]*\bopen\b/gi) || []).length;
  assert.equal(openCount, 1, `默认展开项应为 1 个，实际 ${openCount} 个`);
  const firstDetails = html.indexOf("<details");
  const firstOpen = html.indexOf("<details open");
  assert.equal(firstDetails, firstOpen, "默认展开的应是第一条 FAQ");
});

test("所有 <a> 都有可点击目标（href 非空），装饰元素标记 aria-hidden", () => {
  const anchors = [...html.matchAll(/<a\b[^>]*>/gi)].map((m) => m[0]);
  assert.ok(anchors.length >= 10, "链接数量异常，疑似内容缺失");
  for (const a of anchors) {
    const href = /\bhref\s*=\s*"([^"]*)"/i.exec(a);
    assert.ok(href, `链接缺少 href：${a}`);
    assert.ok(href[1].trim().length > 0, `链接 href 为空：${a}`);
    assert.ok(href[1] !== "#", `链接指向空锚点 #：${a}`);
  }
  const decorative = [...html.matchAll(/<span[^>]*class\s*=\s*"wordmark__mark"[^>]*>/gi)];
  assert.ok(decorative.length > 0, "缺少装饰性 wordmark 标记");
  for (const span of decorative) {
    assert.match(span[0], /aria-hidden\s*=\s*"true"/i, "装饰字符需 aria-hidden=\"true\"");
  }
});

// ---------------------------------------------------------------------------
// 3. 内容质量：无占位废话、无未经证实的指标、无外链风险
// ---------------------------------------------------------------------------

test("无 lorem ipsum 等占位废话文本", () => {
  const placeholders = [
    /lorem ipsum/i,
    /dolor sit amet/i,
    /consectetur adipiscing/i,
    /\bTODO\s*:\s*lorem/i,
    /待补充内容/,
    /这里写点什么/,
  ];
  for (const re of placeholders) {
    assert.doesNotMatch(visibleHtml, re, `页面正文出现占位废话：${re}`);
  }
});

test("示例数据均显式标注为示例/占位/TODO", () => {
  assert.match(html, /示例/, "页面应显式标注示例内容");
  assert.match(html, /占位/, "页面应显式标注占位内容");
  assert.match(html, /TODO/, "未定事项应标注 TODO");
  assert.match(html, /示例占位/, "安装包名应标注为示例占位");
});

test("无未经验证的数字指标与定价/SLA 承诺", () => {
  const risky = [
    /\d+\s*%\s*(?:更快|提升|加速)/,
    /\d+x\s*快/i,
    /\$\s?\d+/,
    /\d+\s*元\s*\/\s*月/,
    /\bSLA\b/i,
    /\b99\.9+\s*%/,
    /免费试用\s*\d+\s*天/,
  ];
  for (const re of risky) {
    assert.doesNotMatch(visibleHtml, re, `出现未证实指标/承诺：${re}`);
  }
});

test("页面自包含：无第三方 http(s) 资源、无外链字体/脚本", () => {
  const tags = [...html.matchAll(/<(script|link|img|iframe|source)\b[^>]*>/gi)].map((m) => m[0]);
  for (const tag of tags) {
    const urls = [...tag.matchAll(/(?:src|href)\s*=\s*"(https?:)?\/\/[^"]*"/gi)].map((m) => m[0]);
    assert.deepEqual(urls, [], `资源标签引用了外部地址：${tag} → ${urls.join(", ")}`);
  }
  const scripts = [...html.matchAll(/<script\b[^>]*>/gi)].map((m) => m[0]);
  for (const s of scripts) {
    assert.doesNotMatch(s, /\bsrc\s*=/i, "脚本必须内联或本地，不得外链");
  }
  // 页面上的 <a href="https://..."> 只允许指向已标注的占位仓库/邮件地址
  const externalLinks = [...html.matchAll(/<a\b[^>]*href\s*=\s*"(https?:\/\/[^"]+)"/gi)].map(
    (m) => m[1],
  );
  for (const url of externalLinks) {
    assert.match(url, /github\.com\/todo\/|example\.com/, `外链应为已标注的占位地址：${url}`);
  }
});

// ---------------------------------------------------------------------------
// 4. 样式：接入、变量集中、无内联样式、响应式与合流
// ---------------------------------------------------------------------------

test("HTML 无 <style> 块、无 style=\"...\" 内联样式（样式全部在 styles.css）", () => {
  assert.doesNotMatch(visibleHtml, /<style\b/i, "不应出现 <style> 块");
  assert.doesNotMatch(visibleHtml, /\sstyle\s*=\s*"/i, "不应出现 style=\"...\" 内联样式");
  assert.doesNotMatch(
    stripCodeBlocks(visibleHtml),
    /\s(?:width|height|bgcolor|color|align)\s*=\s*"/i,
    "不应使用表现型 HTML 属性",
  );
});

test("颜色/间距/字号/圆角集中声明在 :root 变量中", () => {
  assert.match(bareCss, /:root\s*\{[\s\S]*?--bg\s*:/, ":root 应定义颜色变量");
  const requiredVars = [
    "--bg",
    "--bg-raised",
    "--border",
    "--text",
    "--text-muted",
    "--accent",
    "--accent-ink",
    "--space-1",
    "--space-4",
    "--fs-base",
    "--fs-h1",
    "--radius",
    "--container",
    "--header-h",
    "--font-sans",
    "--font-mono",
  ];
  const rootBlock = /:root\s*\{([\s\S]*?)\}/.exec(bareCss);
  assert.ok(rootBlock, "缺少 :root 声明块");
  for (const v of requiredVars) {
    assert.ok(rootBlock[1].includes(v), `:root 缺少变量 ${v}`);
  }
  const varRefs = (bareCss.match(/var\(--[\w-]+\)/g) || []).length;
  assert.ok(varRefs >= 100, `变量复用过少（${varRefs} 处），疑似仍有硬编码`);
});

test("CSS 使用系统字体栈，不外链字体、不引入框架", () => {
  assert.match(css, /system-ui/, "字体栈应包含 system-ui");
  assert.doesNotMatch(css, /@import/, "不应 @import 外部样式");
  assert.doesNotMatch(css, /url\(\s*['"]?https?:/i, "不应引用远程资源");
  assert.doesNotMatch(css, /@font-face/i, "不应自建远程字体");
  assert.match(css, /ui-monospace|SFMono-Regular|Menlo|Consolas/, "等宽字体应走系统栈");
});

test("键盘焦点可见：:focus-visible 有描边，且未出现 outline: none", () => {
  assert.match(bareCss, /:focus-visible/, "需为交互元素提供 :focus-visible 样式");
  assert.match(
    bareCss,
    /outline\s*:\s*2px\s+solid\s+var\(--accent\)/,
    "焦点描边应为 2px solid var(--accent)",
  );
  assert.doesNotMatch(bareCss, /outline\s*:\s*(?:none|0(?:\s|;))/i, "不得使用 outline: none 抹掉焦点");
});

test("响应式断点齐全：640 / 1024 / 1280 与移动优先基准", () => {
  for (const bp of ["640px", "1024px", "1280px"]) {
    assert.ok(css.includes(`min-width: ${bp}`), `缺少 ${bp} 断点`);
  }
  assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/, "需尊重减少动效偏好");
  assert.match(css, /scroll-padding-top/, "sticky 导航需 scroll-padding-top 避免遮挡锚点");
  assert.match(css, /overflow-x:\s*auto/, "代码块需自身横向滚动，避免撑破视口");
  assert.match(css, /minmax\(0,\s*1fr\)/, "网格列需 minmax(0, 1fr) 防止子项溢出");
  assert.match(css, /box-sizing:\s*border-box/, "应统一 box-sizing");
});

test("CSS 大括号配平（无截断/未闭合规则）", () => {
  const open = (bareCss.match(/\{/g) || []).length;
  const close = (bareCss.match(/\}/g) || []).length;
  assert.equal(open, close, `CSS 大括号不配平：{ ${open} 个，} ${close} 个`);
  assert.ok(open > 50, "CSS 规则数量异常，疑似文件被截断");
});

// ---------------------------------------------------------------------------
// 5. 渐进增强脚本：内联、无依赖、可降级
// ---------------------------------------------------------------------------

test("内联脚本为渐进增强：无外链依赖，缺失时内容仍可读", () => {
  const inline = (html.match(/<script>([\s\S]*?)<\/script>/gi) || []).join("\n");
  assert.ok(inline.length > 0, "应有内联渐进增强脚本");
  assert.match(inline, /navigator\.clipboard/, "复制按钮依赖 clipboard，且需自行判断可用性");
  assert.match(
    inline,
    /if\s*\(!pre\s*\|\|\s*!navigator\.clipboard\)\s*return/,
    "clipboard 不可用时应直接跳过渲染按钮",
  );
  assert.match(inline, /prefers-reduced-motion:\s*reduce/, "脚本需尊重减少动效偏好");
  assert.doesNotMatch(inline, /document\.write|eval\(/, "不应使用 document.write / eval");
  assert.doesNotMatch(inline, /https?:\/\//, "内联脚本不应发起网络请求");
});

// ---------------------------------------------------------------------------
// 6. 交叉一致性：HTML class 与 CSS 定义互相对应
// ---------------------------------------------------------------------------

test("HTML 中使用的结构性 class 均在 styles.css 中有定义", () => {
  const htmlClasses = new Set();
  for (const m of html.matchAll(/\bclass\s*=\s*"([^"]+)"/gi)) {
    for (const c of m[1].split(/\s+/)) {
      if (c) htmlClasses.add(c);
    }
  }
  assert.ok(htmlClasses.size > 20, "HTML class 数量异常，疑似结构缺失");

  // 白名单：经核实仅作语义挂钩、由继承样式决定外观（父级 .wordmark 已定义字号/字重/颜色），
  // 无独立 CSS 规则属预期。若未来为其补独立样式，应从此白名单移除。
  // 注意：`.copy-button` 由内联 JS 动态创建，不在此列。
  const STYLE_HOOK_ONLY = new Set(["wordmark__text"]);

  const missing = [...htmlClasses].filter((c) => {
    if (STYLE_HOOK_ONLY.has(c)) return false;
    const re = new RegExp(`\\.${escapeRegExp(c)}(?![\\w-])`);
    return !re.test(bareCss);
  });
  assert.deepEqual(missing, [], `以下 class 在 CSS 中无任何定义：${missing.join(", ")}`);
});

test("代码高亮 token class 与 CSS 一一对应", () => {
  const tokens = new Set([...html.matchAll(/class\s*=\s*"(tok-[\w-]+)"/gi)].map((m) => m[1]));
  assert.ok(tokens.size >= 4, "代码高亮 token 类型过少");
  for (const t of tokens) {
    assert.match(bareCss, new RegExp(`\\.${escapeRegExp(t)}(?![\\w-])`), `缺少 token 样式 .${t}`);
  }
});

test("样式表中引用的 CSS 变量均已在 :root 声明", () => {
  const rootBlock = /:root\s*\{([\s\S]*?)\}/.exec(bareCss);
  assert.ok(rootBlock, "缺少 :root 声明块");
  const declared = new Set([...rootBlock[1].matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
  const used = new Set([...bareCss.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]));
  const undeclared = [...used].filter((v) => !declared.has(v));
  assert.deepEqual(undeclared, [], `以下变量未在 :root 声明：${undeclared.join(", ")}`);
});
