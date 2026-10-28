// Read-only report integrity and browser-logic checks. Not a visual browser test.
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert/strict');

async function main() {
  const directory = path.resolve(process.argv[2] || 'docs/quality-review/2026-10-29');
  const html = fs.readFileSync(path.join(directory, 'issues.html'), 'utf8');
  const scripts = [...html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map(x => x[1]);
  assert.equal(scripts.length, 2);
  const payload = JSON.parse(scripts[0]);
  const summary = JSON.parse(fs.readFileSync(path.join(directory, 'summary.json'), 'utf8'));
  const issues = fs.readFileSync(path.join(directory, 'issues.jsonl'), 'utf8').trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(issues.length, summary.issue_count);
  assert.deepEqual(payload.summary, summary);
  assert.deepEqual(payload.issues, issues);
  assert.equal(new Set(issues.map(x => x.id)).size, issues.length);
  assert.equal(summary.rounds.length, summary.runs);
  assert.equal(summary.rounds.reduce((n, r) => n + r.metrics.cases, 0), summary.case_observations);
  for (const issue of issues) {
    assert.ok(issue.id && issue.input && issue.observations.length);
    assert.equal(issue.status, '待批阅');
    for (const observation of issue.observations) {
      assert.ok(['base', 'current', 'release-legacy', 'explicit-other'].includes(observation.runtime));
      assert.ok(['on', 'off'].includes(observation.correction));
      assert.ok(fs.existsSync(path.join(observation.report, 'cases.jsonl')));
    }
  }

  const elements = new Map();
  let clicked = null, download = null, alertText = '';
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.value = ''; this.textContent = ''; this.events = {}; }
    set id(value) { this._id = value; elements.set(value, this); }
    get id() { return this._id; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = [...children]; }
    after(child) { this.afterElement = child; }
    addEventListener(event, handler) { this.events[event] = handler; }
    click() { clicked = this; }
  }
  for (const match of html.matchAll(/\bid="([^"]+)"/g)) { const e = new Element('div'); e.id = match[1]; }
  elements.get('data').textContent = scripts[0];
  elements.get('runtime').value = 'active';
  elements.get('size').value = '50';
  const store = new Map();
  const context = vm.createContext({
    document: { getElementById: id => { assert.ok(elements.has(id), `Missing element ${id}`); return elements.get(id); }, createElement: tag => new Element(tag) },
    localStorage: { getItem: key => store.get(key) || null, setItem: (key, value) => store.set(key, value) },
    Blob, URL: { createObjectURL: blob => { download = blob; return 'blob:unit-test'; }, revokeObjectURL() {} },
    setTimeout: fn => fn(), alert: text => { alertText = text; }
  });
  new vm.Script(scripts[1]).runInContext(context);
  const evaluate = text => vm.runInContext(text, context);
  const filtered = () => evaluate('filtered()');
  const change = (id, value) => { const e = elements.get(id); e.value = value; e.events[id === 'search' ? 'input' : 'change'](); };
  assert.ok(filtered().length > 0);
  assert.equal(elements.get('rows').children.length, Math.min(50, filtered().length));
  assert.ok(filtered().every(i => i.observations.some(o => o.runtime === 'base' || o.runtime === 'current')));
  for (const category of ['full', 'mixed', 'context', 'typo', 'prefix_latency_probe']) {
    change('category', category);
    assert.ok(filtered().length > 0, `Empty expected category ${category}`);
    assert.ok(filtered().every(i => i.tags.includes(category)));
  }
  change('category', ''); change('runtime', 'release-legacy');
  assert.ok(filtered().length > 0);
  assert.ok(filtered().every(i => i.observations.some(o => o.runtime === 'release-legacy')));
  elements.get('reset').onclick();
  const searchable = filtered()[0];
  change('search', searchable.input);
  assert.ok(filtered().some(issue => issue.id === searchable.id));
  elements.get('reset').onclick();
  const first = filtered()[0];
  const reviewCell = elements.get('rows').children[0].children[3];
  const select = reviewCell.children[0], notes = reviewCell.children[1];
  select.value = '需要复现'; notes.value = 'temporary checker state, not a human decision'; select.onchange();
  assert.ok(store.size);
  elements.get('save').onclick();
  assert.equal(clicked.download, 'owo-quality-human-review.json');
  const saved = JSON.parse(await download.text());
  assert.equal(saved.reviews.find(i => i.id === first.id).status, '需要复现');
  const file = elements.get('file');
  file.files = [{ size: 100, text: async () => JSON.stringify({ schema_version: 1, reviews: [{ id: first.id, status: '不复现', notes: 'temporary import test' }] }) }];
  await file.onchange();
  assert.ok(alertText.includes('1'));
  assert.equal(evaluate(`state(data.issues.find(i=>i.id===${JSON.stringify(first.id)})).status`), '不复现');
  file.files = [{ size: 20, text: async () => '{"schema_version":2}' }];
  await file.onchange(); assert.ok(alertText.startsWith('导入失败'));
  console.log(JSON.stringify({ ok: true, runs: summary.runs, observations: summary.case_observations, issues: issues.length, checks: ['jsonl/html consistency', 'unique IDs', 'raw evidence exists', 'runtime/category filters', 'search', 'pagination row count', 'review export/import', 'invalid import rejected'], scope: 'mock DOM logic only; no pixel/layout or real browser verification; no files/reviews written' }));
}
main().catch(error => { console.error(String(error.stack).slice(0, 1800)); process.exitCode = 1; });
