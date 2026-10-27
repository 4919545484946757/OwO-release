"use strict";

/**
 * OwO Agent SDK landing page — structural / accessibility / content checks.
 *
 * Runner:  node --test tests/design.test.cjs
 * Deps:    node:test, node:assert/strict, node:fs (no third-party packages)
 * Scope:   read-only verification of index.html + styles.css at the workspace root.
 */

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const HTML_PATH = path.join(ROOT, "index.html");
const CSS_PATH = path.join(ROOT, "styles.css");
const HTML_REL = "index.html";
const CSS_REL = "styles.css";

/* ------------------------------------------------------------------ helpers */

function read(filePath, label) {
  assert.ok(fs.existsSync(filePath), `${label} must exist at workspace root (${filePath})`);
  const text = fs.readFileSync(filePath, "utf8");
  assert.ok(text.trim().length > 0, `${label} must be non-empty`);
  return text;
}

function decodeEntities(source) {
  return source
    .replace(/&nbsp;/g, " ")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");
}

/** Text content with tags stripped, entities decoded, whitespace collapsed. */
function visibleText(source) {
  return decodeEntities(
    source
      .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, " ")
      .replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, " ")
      .replace(/<!--[\s\S]*?-->/g, " ")
      .replace(/<[^>]+>/g, " ")
  )
    .replace(/\s+/g, " ")
    .trim();
}

/** All start tags of the given element name, e.g. tagMatches(html, "section"). */
function tagMatches(source, name) {
  const re = new RegExp(`<${name}\\b([^>]*)>`, "gi");
  const out = [];
  let m;
  while ((m = re.exec(source)) !== null) out.push({ attrs: m[1] || "", raw: m[0], index: m.index });
  return out;
}

/** Attribute value inside a raw start-tag attribute string (null when absent). */
function attr(attrs, name) {
  const re = new RegExp(`\\b${name}\\s*=\\s*("([^"]*)"|'([^']*)'|([^\\s"'>]+))`, "i");
  const m = attrs.match(re);
  if (!m) return null;
  return m[2] !== undefined ? m[2] : m[3] !== undefined ? m[3] : m[4];
}

function ids(source) {
  const set = new Set();
  const re = /\bid\s*=\s*("([^"]*)"|'([^']*)')/g;
  let m;
  while ((m = re.exec(source)) !== null) set.add(m[2] !== undefined ? m[2] : m[3]);
  return set;
}

const HEADING_RE = /<h([1-6])\b[^>]*>([\s\S]*?)<\/h\1\s*>/gi;

function headings(source) {
  const out = [];
  let m;
  while ((m = HEADING_RE.exec(source)) !== null) {
    out.push({ level: Number(m[1]), text: visibleText(m[2]) });
  }
  return out;
}

function pickAll(source, re) {
  const out = [];
  let m;
  while ((m = re.exec(source)) !== null) out.push(m);
  return out;
}

const html = read(HTML_PATH, HTML_REL);
const css = read(CSS_PATH, CSS_REL);
const htmlText = visibleText(html);
const headingList = headings(html);

/* ------------------------------------------------- 1. required files exist */

test("index.html and styles.css exist at the workspace root and are non-empty", () => {
  assert.ok(fs.statSync(HTML_PATH).isFile(), "index.html must be a regular file");
  assert.ok(fs.statSync(CSS_PATH).isFile(), "styles.css must be a regular file");
  assert.ok(html.length > 1000, `index.html unexpectedly small (${html.length} bytes)`);
  assert.ok(css.length > 500, `styles.css unexpectedly small (${css.length} bytes)`);
});

/* -------------------------------------------------- 2. stylesheet wiring */

test("index.html links styles.css with a relative, same-origin href", () => {
  const links = tagMatches(html, "link");
  const stylesheet = links.filter((l) => (attr(l.attrs, "rel") || "").toLowerCase() === "stylesheet");
  assert.equal(stylesheet.length, 1, 'expected exactly one <link rel="stylesheet">');
  const href = attr(stylesheet[0].attrs, "href");
  assert.equal(href, "styles.css", "stylesheet href must be exactly styles.css (relative, offline-safe)");
});

/* --------------------------------------------- 3. section ids appear once */

test("each required section id appears exactly once", () => {
  const required = ["top", "hero", "features", "how-it-works", "quickstart", "faq", "footer"];
  for (const id of required) {
    const pattern = new RegExp(`\\bid\\s*=\\s*("${id}"|'${id}')`, "g");
    const occurrences = pickAll(html, pattern);
    assert.equal(occurrences.length, 1, `section id "${id}" must appear exactly once, found ${occurrences.length}`);
  }
  const capCount = pickAll(html, /\bid\s*=\s*["']capabilities["']/g).length;
  assert.ok(capCount <= 1, `optional id "capabilities" must not repeat (found ${capCount})`);
});

/* -------------------------------------------- 4. single h1, no heading skips */

test("exactly one <h1> exists", () => {
  const h1s = tagMatches(html, "h1");
  assert.equal(h1s.length, 1, `expected exactly one <h1>, found ${h1s.length}`);
});

test("heading levels never skip (no h1 -> h3 or h2 -> h4 jumps)", () => {
  assert.ok(headingList.length >= 8, `expected a full heading outline, found ${headingList.length} headings`);
  assert.equal(headingList[0].level, 1, `the first heading must be the h1, got h${headingList[0].level}`);
  for (let i = 1; i < headingList.length; i += 1) {
    const prev = headingList[i - 1];
    const curr = headingList[i];
    assert.ok(
      curr.level <= prev.level + 1,
      `heading level skip: <h${prev.level}> "${prev.text}" is followed by <h${curr.level}> "${curr.text}"`
    );
    assert.notEqual(curr.level, prev.level - 2, `heading level skip: <h${prev.level}> "${prev.text}" jumps backward two levels to "${curr.text}"`);
  }
});

/* --------------------------------------------------------- 5. a11y basics */

test("html declares a lang attribute and the title names the product", () => {
  const htmlTag = tagMatches(html, "html")[0];
  assert.ok(htmlTag, "expected an <html> start tag");
  const lang = attr(htmlTag.attrs, "lang");
  assert.match(lang || "", /^[a-z]{2,3}(-[A-Za-z0-9]+)*$/, `<html lang> must be a valid language tag, got ${lang}`);
  const title = html.match(/<title\b[^>]*>([\s\S]*?)<\/title>/i);
  assert.ok(title, "expected a <title> element");
  assert.ok(title[1].trim().length > 0, "<title> must not be empty");
  assert.match(title[1], /OwO/i, `<title> must contain the product name "OwO", got "${title[1].trim()}"`);
});

test("semantic landmarks are present: header, nav, main, footer", () => {
  for (const name of ["header", "nav", "main", "footer"]) {
    assert.ok(tagMatches(html, name).length >= 1, `expected at least one <${name}> landmark`);
  }
  assert.equal(tagMatches(html, "main").length, 1, "expected exactly one <main> landmark");
  const navLabels = tagMatches(html, "nav").map((n) => attr(n.attrs, "aria-label"));
  assert.ok(navLabels.some((l) => l && l.trim().length > 0), "at least one <nav> must carry a non-empty aria-label");
});

test("a skip link is the first focusable element and targets an in-page main anchor", () => {
  const anchors = tagMatches(html, "a");
  const skip = anchors.filter((a) => ["#main", "#hero"].includes(attr(a.attrs, "href") || ""));
  assert.ok(skip.length >= 1, 'expected a skip link with href="#main" or href="#hero"');
  const first = anchors.find((a) => (attr(a.attrs, "href") || "").trim().length > 0);
  assert.equal(first.raw, skip[0].raw, "the skip link must be the first link in the document");
  const target = (attr(skip[0].attrs, "href") || "").slice(1);
  assert.ok(ids(html).has(target), `skip link target #${target} must exist`);
  const skipRule = css.match(/\.skip-link\s*\{[^}]*\}/);
  assert.ok(skipRule, "styles.css must style .skip-link");
  assert.match(skipRule[0], /top:\s*-/, "the skip link must be visually hidden by default (negative top offset)");
  assert.match(css, /\.skip-link:focus\b[^{]*\{[^}]*top:\s*0/, "the skip link must become visible on :focus");
});

test("visible focus styles and reduced-motion handling exist", () => {
  assert.match(css, /:focus-visible\s*\{[^}]*outline\s*:/, "styles.css must define a :focus-visible outline");
  assert.match(css, /@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)/, "styles.css must honour prefers-reduced-motion: reduce");
});

test("code sample is keyboard reachable and shows the documented API", () => {
  const pres = tagMatches(html, "pre");
  assert.ok(pres.length >= 1, "expected a <pre> code sample in the quickstart section");
  assert.ok(pres.some((p) => attr(p.attrs, "tabindex") === "0"), 'the code block should be focusable (tabindex="0") for keyboard scrolling');
  const preMatch = html.match(/<pre\b[^>]*>([\s\S]*?)<\/pre>/i);
  assert.ok(preMatch, "expected a <pre> block to parse");
  const preText = decodeEntities(preMatch[1]);
  assert.match(preText, /owo-agent-sdk/, "the quickstart sample must reference owo-agent-sdk");
  assert.match(preText, /createAgent/, "the quickstart sample must show createAgent()");
  assert.match(preText, /requiresApproval/, "the quickstart sample must show an approval-gated tool");
  assert.match(preText, /\.run\(/, "the quickstart sample must show agent.run()");
});

/* --------------------------------------------------- 6. no dead anchors */

test("every in-page href #fragment resolves to an existing id", () => {
  const known = ids(html);
  const fragments = pickAll(html, /href\s*=\s*["']#([^"']+)["']/g).map((m) => m[1]);
  assert.ok(fragments.length >= 8, `expected several in-page anchors, found ${fragments.length}`);
  for (const frag of fragments) {
    assert.ok(known.has(frag), `dead anchor: href="#${frag}" has no matching id in the page`);
  }
});

/* ------------------------------------------------ 7. responsive / CSS checks */

test("styles.css contains media queries and a reduced-motion block", () => {
  assert.ok(pickAll(css, /@media[^{]+/g).length >= 1, "styles.css must contain at least one @media query");
  assert.match(css, /@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)/, "styles.css must contain @media (prefers-reduced-motion: reduce)");
});

test("responsive layout is defined for compact, medium and wide viewports", () => {
  const minWidths = pickAll(css, /@media[^{]*min-width\s*:\s*(\d+)px/g).map((m) => Number(m[1]));
  const maxWidths = pickAll(css, /@media[^{]*max-width\s*:\s*(\d+)px/g).map((m) => Number(m[1]));
  // Mobile-first base styles cover the compact viewport; optional max-width rules, when present,
  // must cover the narrow range 320-767px.
  assert.ok(
    maxWidths.length === 0 || maxWidths.some((w) => w >= 320 && w <= 767),
    `max-width media queries must cover the compact viewport (320-767px), found: ${maxWidths.join(", ") || "none"}`
  );
  assert.ok(minWidths.some((w) => w >= 768 && w <= 1024), `expected a 768-1024px rule for two-column layouts, found: ${minWidths.join(", ") || "none"}`);
  assert.ok(minWidths.some((w) => w >= 1024), `expected a >=1024px rule for three-column layouts, found: ${minWidths.join(", ") || "none"}`);

  const cardGridBase = css.match(/\.card-grid\s*\{[^}]*\}/);
  assert.ok(cardGridBase, "expected a base .card-grid rule");
  assert.match(cardGridBase[0], /grid-template-columns\s*:\s*1fr/, "the base card grid must be single-column so 320px viewports do not overflow");
});

test("mobile navigation collapses below 768px and becomes a row above it", () => {
  const desktopNavDefault = css.match(/\.site-nav--desktop\s*\{[^}]*\}/);
  assert.ok(desktopNavDefault, "expected a base .site-nav--desktop rule");
  assert.match(desktopNavDefault[0], /display\s*:\s*none/, "the desktop nav must be hidden by default (mobile-first)");
  const wideBlock = css.slice(css.indexOf("min-width: 768px"));
  assert.match(wideBlock, /\.site-nav--desktop\s*\{[^}]*display\s*:\s*block/, "the desktop nav must appear at >=768px");
  assert.match(wideBlock, /\.nav-toggle\s*\{[^}]*display\s*:\s*none/, "the collapsed <details> menu must be hidden at >=768px");
  assert.match(wideBlock, /\.nav-list\s*\{[^}]*flex-direction\s*:\s*row/, "the desktop nav list must lay out horizontally");
});

test("no horizontal overflow hazards: border-box sizing and scroll containers", () => {
  assert.match(css, /\*\s*,[\s\S]{0,200}box-sizing\s*:\s*border-box/, "expected a global box-sizing: border-box reset");
  const scrollers = pickAll(css, /overflow-x\s*:\s*auto/g).length;
  assert.ok(scrollers >= 2, `expected both the code block and the comparison table to scroll horizontally, found ${scrollers} overflow-x:auto rules`);
  assert.match(css, /\.table-scroll\s*\{[^}]*overflow-x\s*:\s*auto/, "expected a .table-scroll wrapper with overflow-x: auto");
  assert.match(css, /\.code-card pre\s*\{[^}]*overflow-x\s*:\s*auto/, "expected the code block to scroll instead of widening the page");
  assert.match(css, /max-width\s*:\s*100%/, "expected a max-width: 100% guard on wide media");
});

test("typography meets the brief minimums", () => {
  assert.match(css, /font-size\s*:\s*clamp\([^)]*rem/, "expected fluid (clamp) display typography");
  assert.match(css, /font-size\s*:\s*1\.0625rem/, "body copy should be ~17px per the brief (16-18px)");
  assert.match(css, /line-height\s*:\s*(1\.[5-9]|[2-9])/, "body line-height must be >= 1.5");
  const bodyRule = css.match(/\nbody\s*\{[^}]*\}/);
  assert.ok(bodyRule, "expected a body rule");
  const codePre = css.match(/\.code-card pre\s*\{[^}]*\}/);
  assert.match(codePre[0], /font-size\s*:\s*0\.875rem/, "code blocks must be >=14px (0.875rem) per the brief");
  assert.match(css, /--font-mono|ui-monospace|monospace/, "expected a monospace font stack for code");
  assert.match(css, /system-ui/, "expected a system font stack for body copy");
});

/* ------------------------------------------------------ 8. content checks */

test("no placeholder junk appears in visible copy", () => {
  // Brief 7.5 bans placeholder junk generally, so the bare word "placeholder"
  // (e.g. the earlier "repository placeholder" footer note) must fail here too
  // instead of slipping past a narrower "placeholder text" phrase match.
  const banned = [/lorem ipsum/i, /\bTODO\b/, /\bFIXME\b/, /\bxxx\b/i, /\bTBD\b/i, /\bplaceholder\b/i, /\bcoming soon\b/i];
  for (const re of banned) assert.doesNotMatch(htmlText, re, `visible copy must not contain placeholder junk matching ${re}`);
});

test("core messages are carried by their own visible sections", () => {
  const requiredCopy = [
    "Build agents that ship",
    "Tool calling",
    "Context management",
    "Approval & permissions",
    "Test harness",
    "How it works",
    "Quickstart",
    "FAQ"
  ];
  for (const phrase of requiredCopy) {
    assert.ok(htmlText.includes(phrase), `expected visible copy to contain "${phrase}"`);
  }
  const h2Texts = headingList.filter((h) => h.level === 2).map((h) => h.text);
  assert.ok(h2Texts.length >= 5, `expected at least 5 section <h2> headings, found ${h2Texts.length}`);
});

test("FAQ uses native details/summary and answers at least 4 questions", () => {
  const details = tagMatches(html, "details");
  assert.ok(details.length >= 5, `expected the mobile menu plus >=4 FAQ entries as <details>, found ${details.length}`);
  const summaries = pickAll(html, /<summary\b[^>]*>([\s\S]*?)<\/summary>/gi).map((m) => visibleText(m[1]));
  assert.ok(summaries.length >= 5, `expected >=5 <summary> elements (menu + FAQ), found ${summaries.length}`);
  const questions = summaries.filter((s) => s.endsWith("?"));
  assert.ok(questions.length >= 4, `expected at least 4 FAQ questions, found ${questions.length} (${questions.join(" | ")})`);
  const faqStart = html.search(/id\s*=\s*["']faq["']/);
  assert.ok(faqStart > -1, 'expected an element with id="faq"');
  const faqBlock = html.slice(faqStart, faqStart + 12000);
  assert.match(faqBlock, /<h2\b/, "the FAQ section must be introduced by an <h2>");
});

test("comparison table is semantic with a caption and header scopes", () => {
  assert.ok(tagMatches(html, "table").length >= 1, "expected a semantic <table> for the capability comparison");
  const start = html.indexOf("<table");
  const end = html.indexOf("</table>");
  assert.ok(start > -1 && end > start, "expected a complete <table> block");
  const tableBlock = html.slice(start, end);
  assert.match(tableBlock, /<caption\b[^>]*>[\s\S]{5,}?<\/caption>/, "the table must have a non-empty <caption>");
  const ths = tagMatches(tableBlock, "th");
  const colHeaders = ths.filter((t) => attr(t.attrs, "scope") === "col");
  const rowHeaders = ths.filter((t) => attr(t.attrs, "scope") === "row");
  assert.equal(colHeaders.length, 4, `expected 4 column headers (th scope="col"), found ${colHeaders.length}`);
  assert.ok(rowHeaders.length >= 4, `expected >=4 row headers (th scope="row"), found ${rowHeaders.length}`);
});

test("primary CTA copy is reused for the final CTA and every CTA navigates in-page", () => {
  const ctas = pickAll(html, /<a\b([^>]*class="[^"]*\bbtn\b[^"]*"[^>]*)>([\s\S]*?)<\/a>/gi).map((m) => ({
    href: attr(m[1], "href"),
    label: visibleText(m[2])
  }));
  assert.ok(ctas.length >= 3, `expected the hero pair plus header/final CTAs, found ${ctas.length}`);
  const getStarted = ctas.filter((c) => c.label === "Get started");
  assert.ok(getStarted.length >= 2, 'expected at least two "Get started" buttons (header/hero and final CTA)');
  assert.ok(ctas.some((c) => c.label === "Read the docs"), 'expected a secondary "Read the docs" CTA');
  for (const cta of ctas) {
    assert.ok((cta.href || "").startsWith("#"), `CTA "${cta.label}" must point at an in-page anchor, got "${cta.href}"`);
  }
});

test("footer carries docs, license and an accessibility statement", () => {
  const footerStart = html.search(/<footer\b/);
  assert.ok(footerStart > -1, "expected a <footer> element");
  const footerBlock = html.slice(footerStart);
  const footerText = visibleText(footerBlock);
  assert.match(footerText, /accessibility/i, "the footer must include an accessibility statement");
  assert.match(footerText, /MIT/i, "the footer must state the license (MIT)");
  assert.match(footerText, /Docs|Documentation/i, "the footer must link to docs");
});

test("footer names the repository literally instead of announcing a placeholder", () => {
  const footerStart = html.search(/<footer\b/);
  assert.ok(footerStart > -1, "expected a <footer> element");
  const footerBlock = html.slice(footerStart);
  const footerText = visibleText(footerBlock);
  // Regression guard for the "repository placeholder" wording: the brief expects a
  // concrete, non-junk note, so the repo must be named literally.
  assert.match(footerText, /owo-agent-sdk/i, "the footer must name the package/repository literally (owo-agent-sdk)");
  assert.match(footerText, /GitHub/i, "the footer must reference GitHub");
  assert.doesNotMatch(footerText, /\bplaceholder\b/i, "the footer must not describe itself as a placeholder");
  assert.doesNotMatch(footerText, /\bcoming soon\b/i, "the footer must not use an upcoming/coming-soon placeholder note");
  // The repo is named as plain text: no remote href, so the page stays offline-capable.
  const externalHref = pickAll(footerBlock, /href\s*=\s*["'](https?:)?\/\/[^"']+["']/gi).map((m) => m[0]);
  assert.deepEqual(externalHref, [], `footer links must stay in-page, found: ${externalHref.join(", ")}`);
});

/* ------------------------------------------- 9. no remote resource deps */

test("page loads no remote resources (offline-capable)", () => {
  const resourceTags = ["script", "link", "img", "iframe", "source", "video", "audio", "use", "object", "embed"].flatMap((name) => tagMatches(html, name));
  for (const t of resourceTags) {
    for (const name of ["src", "href", "xlink:href", "data"]) {
      const value = attr(t.attrs, name);
      if (value == null) continue;
      assert.ok(!/^(https?:)?\/\//i.test(value), `remote resource reference blocks offline use: ${t.raw}`);
    }
  }
  const externalNav = pickAll(html, /href\s*=\s*["'](https?:)?\/\/[^"']+["']/gi).map((m) => m[0]);
  assert.deepEqual(externalNav, [], `no external navigation links are whitelisted, found: ${externalNav.join(", ")}`);
  assert.doesNotMatch(css, /@import\s+url\(\s*["']?(https?:)?\/\//i, "styles.css must not @import a remote stylesheet");
  assert.doesNotMatch(css, /url\(\s*["']?(https?:)?\/\//i, "styles.css must not load remote url() assets");
  assert.doesNotMatch(html, /<script\b/i, "no <script> is expected: core content must be visible without JS");
});
