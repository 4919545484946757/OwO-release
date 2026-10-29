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
  let clicked = null, download = null, alertText = '', activeElement = null;
  const documentEvents = {};
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.value = ''; this.textContent = ''; this.events = {}; this.style = {}; }
    set id(value) { this._id = value; elements.set(value, this); }
    get id() { return this._id; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = [...children]; }
    after(child) { this.afterElement = child; }
    addEventListener(event, handler) { this.events[event] = handler; }
    click() { clicked = this; }
    setAttribute(name, value) { (this.attributes ||= {})[name] = value; }
    focus() { activeElement = this; }
    scrollIntoView() { this.scrolled = true; }
    closest(selector) {
      return selector.split(',').some(part => part === this.tagName.toLowerCase() || (part === '[contenteditable]' && this.isContentEditable)) ? this : null;
    }
    get classList() {
      const element = this;
      return { toggle(name, enabled) { const values = new Set((element.className || '').split(' ').filter(Boolean)); if (enabled) values.add(name); else values.delete(name); element.className = [...values].join(' '); }, contains(name) { return (element.className || '').split(' ').includes(name); } };
    }
  }
  for (const match of html.matchAll(/\bid="([^"]+)"/g)) { const e = new Element('div'); e.id = match[1]; }
  elements.get('data').textContent = scripts[0];
  elements.get('runtime').value = 'active';
  elements.get('size').value = '50';
  const store = new Map();
  const context = vm.createContext({
    document: { getElementById: id => { assert.ok(elements.has(id), `Missing element ${id}`); return elements.get(id); }, createElement: tag => new Element(tag), get activeElement() { return activeElement; }, addEventListener: (event,handler) => { documentEvents[event] = handler; } },
    localStorage: { getItem: key => store.get(key) || null, setItem: (key, value) => store.set(key, value) },
    Blob, URL: { createObjectURL: blob => { download = blob; return 'blob:unit-test'; }, revokeObjectURL() {} },
    setTimeout: fn => fn(), alert: text => { alertText = text; }
  });
  new vm.Script(scripts[1]).runInContext(context);
  const evaluate = text => vm.runInContext(text, context);
  const filtered = () => evaluate('filtered()');
  const change = (id, value) => { const e = elements.get(id); e.value = value; e.events[id === 'search' ? 'input' : 'change'](); };
  const key = (value, extra = {}) => { const event = { key:value, target:new Element('body'), preventDefault() { this.prevented = true; }, ...extra }; documentEvents.keydown(event); return event; };
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
  const reviewCell = elements.get('rows').children[0].children[4];
  const select = reviewCell.children[0], notes = reviewCell.children[1];
  select.value = '需要复现'; notes.value = 'temporary checker state, not a human decision'; notes.oninput(); select.onchange();
  assert.ok(store.size);
  elements.get('save').onclick();
  assert.equal(clicked.download, 'owo-quality-human-review.json');
  const saved = JSON.parse(await download.text());
  assert.equal(saved.reviews.find(i => i.id === first.id).status, '需要复现');
  assert.equal(saved.reviews.find(i => i.id === first.id).notes, notes.value);

  // Keyboard selection, shift-range growth/shrinkage and cross-page movement.
  change('size','25');
  key('ArrowDown'); assert.equal(evaluate('activeId'), filtered()[0].id);
  key('ArrowDown',{shiftKey:true}); assert.equal(evaluate('selected.size'),2);
  key('ArrowUp',{shiftKey:true}); assert.equal(evaluate('selected.size'),1);
  evaluate('selectIssue(filtered()[24].id)'); key('ArrowDown',{shiftKey:true});
  assert.equal(evaluate('page'),1); assert.equal(evaluate('selected.size'),2);
  assert.ok(evaluate('visibleRows.get(activeId).tr.scrolled'));
  key('ArrowLeft'); assert.equal(evaluate('page'),0); assert.equal(evaluate('selected.size'),1);
  key('ArrowRight'); assert.equal(evaluate('page'),1); assert.equal(evaluate('activeId'),filtered()[25].id);
  key('ArrowLeft');

  // Selected batch vs a separately hovered row; resetting status preserves notes.
  const ids = filtered().slice(0,3).map(issue=>issue.id);
  evaluate(`selectIssue(${JSON.stringify(ids[0])});selectIssue(${JSON.stringify(ids[1])},true)`);
  evaluate(`visibleRows.get(${JSON.stringify(ids[2])}).tr.onpointermove()`);
  assert.ok(key(' ').prevented);
  assert.equal(evaluate(`state(byId.get(${JSON.stringify(ids[2])})).status`),'确认问题');
  assert.notEqual(evaluate(`state(byId.get(${JSON.stringify(ids[1])})).status`),'确认问题');
  evaluate(`visibleRows.get(${JSON.stringify(ids[0])}).tr.onpointermove()`); key('z');
  for(const id of ids.slice(0,2)) assert.equal(evaluate(`state(byId.get(${JSON.stringify(id)})).status`),'可接受歧义');
  const beforeNotes = evaluate(`state(byId.get(${JSON.stringify(ids[0])})).notes`);
  key('Backspace');
  for(const id of ids.slice(0,2)) assert.equal(evaluate(`state(byId.get(${JSON.stringify(id)})).status`),'待批阅');
  assert.equal(evaluate(`state(byId.get(${JSON.stringify(ids[0])})).notes`),beforeNotes);
  key('z',{ctrlKey:true}); assert.equal(evaluate(`state(byId.get(${JSON.stringify(ids[0])})).status`),'可接受歧义');
  const undoCount = evaluate('undoStack.length'); key(' ',{repeat:true}); assert.equal(evaluate('undoStack.length'),undoCount);
  assert.ok(!key(' ',{target:new Element('textarea')}).prevented);
  assert.ok(!key('Backspace',{target:new Element('input')}).prevented);
  assert.ok(!key('z',{isComposing:true}).prevented);
  assert.ok(!key('ArrowRight',{altKey:true}).prevented);
  assert.ok(!key('ArrowDown',{target:new Element('select')}).prevented);
  // Arrow navigation clears stale hover; subsequent review targets the new selection.
  key('ArrowDown'); assert.equal(evaluate('hoveredId'),null);
  assert.ok(key('ArrowDown',{target:new Element('button')}).prevented);

  elements.get('bulk-status').value='需要复现'; elements.get('bulk-notes').value='batch note'; elements.get('notes-mode').value='append';
  elements.get('select-page').onclick(); assert.equal(evaluate('selected.size'),25);
  elements.get('bulk-apply').onclick();
  assert.ok(evaluate("[...selected].every(id=>state(byId.get(id)).status==='需要复现'&&state(byId.get(id)).notes.includes('batch note'))"));
  assert.ok(evaluate("visibleRows.get(activeId).notes.value.includes('batch note')"));
  elements.get('notes-mode').value='replace'; elements.get('bulk-notes').value='replacement'; elements.get('bulk-status').value='';
  elements.get('bulk-apply').onclick(); assert.ok(evaluate("[...selected].every(id=>state(byId.get(id)).notes==='replacement')"));
  elements.get('undo').onclick(); assert.ok(evaluate("[...selected].every(id=>state(byId.get(id)).notes.includes('batch note'))"));
  evaluate("applyReview([...selected],'确认问题')");
  const laterNotes = evaluate('visibleRows.get(activeId).notes'); laterNotes.value='later manual note'; laterNotes.oninput();
  elements.get('undo').onclick(); assert.equal(evaluate('state(byId.get(activeId)).notes'),'later manual note');
  change('status','需要复现'); elements.get('select-filtered').onclick();
  assert.equal(evaluate('selected.size'),filtered().length);
  elements.get('bulk-status').value='确认问题'; elements.get('bulk-notes').value=''; elements.get('bulk-apply').onclick();
  assert.equal(filtered().length,0); assert.equal(evaluate('selected.size'),0);
  elements.get('reset').onclick();
  const file = elements.get('file');
  file.files = [{ size: 100, text: async () => JSON.stringify({ schema_version: 1, reviews: [{ id: first.id, status: '不复现', notes: 'temporary import test' }] }) }];
  await file.onchange();
  assert.ok(alertText.includes('1'));
  assert.equal(evaluate(`state(data.issues.find(i=>i.id===${JSON.stringify(first.id)})).status`), '不复现');
  file.files = [{ size: 20, text: async () => '{"schema_version":2}' }];
  await file.onchange(); assert.ok(alertText.startsWith('导入失败'));
  file.files = [{size:129*1024*1024,text:async()=>{throw Error('Must reject before reading');}}];
  await file.onchange(); assert.ok(alertText.includes('128 MiB'));
  console.log(JSON.stringify({ ok: true, runs: summary.runs, observations: summary.case_observations, issues: issues.length, checks: ['jsonl/html consistency', 'unique IDs', 'raw evidence exists', 'runtime/category filters', 'search', 'keyboard and shift multi-selection', 'cross-page navigation', 'hover/selection shortcut targeting', 'editor/IME guards', 'bulk status and append/replace notes', 'undo and filtered-row removal', 'review export/import', 'invalid import rejected'], scope: 'mock DOM logic only; no pixel/layout or real browser verification; no files/reviews written' }));
}
main().catch(error => { console.error(String(error.stack).slice(0, 1800)); process.exitCode = 1; });
