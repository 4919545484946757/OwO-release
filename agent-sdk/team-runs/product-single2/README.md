# OwO Agent SDK — product landing page

A single-page, dependency-free marketing site for the fictional **OwO Agent SDK**
product, plus a `node:test` suite that verifies the page's structure,
accessibility affordances and responsive CSS.

## Files

| Path | Purpose |
| --- | --- |
| `index.html` | Semantic single-page landing markup: hero, features, quickstart, proof, pricing, FAQ, CTA, footer. |
| `styles.css` | Mobile-first stylesheet with 640 px / 900 px breakpoints, automatic dark mode, reduced-motion support. |
| `script.js` | Progressive enhancement only: mobile nav toggle (`aria-expanded`) and FAQ accordion. |
| `test/landing.test.mjs` | 29 `node:test` cases covering markup, a11y, CSS and the JS behaviour. |
| `scripts/serve.mjs` | Optional zero-dependency static server for local review. |

## Run the tests

```bash
npm test          # node --test test/landing.test.mjs
```

Requires Node.js 18 or newer (verified on Node 22.17.0). No dependencies are
installed: the suite parses the shipped files directly and unit-tests
`script.js` with a minimal fake DOM, so it runs offline.

## Preview locally

```bash
npm run serve     # http://localhost:4173
```

## Accessibility notes

- Skip link to `#main` is the first focusable element.
- Global `:focus-visible` outline; all interactive controls are keyboard reachable.
- One `h1`, no skipped heading levels; every `<section>` is named via
  `aria-labelledby` pointing at a real heading id.
- Decorative glyphs are `aria-hidden="true"`; the nav toggle ships a
  `visually-hidden` text label and exposes state through `aria-expanded`.
- Native `<details>/<summary>` disclosures for the FAQ; code blocks are
  scrollable and labelled.
- Respects `prefers-reduced-motion` and `prefers-color-scheme: dark`.

## Notes

The product copy is illustrative sample content for the landing page itself;
no analytics, cookies or third-party requests are used.
