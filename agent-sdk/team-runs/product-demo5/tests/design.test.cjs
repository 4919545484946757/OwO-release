"use strict";

/**
 * OwO Agent SDK landing page — structural / accessibility / responsive checks.
 *
 * Runner: node:test (built in), assertions: node:assert/strict.
 * No network, no DOM parser dependency, no npm install required:
 *   node --test tests/design.test.cjs
 *
 * The tests read the two shipped assets (index.html, styles.css) as text so the
 * page keeps rendering offline with zero build step.
 *
 * Requirement mapping (docs/product-brief.md §7). Every test name carries the
 * A-numbers it covers; the map below is asserted by its own test so the
 * A1-A3 / A5 / A7-A9 / A11 / A14 / A21-A25 coverage (A19) can never silently
 * regress:
 *   A1  document head ................ document: declares utf-8 charset, a language…
 *   A2  heading hierarchy ............ structure: every section is labelled…
 *   A3  sections + labels ............ structure: every section is labelled…
 *   A4  skip link .................... structure: skip link is the first focusable…
 *   A5  in-page anchors resolve ...... structure: in-page anchors resolve, without
 *                                      empty or dead anchors
 *   A6  primary + secondary CTA ...... content: primary and secondary CTAs exist…
 *   A7  quickstart ol/3 steps/code ... content: quickstart has an ordered 3-step list…
 *   A8  FAQ 3-5 Q&A .................. content: FAQ exposes 3-5 native Q&A items
 *   A9  relative stylesheet .......... assets: stylesheet is linked…
 *   A10 responsive breakpoints ....... css: responsive layout is mobile-first…
 *   A11 focus-visible ................ css: focus ring and pointer-target affordances
 *   A12 reduced motion ............... css: reduced motion is respected
 *   A13 no external resources ........ assets: no remote assets, scripts…
 *   A14 decorative visuals ........... media: images and decorative visuals…
 *   A15 no forms ..................... forms: page ships no submittable form…
 *   A16 lang + title ................. document: declares utf-8 charset, a language…
 *   A17 contrast ledger .............. css: contrast ledger records AA-compliant text
 *   A18 runner = node:test ........... assets: suite runs on node:test…
 *   A19 acceptance coverage map ...... coverage: A1-A17 / A21-A25 are mapped to
 *                                      executable tests
 *   A20 no runtime dependencies ...... assets: no remote assets…
 *   A21-A25 placeholder docs slot .... content: placeholder docs slot (A21-A25)…
 *
 * Optional enhancements: dark mode is recommended-but-optional (brief §5) and
 * the print sheet is not required at all. Both are asserted *only if present*
 * (and any optional block present must still be internally valid), so trimming
 * either to stay inside the brief keeps the suite green — see issue
 * 7e572da4 (was: hard requirement on @media print / prefers-color-scheme).
 */

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const HTML_PATH = path.join(ROOT, "index.html");
const CSS_PATH = path.join(ROOT, "styles.css");

const readHtml = () => fs.readFileSync(HTML_PATH, "utf8");
const readCss = () => fs.readFileSync(CSS_PATH, "utf8");

/* ------------------------------------------------------------------ helpers */

const stripCssComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");

/** Parse `@media (...)` blocks into { query, body } pairs (one nesting level). */
function mediaBlocks(css) {
  const source = stripCssComments(css);
  const blocks = [];
  const re = /@media\s*([^{]+)\{/g;
  let match;
  while ((match = re.exec(source)) !== null) {
    let depth = 1;
    let i = re.lastIndex;
    while (i < source.length && depth > 0) {
      const ch = source[i];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      i += 1;
    }
    blocks.push({ query: match[1].trim(), body: source.slice(re.lastIndex, i - 1) });
    re.lastIndex = i;
  }
  return blocks;
}

/**
 * Declarations of every rule whose selector list contains `selector` verbatim,
 * ignoring rules nested inside a @media whose prelude matches `skipMedia`
 * (default: print — the print sheet may legitimately hide the skip link).
 */
function ruleBodies(css, selector, skipMedia = /print/) {
  const src = stripCssComments(css);
  const shielded = mediaBlocks(src)
    .filter((b) => skipMedia.test(b.query))
    .map((b) => b.body)
    .filter(Boolean);

  const bodies = [];
  const re = /([^{}@]+)\{([^{}]*)\}/g;
  let match;
  while ((match = re.exec(src)) !== null) {
    const selectors = match[1]
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    if (!selectors.includes(selector)) continue;
    const body = match[2];
    if (shielded.some((inner) => inner.includes(body))) continue;
    bodies.push(body);
  }
  return bodies;
}

/** All class names used in an HTML document, as a Set (without the dot). */
function htmlClasses(html) {
  const set = new Set();
  const re = /\bclass\s*=\s*"([^"]*)"/g;
  let match;
  while ((match = re.exec(html)) !== null) {
    match[1]
      .split(/\s+/)
      .filter(Boolean)
      .forEach((name) => set.add(name));
  }
  return set;
}

/** All class selectors defined in a CSS chunk, as a Set (without the dot). */
function cssClasses(css) {
  const set = new Set();
  const re = /\.(-?[_a-zA-Z][\w-]*)/g;
  let match;
  while ((match = re.exec(stripCssComments(css))) !== null) {
    set.add(match[1]);
  }
  return set;
}

const count = (haystack, needle) => haystack.split(needle).length - 1;

/** Markup -> readable text (tags and comments removed, whitespace collapsed). */
const textOf = (fragment) =>
  fragment
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

/** All ids in the document, in source order. */
function idsIn(html) {
  const ids = [];
  const re = /\sid="([^"]+)"/g;
  let match;
  while ((match = re.exec(html)) !== null) ids.push(match[1]);
  return ids;
}

/** All anchors with their attributes, text and source offset. */
function anchorsIn(html) {
  const anchors = [];
  const re = /<a\b([^>]*)>([\s\S]*?)<\/a>/gi;
  let match;
  while ((match = re.exec(html)) !== null) {
    const attrs = match[1];
    const hrefMatch = attrs.match(/\shref="([^"]*)"/i);
    const classMatch = attrs.match(/\sclass="([^"]*)"/i);
    anchors.push({
      attrs,
      href: hrefMatch ? hrefMatch[1] : null,
      classes: classMatch ? classMatch[1].split(/\s+/).filter(Boolean) : [],
      text: textOf(match[2]),
      index: match.index,
      raw: match[0],
    });
  }
  return anchors;
}

/**
 * ids of the elements that enclose `offset` (a coarse DOM ancestor chain built
 * from the opening/closing tags that appear before it).
 */
function ancestorIdsAt(html, offset) {
  const stack = [];
  const re = /<(\/?)([a-zA-Z][\w-]*)\b([^>]*)>/g;
  let match;
  while ((match = re.exec(html)) !== null) {
    if (match.index >= offset) break;
    const closing = match[1] === "/";
    const name = match[2].toLowerCase();
    const attrs = match[3];
    if (closing) {
      const at = stack.map((t) => t.name).lastIndexOf(name);
      if (at > -1) stack.length = at;
      continue;
    }
    if (/\/\s*$/.test(attrs)) continue;
    if (["br", "img", "input", "hr", "meta", "link", "source"].includes(name)) continue;
    const idMatch = attrs.match(/\sid="([^"]+)"/i);
    stack.push({ name, id: idMatch ? idMatch[1] : null });
  }
  return stack.filter((t) => t.id).map((t) => t.id);
}

/**
 * Outer markup of every element whose opening tag carries `attrName="attrValue"`,
 * matched with a same-tag closing tag, in source order.
 */
function elementsWith(html, attrName, attrValue) {
  const found = [];
  const openRe = new RegExp(
    `<([a-zA-Z][\\w-]*)\\b[^>]*\\s${attrName}="${attrValue}"[^>]*>`,
    "gi"
  );
  let open;
  while ((open = openRe.exec(html)) !== null) {
    const tagName = open[1].toLowerCase();
    const start = open.index;
    const innerStart = start + open[0].length;
    const closeRe = new RegExp(`</${tagName}>`, "gi");
    closeRe.lastIndex = innerStart;
    const close = closeRe.exec(html);
    const innerEnd = close ? close.index : html.length;
    found.push({
      tagName,
      start,
      innerStart,
      innerEnd,
      end: close ? innerEnd + close[0].length : html.length,
      markup: html.slice(start, close ? innerEnd + close[0].length : html.length),
      inner: html.slice(innerStart, innerEnd),
    });
    openRe.lastIndex = innerStart;
  }
  return found;
}

/** Parse a CSS declaration block into { property: value } pairs (last wins). */
function declarations(body) {
  const map = {};
  for (const part of body.split(";")) {
    const at = part.indexOf(":");
    if (at < 0) continue;
    const prop = part.slice(0, at).trim().toLowerCase();
    const value = part.slice(at + 1).trim();
    if (prop) map[prop] = value;
  }
  return map;
}

/** Resolve `var(--token)` references against a declaration map. */
function resolveVar(value, decls, depth = 0) {
  if (depth > 5) return value;
  const ref = String(value).match(/^var\(\s*(--[\w-]+)\s*\)$/);
  if (!ref) return value;
  const target = decls[ref[1]];
  return target === undefined ? value : resolveVar(target, decls, depth + 1);
}

/** Rendered rgb() for a hex colour, for WCAG relative-luminance checks. */
function hexToRgb(hex) {
  const clean = String(hex).replace("#", "");
  const full =
    clean.length === 3
      ? clean
          .split("")
          .map((c) => c + c)
          .join("")
      : clean;
  assert.match(full, /^[0-9a-f]{6}$/i, `not a hex colour: ${hex}`);
  return [0, 2, 4].map((i) => Number.parseInt(full.slice(i, i + 2), 16));
}

function relativeLuminance([r, g, b]) {
  const channel = (value) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(fgHex, bgHex) {
  const [l1, l2] = [relativeLuminance(hexToRgb(fgHex)), relativeLuminance(hexToRgb(bgHex))].sort(
    (a, b) => b - a
  );
  return (l1 + 0.05) / (l2 + 0.05);
}

/** Top-level :root token map of a CSS text. */
function rootTokens(css) {
  const base = stripCssComments(css).split("@media")[0];
  const root = base.match(/:root\s*\{([^}]*)\}/);
  return root ? declarations(root[1]) : {};
}

/* ------------------------------------------------------------ asset basics */

test("assets: index.html and styles.css exist and are non-trivial", () => {
  assert.ok(fs.existsSync(HTML_PATH), "index.html must exist at the workspace root");
  assert.ok(fs.existsSync(CSS_PATH), "styles.css must exist at the workspace root");

  const html = readHtml();
  const css = readCss();
  assert.ok(html.length > 1500, `index.html looks too small (${html.length} bytes)`);
  assert.ok(css.length > 1500, `styles.css looks too small (${css.length} bytes)`);
});

test("assets: stylesheet is linked with a relative path (A9)", () => {
  const html = readHtml();
  assert.match(
    html,
    /<link[^>]+rel="stylesheet"[^>]+href="styles\.css"/,
    "index.html must link styles.css via a relative href"
  );
  assert.ok(!/href="styles\.css"[^>]*\bmedia=/.test(html), "stylesheet must apply to every viewport");
});

test("assets: no remote assets, scripts or runtime dependencies (A13, A20)", () => {
  const html = readHtml();
  const css = readCss();
  const forbidden = [
    /(?:src|href)\s*=\s*"(?:https?:)?\/\//i,
    /@import\s+(?:url\()?["']?https?:/i,
    /<script\b/i,
    /<iframe\b/i,
    /<link[^>]+rel="preconnect"/i,
  ];
  for (const pattern of forbidden) {
    assert.ok(!pattern.test(html), `index.html must not load remote assets or scripts: ${pattern}`);
    assert.ok(!pattern.test(css), `styles.css must not load remote assets: ${pattern}`);
  }
  assert.ok(!/https?:\/\//i.test(css), "styles.css must not reference any URL");
});

test("assets: suite runs on node:test without skips or runtime deps (A18)", () => {
  const self = fs.readFileSync(__filename, "utf8");
  assert.match(self, /require\("node:test"\)/, "tests must use node:test");
  assert.match(self, /require\("node:assert\/strict"\)/, "tests must use node:assert/strict");
  assert.ok(!/\.skip\(|\{\s*skip:/.test(self), "no skipped cases may stand in for A1-A17/A21-A25");
  assert.ok(!/require\("(?!node:)/.test(self), "tests must not import third-party packages");
});

/* ------------------------------------------------------------- document head */

test("document: declares utf-8 charset, a language and a responsive viewport (A1, A16)", () => {
  const html = readHtml();
  assert.match(html, /^<!doctype html>/i, "must start with an HTML5 doctype");
  assert.match(html, /<meta\s+charset="utf-8"/i, 'must declare <meta charset="utf-8">');

  const lang = html.match(/<html[^>]*\blang="([^"]+)"/i);
  assert.ok(lang, "<html> must carry a lang attribute");
  assert.match(lang[1], /^[a-z]{2,3}(-[a-z0-9]{2,8})*$/i, `lang="${lang[1]}" must be a BCP-47 tag`);
  assert.match(lang[1], /^zh/i, "content is Chinese, so lang should be zh-CN");

  const viewport = html.match(/<meta[^>]+name="viewport"[^>]+content="([^"]+)"/i);
  assert.ok(viewport, "must declare a viewport meta tag");
  assert.match(viewport[1], /width=device-width/i, "viewport must use width=device-width");
  assert.match(viewport[1], /initial-scale=1/i, "viewport must set initial-scale=1");
});

test("document: has a unique descriptive <title> and a meta description (A1, A16)", () => {
  const html = readHtml();
  const titles = html.match(/<title>[\s\S]*?<\/title>/gi) || [];
  assert.equal(titles.length, 1, "exactly one <title> element");
  const title = titles[0].replace(/<\/?title>/gi, "").trim();
  assert.ok(title.length >= 10, "title must be descriptive");
  assert.match(title, /OwO Agent SDK/i, "title must name the product");

  const desc = html.match(/<meta[^>]+name="description"[^>]+content="([^"]+)"/i);
  assert.ok(desc, "must declare a meta description");
  assert.ok(desc[1].trim().length >= 40, "meta description must be substantive");
});

/* ------------------------------------------------------ landmark structure */

test("structure: one <main>, header and footer landmarks with accessible names (A3)", () => {
  const html = readHtml();
  assert.equal(count(html, "<main"), 1, "exactly one <main> landmark");
  assert.equal(count(html, "<header"), 1, "exactly one <header> element");
  assert.equal(count(html, "<footer"), 1, "exactly one <footer> element");
  assert.match(html, /<main[^>]*\bid="main"/i, '<main> must expose id="main" for the skip link');
  assert.match(html, /<nav[^>]*aria-label="[^"]+"/i, "navigation must have an aria-label");
});

test("structure: skip link is the first focusable element and targets main (A4)", () => {
  const html = readHtml();
  const firstLink = html.match(/<a\b[^>]*>/i);
  assert.ok(firstLink, "document must contain links");
  assert.match(firstLink[0], /class="skip-link"/, "the first <a> must be the skip link");
  assert.match(firstLink[0], /href="#main"/, "skip link must point at #main");

  const skipAt = html.indexOf("skip-link");
  assert.ok(skipAt > -1 && skipAt < html.indexOf("<main"), "skip link must precede <main>");
});

test("structure: every section is labelled and headings form a single h1 hierarchy (A2, A3)", () => {
  const html = readHtml();
  const sections = html.match(/<section\b[^>]*>/gi) || [];
  assert.ok(sections.length >= 4, `expected at least 4 sections, found ${sections.length}`);
  for (const tag of sections) {
    assert.match(tag, /aria-labelledby="[^"]+"/i, `section must be labelled: ${tag}`);
  }
  assert.equal((html.match(/<h1\b/gi) || []).length, 1, "exactly one <h1>");
  assert.ok((html.match(/<h2\b/gi) || []).length >= 3, "each major section needs an <h2>");
  assert.ok((html.match(/<h3\b/gi) || []).length >= 4, "capability/step items need <h3>");

  // No skipped levels: heading sequence must start at h1 and never jump +2.
  const headings = [...html.matchAll(/<h([1-6])\b/gi)].map((m) => Number(m[1]));
  assert.equal(headings[0], 1, "the first heading must be the <h1>");
  for (let i = 1; i < headings.length; i += 1) {
    assert.ok(
      headings[i] - headings[i - 1] <= 1,
      `heading level jumps from h${headings[i - 1]} to h${headings[i]}`
    );
  }

  // Every aria-labelledby / aria-describedby / aria-controls target must exist.
  const ids = new Set(idsIn(html));
  const refRe = /aria-(?:labelledby|describedby|controls)="([^"]+)"/g;
  let refMatch;
  while ((refMatch = refRe.exec(html)) !== null) {
    for (const ref of refMatch[1].split(/\s+/).filter(Boolean)) {
      assert.ok(ids.has(ref), `aria reference "${ref}" must resolve to an element id`);
    }
  }
});

test("structure: ids are unique across the document (A3)", () => {
  const html = readHtml();
  const seen = new Set();
  for (const id of idsIn(html)) {
    assert.ok(!seen.has(id), `duplicate id "${id}"`);
    seen.add(id);
  }
  assert.ok(seen.size >= 5, "page should expose several anchor targets");
});

test("structure: in-page anchors resolve, without empty or dead anchors (A5)", () => {
  const html = readHtml();
  const ids = new Set(idsIn(html));
  const anchors = anchorsIn(html);

  assert.ok(anchors.length >= 8, "landing page needs navigation, CTAs and footer links");
  for (const anchor of anchors) {
    assert.notEqual(anchor.href, null, `anchor without href: ${anchor.raw.slice(0, 60)}`);
    assert.notEqual(anchor.href.trim(), "", `anchor has an empty href: ${anchor.raw.slice(0, 60)}`);
    assert.notEqual(anchor.href.trim(), "#", `anchor uses a dead href="#": ${anchor.text}`);

    if (!anchor.href.startsWith("#")) continue;
    const target = anchor.href.slice(1);
    assert.ok(ids.has(target), `in-page link "#${target}" must resolve to an id`);
  }
});

/* ------------------------------------------------------ content requirements */

test("content: required sections are present in reading order (A3)", () => {
  const html = readHtml();
  const order = ["#features", "#quickstart", "#faq"];
  const positions = order.map((href) => html.indexOf(`href="${href}"`));
  positions.forEach((pos, i) => assert.ok(pos > -1, `navigation must link ${order[i]}`));
  assert.ok(
    positions[0] < positions[1] && positions[1] < positions[2],
    "sections must appear in the order features -> quickstart -> faq"
  );

  const domOrder = ['id="features"', 'id="quickstart"', 'id="faq"'].map((needle) =>
    html.indexOf(needle)
  );
  domOrder.forEach((pos, i) => assert.ok(pos > -1, `section ${order[i]} must exist in the DOM`));
  assert.ok(
    domOrder[0] < domOrder[1] && domOrder[1] < domOrder[2],
    "sections must be in DOM order features -> quickstart -> faq"
  );
});

test("content: primary and secondary CTAs exist, also inside the hero (A6)", () => {
  const html = readHtml();
  const heroes = elementsWith(html, "class", "hero");
  assert.equal(heroes.length, 1, 'exactly one hero section (class="hero") must exist');
  const heroAnchors = anchorsIn(heroes[0].markup);
  const allAnchors = anchorsIn(html);

  const isPrimary = (a) =>
    a.classes.includes("btn--primary") || /开始接入|Get started/i.test(a.text);
  const isSecondary = (a) =>
    a.classes.includes("btn--secondary") || /阅读文档|Read the docs/i.test(a.text);

  const primary = allAnchors.filter(isPrimary);
  const secondary = allAnchors.filter(isSecondary);
  assert.ok(primary.length >= 1, "a primary CTA (开始接入) must appear on the page");
  assert.ok(secondary.length >= 1, "a secondary docs CTA (阅读文档) must appear on the page");
  assert.ok(heroAnchors.some(isPrimary), "the primary CTA must appear inside the hero section");
  assert.ok(heroAnchors.some(isSecondary), "the secondary CTA must appear inside the hero section");
  assert.ok(
    primary.some((a) => a.href === "#quickstart"),
    'the primary CTA must point at "#quickstart"'
  );

  // The two hero CTAs must stay distinct actions: the docs CTA may not reuse
  // the primary target (that would collapse both CTAs into one action).
  const heroPrimary = heroAnchors.find(isPrimary);
  const heroSecondary = heroAnchors.find(isSecondary);
  assert.notEqual(
    heroSecondary.href,
    heroPrimary.href,
    "the secondary CTA must not reuse the primary CTA target"
  );
});

test("content: quickstart has an ordered 3-step list and code samples (A7)", () => {
  const html = readHtml();
  const section = html.slice(html.indexOf('id="quickstart"'), html.indexOf('id="faq"'));
  assert.ok(section.length > 0, "quickstart section must exist");

  const ol = section.match(/<ol\b[^>]*>([\s\S]*?)<\/ol>/);
  assert.ok(ol, "quickstart needs a closed, native <ol>");
  const steps = ol[1].match(/<li\b/gi) || [];
  assert.equal(steps.length, 3, `quickstart must have exactly 3 steps, found ${steps.length}`);
  assert.ok((section.match(/<pre><code>/g) || []).length >= 1, "quickstart needs <pre><code> samples");
  assert.match(section, /npm install/, "step 1 must show the install command");
});

test("content: FAQ exposes 3-5 native Q&A items (A8)", () => {
  const html = readHtml();
  const section = html.slice(html.indexOf('id="faq"'), html.indexOf("</main>"));
  const details = section.match(/<details\b/gi) || [];
  const summaries = section.match(/<summary\b/gi) || [];
  assert.ok(details.length >= 3 && details.length <= 5, `FAQ needs 3-5 items, found ${details.length}`);
  assert.equal(summaries.length, details.length, "each <details> needs exactly one <summary>");
  for (const summary of section.match(/<summary>([\s\S]*?)<\/summary>/gi) || []) {
    assert.ok(textOf(summary).length >= 4, "every FAQ question needs readable text");
  }
});

test("content: product copy is real text, not placeholder filler", () => {
  const html = readHtml();
  assert.ok(!/lorem ipsum/i.test(html), "no lorem ipsum placeholder text");
  assert.ok(!/\bTBD\b|\bTODO\b/i.test(html), "no TODO/TBD placeholders");

  const text = textOf(html.replace(/<script[\s\S]*?<\/script>/gi, " "));
  assert.ok(text.length > 800, `visible copy looks thin (${text.length} chars)`);
  assert.match(text, /OwO Agent SDK/, "visible copy must name the product");
});

test("content: the <h1> carries the positioning sentence verbatim (A25)", () => {
  const html = readHtml();
  const h1 = html.match(/<h1\b[^>]*>([\s\S]*?)<\/h1>/i);
  assert.ok(h1, "page needs an <h1>");
  const h1Text = textOf(h1[1]);
  const sentence = "OwO Agent SDK 让开发者用少量代码构建可编排、可观测的 agent 能力。";
  assert.ok(
    h1Text.includes(sentence),
    `the positioning sentence must appear contiguously inside <h1>, got: ${h1Text}`
  );
});

test("content: interactive elements have discernible text", () => {
  const html = readHtml();
  const anchors = anchorsIn(html);
  assert.ok(anchors.length >= 8, "landing page needs navigation, CTAs and footer links");
  for (const anchor of anchors) {
    const labelled =
      anchor.text.length > 0 ||
      /aria-label="[^"]+"/i.test(anchor.attrs) ||
      /aria-hidden="true"/i.test(anchor.raw);
    assert.ok(labelled, `anchor has no accessible name: ${anchor.raw.slice(0, 80)}`);
  }
});

/* ------------------------------------------ placeholder docs slot (A21-A25) */

test("content: placeholder docs slot is unique, annotated and not self-referencing (A21-A25)", () => {
  const html = readHtml();
  const ids = idsIn(html);

  // A21: exactly one #docs-placeholder, inside Quickstart and after the closed
  // <ol> — an annotation, not a fourth step.
  assert.equal(
    ids.filter((id) => id === "docs-placeholder").length,
    1,
    'id="docs-placeholder" must appear exactly once'
  );
  const slots = elementsWith(html, "id", "docs-placeholder");
  assert.equal(slots.length, 1, 'exactly one element may hold id="docs-placeholder"');
  const slot = slots[0];
  assert.notEqual(
    slot.tagName,
    "a",
    "the placeholder docs slot must be a block element, not an anchor (that allows a self link)"
  );

  const quickstarts = elementsWith(html, "id", "quickstart");
  assert.equal(quickstarts.length, 1, 'exactly one element may hold id="quickstart"');
  const quickstart = quickstarts[0];
  assert.ok(
    slot.start > quickstart.start && slot.end <= quickstart.end,
    "the placeholder docs element must live inside the Quickstart <section>"
  );

  const closeOl = quickstart.inner.lastIndexOf("</ol>");
  assert.ok(closeOl > -1, "quickstart must contain a closed <ol>");
  assert.ok(
    slot.start - quickstart.innerStart > closeOl,
    "the placeholder docs element must appear after the closed <ol>, not inside a step"
  );

  // A22: the annotation is readable text with the mandated literal.
  assert.match(textOf(slot.inner), /占位链接位/, 'the docs slot visible text must contain "占位链接位"');

  // A23: the slot links to #docs-placeholder-note, and that id is unique.
  const noteLinks = anchorsIn(slot.inner).filter((a) => a.href === "#docs-placeholder-note");
  assert.ok(noteLinks.length >= 1, 'the docs slot must link to "#docs-placeholder-note"');
  assert.equal(
    ids.filter((id) => id === "docs-placeholder-note").length,
    1,
    'id="docs-placeholder-note" must appear exactly once'
  );
  assert.equal(elementsWith(html, "id", "docs-placeholder-note").length, 1, "the note target must exist once");

  // A24: every #docs-placeholder consumer sits outside the slot itself. This is
  // the regression behind issue 6bcf5dce: a link whose href equals an id on its
  // own container (or an ancestor) resolves fine, yet sends the keyboard user
  // nowhere.
  const consumers = anchorsIn(html).filter((a) => a.href === "#docs-placeholder");
  assert.ok(consumers.length >= 2, "the hero docs CTA and the footer must link to #docs-placeholder");
  assert.ok(
    consumers.some((a) => a.classes.includes("btn--secondary")),
    "the hero docs CTA (btn--secondary) must be one of the consumers"
  );
  for (const link of consumers) {
    const ancestors = ancestorIdsAt(html, link.index);
    assert.ok(
      !ancestors.includes("docs-placeholder"),
      `link "${link.text}" targets its own container (#docs-placeholder): dead self-loop`
    );
  }
  // No segment may be an empty fragment or a self target on the slot element.
  assert.ok(
    !/<a\b[^>]*\sid="docs-placeholder"/i.test(html),
    "the slot id must not sit on an anchor element"
  );

  // Docs-type links may not hijack an existing content section as a stand-in.
  const hijackTargets = new Set(["", "#", "#quickstart", "#features", "#faq", "#top"]);
  const docLinks = anchorsIn(html).filter(
    (a) => /文档/.test(a.text) || a.classes.some((name) => /docs/.test(name))
  );
  assert.ok(docLinks.length >= 2, "the docs path must be visible in at least two places");
  for (const link of docLinks) {
    assert.ok(
      !hijackTargets.has((link.href || "").trim()),
      `docs link "${link.text}" points at ${link.href} instead of the placeholder slot`
    );
    assert.ok(!/^https?:/i.test(link.href || ""), `docs link must stay on-page: ${link.href}`);
  }
});

/* ------------------------------------------------------------- media/render */

test("media: images and decorative visuals are described or hidden (A14)", () => {
  const html = readHtml();
  for (const img of html.match(/<img\b[^>]*>/gi) || []) {
    assert.match(img, /\salt="([^"]*)"/i, `every <img> needs an alt attribute: ${img}`);
  }

  // Decoration may be hidden on the wrapper OR on the node itself; what matters
  // is that no decorative box reaches the accessibility tree.
  const hidden = elementsWith(html, "aria-hidden", "true");
  const hiddenRanges = hidden.map((el) => [el.start, el.end]);
  const decorative =
    html.match(/<(div|span|svg|figure)\b[^>]*class="[^"]*(?:figure|grid|mark|visual|deco)[^"]*"[^>]*>/gi) || [];
  for (const node of decorative) {
    const at = html.indexOf(node);
    const insideHidden = hiddenRanges.some(([start, end]) => at > start && at < end);
    assert.ok(
      insideHidden || /aria-hidden="true"/i.test(node),
      `decorative visual must be aria-hidden (itself or via a wrapper): ${node.slice(0, 70)}`
    );
  }
});

test("forms: page ships no submittable form and no unlabelled control (A15)", () => {
  const html = readHtml();
  assert.ok(!/<form\b/i.test(html), "the landing page must not ship a form");
  for (const control of html.match(/<(input|select|textarea)\b[^>]*>/gi) || []) {
    assert.ok(
      /aria-label="[^"]+"|\sid="/i.test(control),
      `form control needs a label or aria-label: ${control.slice(0, 70)}`
    );
  }
});

/* ------------------------------------------------------------- CSS checks */

test("css: colour decisions are driven by design tokens", () => {
  const tokens = rootTokens(readCss());
  assert.ok(tokens["--ink"], "colour tokens must be declared on :root");
  assert.ok(tokens["--focus"], "a focus token must exist");
  assert.match(tokens["--surface"] || "", /^#[0-9a-f]{3,6}$/i, "surface must be an explicit colour");
  assert.ok(!/lorem ipsum/i.test(readCss()), "no placeholder filler text in CSS");
});

test("css: contrast ledger records AA-compliant text pairs and tokens are re-checked (A17)", () => {
  // A17 asks the implementer to record the rendered contrast pairs. Recorded
  // "text on background" rows are re-computed from their hex values so the
  // ledger cannot drift; the token pairs the page always renders are checked
  // unconditionally, so a stylesheet without a ledger still passes as long as
  // its tokens are compliant.
  const css = readCss();
  const flat = stripCssComments(css);

  const textPairs = [...flat.matchAll(/(#[0-9a-f]{6})\s*\|\s*(#[0-9a-f]{6})\s*\|\s*(\d{1,2}\.\d{1,2}):1/gi)];
  for (const pair of textPairs) {
    const claimed = Number.parseFloat(pair[3]);
    const actual = contrastRatio(pair[1], pair[2]);
    assert.ok(
      claimed <= actual + 0.05,
      `ledger claims ${pair[3]}:1 for ${pair[1]} on ${pair[2]}, recomputed ${actual.toFixed(2)}:1`
    );
    assert.ok(
      actual >= 4.5,
      `recorded pair ${pair[1]} on ${pair[2]} is ${actual.toFixed(2)}:1, below the 4.5:1 text floor`
    );
  }

  const tokens = rootTokens(css);
  const requiredPairs = [
    ["--ink", "--surface", "body text"],
    ["--ink-muted", "--surface", "secondary text"],
    ["--on-brand", "--brand", "primary CTA label"],
    ["--docs-ink", "--docs-bg", "placeholder docs slot text"],
    ["--docs-link", "--docs-bg", "placeholder docs slot link"],
  ];
  for (const [fgToken, bgToken, label] of requiredPairs) {
    const fg = resolveVar(tokens[fgToken] || "", tokens);
    const bg = resolveVar(tokens[bgToken] || "", tokens);
    assert.ok(fg && bg, `${label}: ${fgToken} and ${bgToken} must both be defined on :root`);
    const ratio = contrastRatio(fg, bg);
    assert.ok(
      ratio >= 4.5,
      `${label} (${fgToken} on ${bgToken}) is ${ratio.toFixed(2)}:1, below the 4.5:1 floor`
    );
  }
});

test("css: responsive layout is mobile-first with explicit breakpoints (A10)", () => {
  const css = readCss();
  const blocks = mediaBlocks(css);
  assert.ok(blocks.length >= 3, `expected at least 3 @media blocks, found ${blocks.length}`);

  const queries = blocks
    .filter((b) => /(?:min|max)-width/.test(b.query))
    .map((b) => b.query)
    .join(" | ");
  assert.match(queries, /min-width:\s*600px|max-width:\s*599px/, "tablet breakpoint around 600px");
  assert.match(queries, /min-width:\s*900px/, "desktop breakpoint (min-width: 900px)");

  const base = stripCssComments(css).split("@media")[0];
  assert.match(base, /grid-template-columns:\s*1fr/, "base layout must be single column (mobile-first)");

  const tablet = blocks.find((b) => /min-width:\s*600px/.test(b.query));
  assert.match(tablet.body, /repeat\(2,/, "feature cards go to 2 columns on tablet");

  const desktop = blocks.find((b) => /min-width:\s*900px/.test(b.query));
  assert.match(desktop.body, /repeat\(3,/, "steps go to a 3-column grid on desktop");

  const narrow = blocks.find((b) => /max-width:\s*599px/.test(b.query));
  if (narrow) {
    assert.match(narrow.body, /grid-template-columns:\s*1fr/, "narrow screens must stay single column");
  }
});

test("css: long lines and wide media cannot overflow the viewport (A10)", () => {
  const css = stripCssComments(readCss());
  assert.match(css, /overflow-x:\s*auto/, "code blocks must scroll horizontally");
  assert.match(css, /overflow-x:\s*hidden/, "body must not gain a horizontal scrollbar");
  assert.match(css, /max-width:\s*var\(--wrap\)/, "content must be capped by a max-width container");
});

test("css: focus ring and pointer-target affordances (A11)", () => {
  const css = readCss();
  const flat = stripCssComments(css);
  const tokens = rootTokens(css);

  // A visible ring driven by the focus token, either on :focus-visible alone or
  // through a shared :focus, :focus-visible selector list.
  const ringRules = [...flat.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((rule) =>
    /:focus-visible/.test(rule[1])
  );
  assert.ok(ringRules.length >= 1, "a :focus-visible rule must exist");
  const hasRing = ringRules.some((rule) => {
    const outline = declarations(rule[2]).outline || "";
    const width = Number.parseInt(outline, 10);
    return Number.isFinite(width) && width >= 2 && /var\(--focus\)/.test(outline);
  });
  assert.ok(hasRing, "the focus ring must be a >= 2px outline driven by var(--focus)");
  assert.equal(tokens["--tap"], "44px", "--tap must be 44px (44x44 CSS px minimum target)");
  assert.match(flat, /min-height:\s*var\(--tap\)/, "interactive elements must honour the tap token");

  // A bare `outline: none` is only acceptable when a replacement ring exists.
  for (const rule of [...flat.matchAll(/([^{}]+)\{([^{}]*)\}/g)]) {
    const outline = declarations(rule[2]).outline;
    if (!outline || !/^(?:none|0)$/.test(outline)) continue;
    assert.ok(hasRing, `rule removes the outline without an explicit replacement: ${rule[1].trim()}`);
  }
});

test("css: reduced motion is respected (A12)", () => {
  const flat = stripCssComments(readCss());
  const block = mediaBlocks(flat).find((b) => /prefers-reduced-motion:\s*reduce/.test(b.query));
  assert.ok(block, "must respect reduced motion via @media (prefers-reduced-motion: reduce)");
  assert.match(block.body, /scroll-behavior:\s*auto/, "reduced motion must disable smooth scrolling");
  assert.match(
    block.body,
    /transition-duration:|animation-duration:/,
    "reduced motion must mute transitions/animations"
  );
});

test("css: optional dark theme and print sheet stay consistent when present (optional)", (t) => {
  // OPTIONAL, never a hard contract. Dark mode is "recommended but optional"
  // (brief §5) and explicitly a non-goal as a toggle (§8); the print sheet is
  // required by no acceptance item. Only blocks that actually exist are
  // validated, so removing either one keeps this suite green.
  const flat = stripCssComments(readCss());
  const colourSchemes = mediaBlocks(flat).filter((b) =>
    /prefers-color-scheme:\s*(?:dark|light)/.test(b.query)
  );
  const darkBlocks = colourSchemes.filter((b) => /dark/.test(b.query));

  if (colourSchemes.length === 0) {
    t.diagnostic("no prefers-color-scheme block — dark mode is optional (brief §5/§8)");
  } else if (darkBlocks.length === 0) {
    t.diagnostic("only a light-scheme block is present — dark mode is optional");
  } else {
    assert.equal(darkBlocks.length, 1, "if a dark theme exists it must be declared once");
    const body = stripCssComments(darkBlocks[0].body);
    assert.match(body, /--ink:\s*#/, "a dark theme must redefine the text token");
    assert.match(body, /--surface:\s*#/, "a dark theme must redefine the surface token");
    assert.ok(
      !/class="[^"]*(?:theme-toggle|dark-toggle|switch)/i.test(readHtml()),
      "brief §8: no dark-mode toggle control (auto-adaptation only)"
    );
    // Optional blocks still have to be valid: check only self-contained pairs
    // (token defined inside the block, or inherited from :root) and only when
    // both sides are explicit hex values.
    const inherited = rootTokens(readCss());
    const own = declarations(body);
    const darkTokens = { ...inherited, ...own };
    const pairs = [
      ["--ink", "--surface", "dark body text"],
      ["--docs-ink", "--docs-bg", "dark placeholder docs slot text"],
    ];
    for (const [fgToken, bgToken, label] of pairs) {
      const inBlock = own[fgToken] && own[bgToken];
      if (!inBlock) {
        t.diagnostic(`${label}: tokens are not both set inside the dark block — pair not checked`);
        continue;
      }
      const fg = resolveVar(own[fgToken], darkTokens);
      const bg = resolveVar(own[bgToken], darkTokens);
      if (!/^#[0-9a-f]{3,6}$/i.test(fg) || !/^#[0-9a-f]{3,6}$/i.test(bg)) {
        t.diagnostic(`${label}: non-hex values (${fg} / ${bg}) — pair not checked`);
        continue;
      }
      const ratio = contrastRatio(fg, bg);
      assert.ok(ratio >= 4.5, `${label} is ${ratio.toFixed(2)}:1, below the 4.5:1 floor`);
    }
  }

  const print = mediaBlocks(flat).find((b) => /^print$/.test(b.query.trim()));
  if (!print) {
    t.diagnostic("no @media print block — optional, nothing to assert");
  } else {
    assert.ok(print.body.trim().length > 0, "a @media print block must hold declarations");
  }

  // Whatever the optional blocks do, dark-only colours must not leak into the
  // default cascade.
  const defaultBlock = flat.split("@media")[0];
  assert.ok(
    !/background:\s*#0[0-9a-f]{5}\b/i.test(defaultBlock),
    "near-black backgrounds belong inside a prefers-color-scheme block"
  );
});

test("css: skip link is visually hidden but stays reachable by keyboard", () => {
  const css = readCss();

  // Base .skip-link rule only — state rules (.skip-link:focus) are separate selectors.
  const baseRules = ruleBodies(css, ".skip-link");
  assert.ok(baseRules.length >= 1, "a base .skip-link rule must exist");
  assert.ok(
    baseRules.some((body) => /transform:\s*translateY\(-200%\)/.test(body)),
    "base .skip-link rule must hide it with transform: translateY(-200%)"
  );
  assert.ok(
    !baseRules.some((body) => /display:\s*none/.test(body)),
    "base .skip-link rule must never use display:none (that removes it from the tab order)"
  );
  assert.ok(
    baseRules.some((body) => /min-height:\s*var\(--tap\)/.test(body)),
    "base .skip-link rule must be a full-size tap target"
  );

  const focusRules = ruleBodies(css, ".skip-link:focus").concat(ruleBodies(css, ".skip-link:focus-visible"));
  assert.ok(focusRules.length >= 1, "a .skip-link:focus rule must exist");
  assert.ok(
    focusRules.some((body) => /transform:\s*translateY\(0\)/.test(body)),
    "focused skip link must slide back into view"
  );
});

test("css: classes used in the HTML are styled, and styles target real markup", () => {
  const htmlClassesUsed = htmlClasses(readHtml());
  const cssClassesDefined = cssClasses(readCss());

  // Pure `__name` wrapper hooks (e.g. .brand__name, .hero__inner) may inherit
  // typography/layout from their block, so they are reported but not blocking.
  const isCompositionHook = (name) => /__[a-z][\w-]*$/.test(name);
  const unstyled = [...htmlClassesUsed].filter((name) => !cssClassesDefined.has(name));
  const blocking = unstyled.filter((name) => !isCompositionHook(name));
  assert.deepEqual(blocking, [], `classes used in HTML but missing from styles.css: ${blocking}`);
  assert.ok(
    unstyled.every((name) => /__/.test(name)),
    `unstyled classes must be BEM element hooks only: ${unstyled.join(", ")}`
  );

  // Reverse direction only for project-specific prefixes; shared utilities may
  // be composed elsewhere.
  const ownPattern = /^(feature|hero|step|faq|btn|nav|brand|site-|docs)/;
  const dead = [...cssClassesDefined].filter(
    (name) => (ownPattern.test(name) || name === "skip-link") && !htmlClassesUsed.has(name)
  );
  assert.deepEqual(dead, [], `classes styled in CSS but absent from HTML: ${dead}`);
});

/* -------------------------------------------------------- coverage ledger */

test("coverage: A1-A17 and A21-A25 each map to a named, executable test (A19)", () => {
  const self = fs.readFileSync(__filename, "utf8");
  const required = [
    "A1",
    "A2",
    "A3",
    "A4",
    "A5",
    "A6",
    "A7",
    "A8",
    "A9",
    "A10",
    "A11",
    "A12",
    "A13",
    "A14",
    "A15",
    "A16",
    "A17",
    "A18",
    "A20",
    "A21",
    "A22",
    "A23",
    "A24",
    "A25",
  ];
  const testNames = [...self.matchAll(/^test\("([^"]+)"/gm)].map((m) => m[1]);
  assert.ok(testNames.length >= 20, `suite should be substantial, found ${testNames.length} tests`);

  const docsSlotTest = testNames.filter((name) => /placeholder docs slot/.test(name));
  assert.equal(docsSlotTest.length, 1, "exactly one dedicated placeholder docs slot test must exist");

  const uncovered = required.filter((id) => {
    if (/^A2[1-5]$/.test(id)) return docsSlotTest.length === 0;
    return !testNames.some((name) => new RegExp(`\\b${id}\\b`).test(name));
  });
  assert.deepEqual(uncovered, [], `requirements without a named test: ${uncovered.join(", ")}`);

  // The two regression areas must be reachable from the coverage map too.
  assert.ok(
    testNames.some((name) => /A21-A25/.test(name)),
    "the docs slot test must advertise the A21-A25 range"
  );
  assert.ok(
    !testNames.some((name) => /dark mode|print styles required/i.test(name)),
    "optional dark/print features must not be named as hard requirements"
  );
});
