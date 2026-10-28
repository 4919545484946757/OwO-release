import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { nextExpandedState, initNav, initFaqAccordion } from "../script.js";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const read = (relPath) => readFileSync(resolve(root, relPath), "utf8");

const html = read("index.html");
const css = read("styles.css");
const js = read("script.js");

/* ------------------------------------------------------------------ */
/* Small helpers (regex based, no third-party parser needed)           */
/* ------------------------------------------------------------------ */

const stripTags = (markup) => markup.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();

const matchAll = (source, regex) => Array.from(source.matchAll(regex));

const ids = () => new Set(matchAll(html, /\sid="([^"]+)"/g).map((m) => m[1]));

const openTags = (name) =>
  matchAll(html, new RegExp(`<${name}(\\s|>)`, "gi")).filter(
    (m) => !m[0].endsWith("/>"),
  ).length;

const closeTags = (name) => matchAll(html, new RegExp(`</${name}>`, "gi")).length;

const tagText = (name) =>
  matchAll(html, new RegExp(`<${name}\\b[^>]*>([\\s\\S]*?)</${name}>`, "gi")).map((m) =>
    stripTags(m[1]),
  );

/* ------------------------------------------------------------------ */
/* Deliverable integrity                                              */
/* ------------------------------------------------------------------ */

test("all three deliverable files exist and are non-trivial", () => {
  for (const file of ["index.html", "styles.css", "script.js"]) {
    const stats = statSync(resolve(root, file));
    assert.ok(stats.isFile(), `${file} must exist`);
    assert.ok(stats.size > 1500, `${file} must contain real content (got ${stats.size}B)`);
  }
});

test("styles.css and script.js are wired from the HTML and exist on disk", () => {
  const cssHref = html.match(/<link[^>]+rel="stylesheet"[^>]+href="([^"]+)"/i);
  assert.ok(cssHref, "index.html must link a stylesheet");
  assert.equal(cssHref[1], "styles.css");
  assert.ok(statSync(resolve(root, cssHref[1])).isFile());

  const scriptSrc = html.match(/<script[^>]+src="([^"]+)"/i);
  assert.ok(scriptSrc, "index.html must load a script");
  assert.equal(scriptSrc[1], "script.js");
  assert.ok(statSync(resolve(root, scriptSrc[1])).isFile());
});

/* ------------------------------------------------------------------ */
/* Document metadata                                                  */
/* ------------------------------------------------------------------ */

test("document has a doctype, language, charset, viewport and title", () => {
  assert.match(html.trimStart(), /^<!DOCTYPE html>/i);
  assert.match(html, /<html[^>]+lang="en"/i);
  assert.match(html, /<meta[^>]+charset="utf-8"/i);
  assert.match(html, /<meta[^>]+name="viewport"[^>]+content="width=device-width, initial-scale=1"/i);
  const title = html.match(/<title>([^<]+)<\/title>/i);
  assert.ok(title, "a <title> is required");
  assert.match(title[1], /OwO Agent SDK/);
  assert.ok(title[1].length <= 70, "title should stay readable in search results");
});

test("page declares a meta description and a theme colour", () => {
  const description = html.match(/<meta[^>]+name="description"[^>]+content="([^"]+)"/i);
  assert.ok(description, "description meta tag is required");
  assert.ok(description[1].length > 60, "description should be meaningful");
  assert.match(html, /<meta[^>]+name="theme-color"/i);
});

/* ------------------------------------------------------------------ */
/* Structure & landmarks                                              */
/* ------------------------------------------------------------------ */

test("landmark structure is present exactly once where it matters", () => {
  assert.equal(openTags("main"), 1, "exactly one <main>");
  assert.match(html, /<main[^>]+id="main"/i);
  assert.equal(openTags("header") >= 1, true);
  assert.equal(openTags("footer"), 1, "exactly one <footer>");
  assert.equal(openTags("h1"), 1, "exactly one <h1>");
  assert.ok(openTags("nav") >= 2, "primary and footer navigation");
});

test("every navigation landmark is labelled", () => {
  const navs = matchAll(html, /<nav\b[^>]*>/gi).map((m) => m[0]);
  assert.ok(navs.length >= 2);
  for (const nav of navs) {
    assert.match(nav, /aria-label="[^"]+"/, `unlabelled nav: ${nav}`);
  }
});

test("sections are named via aria-labelledby pointing at real headings", () => {
  const idSet = ids();
  const sections = matchAll(html, /<section\b[^>]*>/gi).map((m) => m[0]);
  assert.ok(sections.length >= 5, "the page should have multiple content sections");
  for (const section of sections) {
    const label = section.match(/aria-labelledby="([^"]+)"/);
    assert.ok(label, `section without aria-labelledby: ${section}`);
    assert.ok(idSet.has(label[1]), `aria-labelledby target #${label[1]} does not exist`);
  }
});

test("heading levels never skip a level", () => {
  const levels = matchAll(html, /<h([1-6])\b/gi).map((m) => Number(m[1]));
  assert.ok(levels.length > 10, "expected a rich heading outline");
  assert.equal(levels[0], 1);
  for (let i = 1; i < levels.length; i += 1) {
    const jump = levels[i] - levels[i - 1];
    assert.ok(jump <= 1, `heading jumped from h${levels[i - 1]} to h${levels[i]}`);
  }
});

test("required landing-page sections exist with stable ids", () => {
  const idSet = ids();
  for (const id of ["top", "features", "quickstart", "proof", "pricing", "faq", "main"]) {
    assert.ok(idSet.has(id), `missing section id #${id}`);
  }
});

test("HTML container tags are balanced", () => {
  for (const tag of ["html", "head", "body", "header", "main", "footer", "section", "nav", "ul", "li", "div", "article", "details", "pre", "code"]) {
    assert.equal(openTags(tag), closeTags(tag), `<${tag}> open/close mismatch`);
  }
});

/* ------------------------------------------------------------------ */
/* Accessibility specifics                                            */
/* ------------------------------------------------------------------ */

test("a skip link is the first focusable element and targets main", () => {
  const skip = html.match(/<a class="skip-link" href="#([^"]+)">([^<]+)<\/a>/i);
  assert.ok(skip, "skip link is required");
  assert.equal(skip[1], "main");
  assert.match(skip[2], /Skip to main content/i);
});

test("the mobile nav toggle exposes state and exposes a text alternative", () => {
  const button = html.match(/<button[^>]*data-nav-toggle[^>]*>[\s\S]*?<\/button>/i);
  assert.ok(button, "nav toggle button is required");
  assert.match(button[0], /type="button"/);
  assert.match(button[0], /aria-expanded="false"/);
  const controls = button[0].match(/aria-controls="([^"]+)"/);
  assert.ok(controls, "toggle must reference the nav it controls");
  assert.equal(controls[1], "primary-navigation");
  assert.ok(ids().has(controls[1]));
  assert.match(button[0], /visually-hidden">[^<]*nav[^<]*<\/span>/i);
});

test("every internal anchor resolves to an existing id", () => {
  const idSet = ids();
  const hrefs = matchAll(html, /href="#([^"]+)"/g).map((m) => m[1]);
  assert.ok(hrefs.length >= 8, "expected several in-page links");
  for (const href of hrefs) {
    assert.ok(idSet.has(href), `dangling in-page link: #${href}`);
  }
});

test("aria-* id references all resolve", () => {
  const idSet = ids();
  const refs = [
    ...matchAll(html, /aria-labelledby="([^"]+)"/g),
    ...matchAll(html, /aria-controls="([^"]+)"/g),
    ...matchAll(html, /aria-describedby="([^"]+)"/g),
  ].flatMap((m) => m[1].split(/\s+/));
  assert.ok(refs.length > 0);
  for (const ref of refs) {
    assert.ok(idSet.has(ref), `aria reference to #${ref} has no matching id`);
  }
});

test("links and buttons all have discernible text", () => {
  const anchors = matchAll(html, /<a\b[^>]*>([\s\S]*?)<\/a>/gi);
  assert.ok(anchors.length >= 10);
  for (const anchor of anchors) {
    assert.ok(stripTags(anchor[1]).length > 0, `link without text: ${anchor[0]}`);
  }

  const buttons = matchAll(html, /<button\b[^>]*>([\s\S]*?)<\/button>/gi);
  for (const button of buttons) {
    assert.ok(stripTags(button[1]).length > 0, "button without an accessible name");
  }
});

test("purely decorative glyphs are hidden from assistive technology", () => {
  const decorative = matchAll(html, /<span class="(?:brand-mark|card-icon|dot dot-[a-z]+|step-number)"[^>]*>/gi);
  assert.ok(decorative.length >= 8);
  for (const span of decorative) {
    assert.match(span[0], /aria-hidden="true"/, `decorative element not hidden: ${span[0]}`);
  }
});

test("no <img> ships without an alt attribute", () => {
  const images = matchAll(html, /<img\b[^>]*>/gi);
  for (const image of images) {
    assert.match(image[0], /\balt="/, `image without alt: ${image[0]}`);
  }
});

test("code samples are keyboard reachable and labelled", () => {
  const preBlocks = matchAll(html, /<pre\b[^>]*>/gi).map((m) => m[0]);
  assert.ok(preBlocks.length >= 4, "the page shows real code samples");
  for (const block of preBlocks) {
    assert.match(block, /tabindex="0"/, "scrollable code needs a focus stop");
    assert.match(block, /aria-label="[^"]+"/, "code block needs an accessible name");
  }
});

test("the FAQ uses native disclosure widgets with a summary per item", () => {
  const details = openTags("details");
  const summaries = openTags("summary");
  assert.ok(details >= 4, "expected several FAQ entries");
  assert.equal(details, summaries, "each <details> needs exactly one <summary>");
  assert.equal(matchAll(html, /class="faq-item"/g).length, details);
});

test("pricing plans are articles named by their own headings", () => {
  const idSet = ids();
  const plans = matchAll(html, /<article\b[^>]*class="plan[^"]*"[^>]*>/gi).map((m) => m[0]);
  assert.equal(plans.length, 3, "three pricing tiers");
  for (const plan of plans) {
    const label = plan.match(/aria-labelledby="([^"]+)"/);
    assert.ok(label, `pricing plan without aria-labelledby: ${plan}`);
    assert.ok(idSet.has(label[1]));
  }
});

/* ------------------------------------------------------------------ */
/* Copy quality                                                       */
/* ------------------------------------------------------------------ */

test("no lorem ipsum or leftover placeholders remain", () => {
  const body = stripTags(html);
  for (const word of ["lorem ipsum", "TODO", "TBD", "FIXME", "PLACEHOLDER", "coming soon"]) {
    assert.ok(
      !body.toLowerCase().includes(word.toLowerCase()),
      `placeholder copy found: ${word}`,
    );
  }
});

test("the hero communicates the product and a primary action", () => {
  const h1 = html.match(/<h1[^>]*>([\s\S]*?)<\/h1>/i);
  assert.ok(h1);
  assert.ok(stripTags(h1[1]).length >= 20, "hero headline should be descriptive");
  assert.match(html, /class="[^"]*button-primary[^"]*"[^>]*href="#quickstart"/i);
});

/* ------------------------------------------------------------------ */
/* Responsive CSS                                                     */
/* ------------------------------------------------------------------ */

test("styles.css is syntactically balanced", () => {
  const opens = (css.match(/{/g) || []).length;
  const closes = (css.match(/}/g) || []).length;
  assert.ok(opens > 40, "stylesheet should contain real rules");
  assert.equal(opens, closes, "unbalanced braces in styles.css");
});

test("layout is responsive at the documented breakpoints", () => {
  assert.match(css, /@media \(min-width: 640px\)/);
  assert.match(css, /@media \(min-width: 900px\)/);
  assert.match(css, /grid-template-columns: repeat\(3, minmax\(0, 1fr\)\)/);
  assert.match(css, /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/);
  assert.match(css, /clamp\(/, "fluid type keeps headings readable on small screens");
  assert.match(css, /width: min\(100% - 2\.5rem, var\(--container\)\)/);
});

test("styles.css includes accessibility affordances", () => {
  assert.match(css, /:focus-visible/);
  assert.match(css, /\.visually-hidden\s*{/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\)/);
  assert.match(css, /@media \(prefers-color-scheme: dark\)/);
  assert.match(css, /\.skip-link:focus/);
  assert.match(css, /box-sizing: border-box/);
});

/* ------------------------------------------------------------------ */
/* Behaviour unit tests                                               */
/* ------------------------------------------------------------------ */

test("nextExpandedState toggles and honours an explicit target", () => {
  assert.equal(nextExpandedState(false), true);
  assert.equal(nextExpandedState(true), false);
  assert.equal(nextExpandedState(false, true), true);
  assert.equal(nextExpandedState(true, false), false);
});

test("script.js drives aria-expanded rather than visual classes alone", () => {
  assert.match(js, /export function initNav/);
  assert.match(js, /setAttribute\("aria-expanded", String\(expanded\)\)/);
  assert.match(js, /classList\.toggle\("is-open", expanded\)/);
  assert.match(js, /event\.key === "Escape"/);
  assert.match(js, /export function initFaqAccordion/);
});

test("initNav returns null when the toggle markup is missing", () => {
  const fakeDoc = { querySelector: () => null };
  assert.equal(initNav(fakeDoc), null);
  assert.equal(initFaqAccordion({ querySelectorAll: () => [] }), undefined);
});

test("initNav wires a working toggle on a minimal fake DOM", () => {
  const listeners = new Map();
  const attributes = new Map([["aria-expanded", "false"]]);
  const classes = new Set();

  const button = {
    addEventListener: (type, fn) => listeners.set(`button:${type}`, fn),
    removeEventListener: () => {},
    setAttribute: (name, value) => attributes.set(name, value),
    getAttribute: (name) => attributes.get(name) ?? null,
    focus: () => {},
  };
  const nav = {
    classList: {
      toggle: (name, on) => (on ? classes.add(name) : classes.delete(name)),
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  const doc = {
    defaultView: { HTMLElement: class HTMLElement {} },
    querySelector: (selector) =>
      selector === "[data-nav-toggle]" ? button : selector === "[data-nav]" ? nav : null,
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  const controller = initNav(doc);
  assert.ok(controller);
  assert.equal(attributes.get("aria-expanded"), "false");

  assert.equal(controller.toggle(), true);
  assert.equal(attributes.get("aria-expanded"), "true");
  assert.equal(classes.has("is-open"), true);

  assert.equal(controller.toggle(false), false);
  assert.equal(attributes.get("aria-expanded"), "false");
  assert.equal(classes.has("is-open"), false);
});
