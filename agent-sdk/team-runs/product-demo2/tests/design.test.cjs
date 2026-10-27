/**
 * OwO Agent 静态官网 —— index.html / styles.css 设计验收用例（tester）
 *
 * 运行：node --test tests/design.test.cjs
 * 依赖：仅 Node 内置模块 fs / path / assert，无第三方依赖、不联网。
 *
 * 用例映射（对应 docs/product-brief.md §5 与 designer 交付说明）：
 *   AC1  文件存在且非空（含最小体量，防止空壳交付）
 *   AC2  文档骨架（DOCTYPE / lang / viewport / title / charset / description）
 *   AC3  结构与语义（恰好一个 h1、header/main/footer、双 CTA）
 *   AC4  层级完整（overview / capabilities / workflow / docs 锚点 + 导航链接 + 全量锚点落点）
 *   AC5  样式外链（link rel=stylesheet、无内联 <style>、无内联 style 属性）
 *   AC6  无外部资源（CSS/HTML 无 http(s) 外链、无 @import / @font-face / CDN、无 script、有系统字体栈）
 *   AC7  响应式（断点 640 / 768 / 1024，导航折叠、能力 3 列、流程 4 列、44px 触控目标）
 *   AC8  可访问性基线（focus-visible、跳过链接、prefers-reduced-motion、img alt、SVG role、caption）
 *   AC9  内容结构（能力卡片 ≥ 6 且带图标位、流程 4 步、设计 token / 字号 / 可见文本体量）
 *   AC10 无占位残留（lorem ipsum / TODO / FIXME / 中文占位 / 空链接）
 *   AC11 离线可打开（styles.css 真实存在、相对路径、无运行时请求、无表单）
 */

'use strict';

const { test, describe, before } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const HTML_PATH = path.join(ROOT, 'index.html');
const CSS_PATH = path.join(ROOT, 'styles.css');

/** 读取单个被测文件；失败时返回 { missing: true, reason }，让断言给出可读失败信息 */
function loadFile(absPath) {
  try {
    const stat = fs.statSync(absPath);
    if (!stat.isFile()) return { missing: true, reason: 'not-a-file' };
    const raw = fs.readFileSync(absPath);
    return { raw, size: stat.size, bytes: raw.length, text: raw.toString('utf8') };
  } catch (err) {
    return { missing: true, reason: err.code || 'ENOENT' };
  }
}

/** 去除 HTML 注释，避免注释内容干扰结构断言 */
function stripHtmlComments(html) {
  return html.replace(/<!--[\s\S]*?-->/g, '');
}

/** 去除 CSS 注释，避免注释内容干扰样式断言 */
function stripCssComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

/** 开标签计数（本页无同名嵌套标签，足够可靠） */
function countTags(html, tag) {
  const re = new RegExp(`<${tag}(\\s|>|/)`, 'gi');
  const matches = html.match(re);
  return matches ? matches.length : 0;
}

let htmlFile;
let cssFile;
let html;
let css;

before(() => {
  htmlFile = loadFile(HTML_PATH);
  cssFile = loadFile(CSS_PATH);
  html = htmlFile.missing ? '' : stripHtmlComments(htmlFile.text);
  css = cssFile.missing ? '' : stripCssComments(cssFile.text);
});

describe('OwO Agent 官网设计验收', () => {
  test('AC1 · 文件存在且非空（含 UTF-8 可解码）', () => {
    assert.equal(htmlFile.missing, undefined, `index.html 缺失（${htmlFile.reason || 'unknown'}）`);
    assert.equal(cssFile.missing, undefined, `styles.css 缺失（${cssFile.reason || 'unknown'}）`);
    assert.ok(htmlFile.bytes > 0, 'index.html 字节数为 0');
    assert.ok(cssFile.bytes > 0, 'styles.css 字节数为 0');
    assert.ok(htmlFile.bytes > 3000, `index.html 过小（${htmlFile.bytes} 字节），疑似占位空壳`);
    assert.ok(cssFile.bytes > 2000, `styles.css 过小（${cssFile.bytes} 字节），疑似占位空壳`);
    assert.ok(!html.includes('\uFFFD'), 'index.html 含替换字符，疑似编码错误');
    assert.ok(!css.includes('\uFFFD'), 'styles.css 含替换字符，疑似编码错误');
  });

  test('AC2 · 文档骨架：DOCTYPE / lang / viewport / title / charset', () => {
    assert.match(html.trimStart(), /^<!DOCTYPE html>/i, '缺少 <!DOCTYPE html>');
    const langMatch = html.match(/<html[^>]*\slang="([^"]+)"/i);
    assert.ok(langMatch, '<html> 缺少 lang 属性');
    assert.match(langMatch[1], /^(zh-CN|zh|en|en-US)$/i, `<html lang="${langMatch[1]}"> 不在允许集合内`);
    assert.match(html, /<head[\s>]/i, '缺少 <head>');
    assert.match(html, /<meta\s+name="viewport"\s+content="[^"]*width=device-width[^"]*"/i, '缺少移动端 viewport meta');
    const title = html.match(/<title>([\s\S]*?)<\/title>/i);
    assert.ok(title, '缺少 <title>');
    assert.ok(title[1].trim().length >= 4, '<title> 文本过短');
    assert.match(html, /<meta\s+charset="utf-8"/i, '缺少 <meta charset="utf-8">');
    assert.match(html, /<meta\s+name="description"\s+content="[^"]{20,}"/i, '缺少有效的 meta description');
  });

  test('AC3 · 结构与语义：唯一 h1 / header-main-footer / 双 CTA', () => {
    assert.equal(countTags(html, 'h1'), 1, `h1 数量应为 1，实际 ${countTags(html, 'h1')}`);
    for (const tag of ['header', 'main', 'footer', 'nav', 'section']) {
      assert.ok(countTags(html, tag) >= 1, `缺少 <${tag}> 语义标签`);
    }
    assert.equal(countTags(html, 'main'), 1, 'main 应唯一');
    assert.match(html, /<main[^>]*\sid="main"/i, '<main> 应带 id="main" 以承接跳过链接');

    const textOnly = html.replace(/<[^>]+>/g, ' ');
    const getStarted = textOnly.match(/Get Started/gi) || [];
    assert.ok(getStarted.length >= 2, `主 CTA「Get Started」应至少出现两次（顶栏 + Hero），实际 ${getStarted.length}`);
    assert.match(textOnly, /(文档|Docs)/i, '缺少次 CTA「文档 / Docs」文本');
    assert.match(textOnly, /浏览示例工作流/, '缺少次 CTA「浏览示例工作流」');

    assert.ok(countTags(html, 'h2') >= 3, `h2 数量不足（${countTags(html, 'h2')}）`);
    assert.ok(countTags(html, 'h3') >= 6, `h3 数量不足（${countTags(html, 'h3')}）`);
    assert.equal(countTags(html, 'h4'), 0, '出现 h4，疑似标题层级跳级');
  });

  test('AC4 · 层级完整：四个 section 锚点与导航链接一致', () => {
    const anchors = ['overview', 'capabilities', 'workflow', 'docs'];
    for (const id of anchors) {
      assert.match(html, new RegExp(`id="${id}"`), `缺少锚点 id="${id}"`);
      assert.match(html, new RegExp(`href="#${id}"`), `缺少指向 #${id} 的链接`);
    }

    const navMatch = html.match(/<nav[\s\S]*?<\/nav>/i);
    assert.ok(navMatch, '缺少主导航 <nav>');
    for (const id of anchors) {
      assert.ok(navMatch[0].includes(`href="#${id}"`), `主导航缺少 href="#${id}"`);
    }

    const labelled = html.match(/aria-labelledby="([^"]+)"/g) || [];
    assert.ok(labelled.length >= 4, `aria-labelledby 少于 4 处（实际 ${labelled.length}）`);
    for (const attr of labelled) {
      const targetId = attr.match(/aria-labelledby="([^"]+)"/)[1];
      assert.ok(html.includes(`id="${targetId}"`), `aria-labelledby 指向的 id="${targetId}" 不存在`);
    }

    const hrefs = [...html.matchAll(/href="#([^"]+)"/g)].map((m) => m[1]);
    assert.ok(hrefs.length >= 8, `页内锚点过少（${hrefs.length}）`);
    for (const href of new Set(hrefs)) {
      assert.ok(html.includes(`id="${href}"`), `页内链接 #${href} 无对应落点`);
    }
  });

  test('AC5 · 样式外链：link stylesheet 且无内联大段样式', () => {
    assert.match(html, /<link\s+rel="stylesheet"\s+href="styles\.css"/i, '缺少 <link rel="stylesheet" href="styles.css">');
    const styleBlocks = [...html.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/gi)];
    assert.equal(styleBlocks.length, 0, 'index.html 存在内联 <style> 块，应改为外链 styles.css');
    const inlineStyleAttrs = [...html.matchAll(/\sstyle="[^"]*"/gi)];
    assert.equal(inlineStyleAttrs.length, 0, `存在 ${inlineStyleAttrs.length} 处内联 style 属性`);
    assert.ok(css.length > 1000, 'styles.css 去除注释后有效样式过短');
  });

  test('AC6 · 无外部资源 / 无脚本 / 无网络字体', () => {
    const hitPatterns = [
      [/https?:\/\//gi, 'http(s) 外链'],
      [/\burl\(\s*['"]?(?!\s*#)[^)'"]*\.(?:png|jpe?g|gif|webp|svg|woff2?|ttf|eot)/gi, '外部 URL 资源引用'],
      [/@import/gi, '@import 指令'],
      [/fonts\.googleapis|fonts\.gstatic|cdn\.|unpkg|jsdelivr/gi, 'CDN / 网络字体'],
      [/<\/?(?:iframe|video|audio|embed|object)\b/gi, '外部嵌入元素'],
      [/<script\b/gi, '脚本标签'],
    ];
    const hits = [];
    for (const [label, text] of [['index.html', html], ['styles.css', css]]) {
      for (const [re, name] of hitPatterns) {
        const m = text.match(re);
        if (m) hits.push(`${label}: ${name} × ${m.length}`);
      }
    }
    assert.deepEqual(hits, [], `检测到外部依赖或脚本：\n${hits.join('\n')}`);

    assert.doesNotMatch(css, /@font-face/i, 'styles.css 不应包含 @font-face');
    assert.match(css, /-apple-system/i, '缺少系统字体栈（-apple-system）');
    assert.match(css, /ui-monospace|SFMono-Regular|Menlo/i, '缺少等宽字体栈');
  });

  test('AC7 · 响应式：断点 640 / 768 / 1024 与导航折叠、网格列变化', () => {
    const mediaQueries = css.match(/@media[^{]+\{/g) || [];
    assert.ok(mediaQueries.length >= 1, 'styles.css 未包含任何 @media 查询');
    assert.match(css, /@media\s*\(min-width:\s*640px\)/i, '缺少 640px 断点（小屏增强）');
    assert.match(css, /@media\s*\(min-width:\s*768px\)/i, '缺少 768px 断点（导航切换）');
    assert.match(css, /@media\s*\(min-width:\s*1024px\)/i, '缺少 1024px 断点（桌面三列）');

    assert.match(css, /\.site-nav\s*\{[^}]*display:\s*none/i, '移动端 .site-nav 应默认隐藏');
    assert.match(css, /\.mobile-menu\s*\{[^}]*display:\s*none/i, '桌面端 .mobile-menu 应隐藏');
    assert.match(css, /\.card-grid[\s\S]*?repeat\(3,\s*minmax/i, '能力网格桌面应 3 列');
    assert.match(css, /\.steps[\s\S]*?repeat\(4,\s*minmax/i, '工作流步骤桌面应 4 列');
    assert.match(css, /min-height:\s*44px/i, '主按钮缺少 44px 触控目标高度');
    assert.match(css, /max-width:\s*var\(--wrap\)/i, '缺少内容最大宽度约束');
  });

  test('AC8 · 可访问性基线：focus-visible / 跳过链接 / reduced-motion / img alt', () => {
    assert.match(css, /:focus-visible/i, 'styles.css 缺少 :focus-visible 规则');
    assert.match(css, /prefers-reduced-motion/i, 'styles.css 缺少 prefers-reduced-motion 媒体查询');
    assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/i, '缺少 prefers-reduced-motion: reduce 兜底');

    assert.match(html, /<a[^>]*class="skip-link"[^>]*href="#main"/i, '缺少指向 #main 的跳过链接');
    assert.match(css, /\.skip-link:focus/i, '跳过链接缺少 :focus 可见态');

    for (const img of html.match(/<img\b[^>]*>/gi) || []) {
      assert.match(img, /\salt="[^"]*"/i, `图片缺少 alt：${img}`);
    }
    const svgs = html.match(/<svg\b[^>]*>/gi) || [];
    assert.ok(svgs.length >= 1, '缺少内联 SVG 示意图');
    for (const svg of svgs) {
      assert.match(svg, /(role="img"|aria-hidden="true")/i, `SVG 缺少 role="img" 或 aria-hidden：${svg}`);
    }
    assert.match(html, /aria-label="[^"]+"/i, '缺少 aria-label 描述性标签');
    assert.match(html, /<caption/i, '数据表格缺少 <caption>');
    assert.match(html, /class="brand"[^>]*aria-label="[^"]+"/i, '品牌链接缺少 aria-label');
  });

  test('AC9 · 内容结构：能力卡片 ≥ 6、工作流 4 步、设计 token', () => {
    const cardCount = (html.match(/class="card"/g) || []).length;
    assert.ok(cardCount >= 6, `能力卡片数量 ${cardCount} < 6`);
    const stepCount = (html.match(/class="step"/g) || []).length;
    assert.equal(stepCount, 4, `工作流步骤应为 4，实际 ${stepCount}`);
    const cardIcons = (html.match(/class="card-icon"/g) || []).length;
    assert.equal(cardIcons, cardCount, '每张能力卡片都应有图标位 .card-icon');

    assert.match(css, /:root\s*\{/, '缺少 :root 设计 token 定义');
    for (const token of ['--s1:', '--s2:', '--s3:', '--s4:', '--accent:', '--radius']) {
      assert.ok(css.includes(token), `缺少设计 token ${token}`);
    }
    assert.match(css, /--s1:\s*8px/, '--s1 应为 8px 基准');
    assert.match(css, /line-height:\s*1\.6/, '正文行高应为 1.6');
    assert.match(css, /h1\s*\{[^}]*font-size:\s*clamp\(/i, 'h1 应使用 clamp() 响应式字号');

    const bodyText = html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();
    assert.ok(bodyText.length > 600, `页面可见文本过少（${bodyText.length} 字符），疑似占位内容`);
    assert.match(bodyText, /OwO Agent/, '可见文本缺少产品名 OwO Agent');
  });

  test('AC10 · 无占位残留（lorem ipsum / TODO / FIXME / placeholder）', () => {
    const placeholderPatterns = [
      [/lorem ipsum/gi, 'lorem ipsum'],
      [/\bTODO\b/g, 'TODO'],
      [/\bFIXME\b/g, 'FIXME'],
      [/\bXXX\b/g, 'XXX'],
      [/\{placeholder\}/gi, '{placeholder}'],
      [/待补充|待填写|此处填写|占位文本/g, '中文占位标记'],
      [/href="#"(\s|>)/gi, 'href="#" 空链接'],
      [/src=""/gi, '空 src'],
    ];
    for (const [re, name] of placeholderPatterns) {
      for (const [label, text] of [['index.html', html], ['styles.css', css]]) {
        assert.doesNotMatch(text, re, `${label} 中存在占位/未完成标记：${name}`);
      }
    }
  });

  test('AC11 · 离线可打开：样式真实存在、无运行时依赖、编码一致', () => {
    assert.ok(fs.existsSync(path.join(ROOT, 'styles.css')), 'styles.css 不存在，离线打开将丢失样式');
    const hrefMatch = html.match(/<link\s+rel="stylesheet"\s+href="([^"]+)"/i);
    assert.ok(hrefMatch, '缺少样式引用');
    assert.equal(hrefMatch[1], 'styles.css', `样式路径应为本目录 styles.css，实际 ${hrefMatch[1]}`);
    assert.ok(!path.isAbsolute(hrefMatch[1]), '样式引用不应为绝对路径');
    assert.doesNotMatch(html, /fetch\(|XMLHttpRequest|import\(/i, 'index.html 存在运行时网络请求代码');
    assert.match(html, /<meta\s+charset="utf-8"/i, 'HTML 编码声明缺失');
    assert.doesNotMatch(html, /<form\b|<input\b/i, '静态介绍页不应包含表单控件');
  });
});
