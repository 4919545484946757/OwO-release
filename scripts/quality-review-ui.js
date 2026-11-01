'use strict';
const data = JSON.parse(document.getElementById('data').textContent);
const statuses = ['待批阅', '确认问题', '可接受歧义', '标注需修正', '需要复现', '不复现'];
const byId = new Map(data.issues.map(issue => [issue.id, issue]));
const $ = id => document.getElementById(id);
const selected = new Set(), undoStack = [], visibleRows = new Map();
let reviews = {}, page = 0, activeId = null, anchorId = null, hoveredId = null, filterCache = null;
const storage = 'owo-quality-review-20261029';
try {
  const saved = JSON.parse(localStorage.getItem(storage) || '{}');
  if (saved && typeof saved === 'object' && !Array.isArray(saved)) {
    for (const [id, review] of Object.entries(saved)) {
      if (byId.has(id) && review && statuses.includes(review.status) && typeof review.notes === 'string')
        reviews[id] = { status: review.status, notes: review.notes.slice(0, 5000) };
    }
  }
} catch {}
function node(tag, text, cls) {
  const element = document.createElement(tag);
  if (text !== undefined) element.textContent = text;
  if (cls) element.className = cls;
  return element;
}
function state(issue) { return reviews[issue.id] || { status: '待批阅', notes: '' }; }
function persist() {
  try { localStorage.setItem(storage, JSON.stringify(reviews)); return true; } catch { return false; }
}
function options(id, values) {
  for (const value of values) { const option = node('option', value); option.value = value; $(id).append(option); }
}
options('priority', [...new Set(data.issues.map(issue => issue.priority))]);
options('kind', [...new Set(data.issues.map(issue => issue.kind))]);
options('status', statuses); options('bulk-status', statuses);
$('stats').textContent = `${data.summary.runs} 个后端/配置轮次 · ${data.summary.case_observations} 条观测 · ${data.summary.distinct_test_definitions} 个独立测试定义 · ${data.summary.issue_count} 个去重待批阅项`;
const category = node('select'); category.id = 'category';
const categoryAll = node('option', '所有语料类别'); categoryAll.value = ''; category.append(categoryAll); $('runtime').after(category);
for (const [value, label] of [['full','完整拼音'],['explicit','显式分割'],['initials','简拼'],['initial_character','单字首字母'],['mixed','混拼'],['incomplete','不完整拼音'],['typo','纠错变体'],['prefix','部分消费前缀'],['context','上下文'],['dictionary_self_consistency','词典自一致性'],['prefix_latency_probe','中间前缀延迟']]) {
  const option = node('option', label); option.value = value; category.append(option);
}
function relevant(issue) {
  const mode = $('runtime').value;
  return issue.observations.filter(observation => mode === 'all' || (mode === 'active'
    ? observation.runtime === 'base' || observation.runtime === 'current' : observation.runtime === mode))
    .sort((a,b) => (a.runtime === 'current' ? 0 : a.runtime === 'base' ? 1 : 2) - (b.runtime === 'current' ? 0 : b.runtime === 'base' ? 1 : 2));
}
function filtered() {
  if (filterCache) return filterCache;
  const query = $('search').value.trim().toLowerCase();
  filterCache = data.issues.filter(issue => relevant(issue).length && (!category.value || issue.tags.includes(category.value))
    && (!$('priority').value || issue.priority === $('priority').value) && (!$('kind').value || issue.kind === $('kind').value)
    && (!$('status').value || state(issue).status === $('status').value)
    && (!query || [issue.input, issue.expected.join(' '), issue.tags.join(' '), issue.kind, ...relevant(issue).map(o => o.first_final || '')].join(' ').toLowerCase().includes(query)));
  return filterCache;
}
function announce(message) { $('action-message').textContent = message; }
function paintSelection() {
  for (const [id, row] of visibleRows) {
    row.tr.classList.toggle('selected', selected.has(id)); row.tr.classList.toggle('active', activeId === id);
    row.tr.setAttribute('aria-selected', String(selected.has(id))); row.checkbox.checked = selected.has(id);
    row.select.value = state(byId.get(id)).status;
    if (document.activeElement !== row.notes) row.notes.value = state(byId.get(id)).notes;
  }
  $('selection-count').textContent = `已选 ${selected.size} 项`;
  $('bulk-apply').disabled = selected.size === 0; $('clear-selection').disabled = selected.size === 0;
  $('undo').disabled = undoStack.length === 0;
}
function focusActive() {
  const row = visibleRows.get(activeId);
  if (row) {
    row.tr.style.scrollMarginTop = `${($('bulk-toolbar').offsetHeight || 0)+12}px`;
    row.tr.focus({ preventScroll: true }); row.tr.scrollIntoView({ block: 'nearest' });
  }
}
function selectIssue(id, extend = false, toggle = false) {
  const all = filtered(), index = all.findIndex(issue => issue.id === id);
  if (index < 0) return;
  hoveredId = null;
  const anchor = all.findIndex(issue => issue.id === anchorId);
  if (extend && anchor >= 0) {
    selected.clear(); for (const issue of all.slice(Math.min(anchor,index), Math.max(anchor,index)+1)) selected.add(issue.id);
  } else if (toggle) {
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    anchorId = id;
  } else { selected.clear(); selected.add(id); anchorId = id; }
  activeId = id; paintSelection();
}
function navigate(delta, extend) {
  const all = filtered(); if (!all.length) return;
  const index = all.findIndex(issue => issue.id === activeId);
  const next = index < 0 ? Math.min(page * +$('size').value, all.length-1) : Math.max(0, Math.min(all.length-1, index+delta));
  selectIssue(all[next].id, extend);
  const targetPage = Math.floor(next / +$('size').value);
  if (page !== targetPage) { page = targetPage; render(); }
  focusActive();
}
function turnPage(delta) {
  const all = filtered(), size = +$('size').value, target = Math.max(0, Math.min(Math.max(0, Math.ceil(all.length/size)-1), page+delta));
  if (target === page) return;
  page = target; selected.clear(); hoveredId = null;
  activeId = all[page*size]?.id || null; anchorId = activeId; if (activeId) selected.add(activeId);
  render(); focusActive();
}
function shortcutTargets() {
  if (hoveredId && visibleRows.has(hoveredId) && !selected.has(hoveredId)) return [hoveredId];
  return [...selected];
}
function applyReview(ids, status, notes = '', mode = 'append') {
  ids = [...new Set(ids)].filter(id => byId.has(id));
  if (!ids.length) { announce('请先选择或悬浮一个待批注目标。'); return; }
  const allBefore = filtered(), oldIndex = allBefore.findIndex(issue => issue.id === activeId), changes = [];
  for (const id of ids) {
    const previous = state(byId.get(id));
    const nextNotes = notes ? (mode === 'replace' ? notes : [previous.notes, notes].filter(Boolean).join('\n')).slice(0,5000) : previous.notes;
    const next = { status: status || previous.status, notes: nextNotes };
    if (next.status === previous.status && next.notes === previous.notes) continue;
    changes.push({ id, before: reviews[id] ? { ...reviews[id] } : null, after: next }); reviews[id] = next;
  }
  if (changes.length) { undoStack.push(changes); if (undoStack.length > 30) undoStack.shift(); }
  const stored = persist(); filterCache = null;
  if ($('status').value) {
    const all = filtered(), allowed = new Set(all.map(issue => issue.id));
    for (const id of selected) if (!allowed.has(id)) selected.delete(id);
    if (!allowed.has(activeId)) {
      activeId = all[Math.max(0,Math.min(oldIndex,all.length-1))]?.id || null; anchorId = activeId;
      if (!selected.size && activeId) selected.add(activeId);
    }
    render();
  } else paintSelection();
  announce(`已批注 ${ids.length} 项${status ? '：'+status : ''}。${stored ? '已保存到浏览器；可下载JSON备份。' : '浏览器保存失败，请立即下载JSON备份。'}`);
}
function undo() {
  const changes = undoStack.pop(); if (!changes) return;
  for (const change of changes) {
    const before = change.before || {status:'待批阅',notes:''}, current = state(byId.get(change.id));
    // Do not overwrite notes manually edited after the batch being undone.
    const restored = {status:before.status,notes:current.notes === change.after.notes ? before.notes : current.notes};
    if (!change.before && !restored.notes && restored.status==='待批阅') delete reviews[change.id]; else reviews[change.id] = restored;
  }
  persist(); filterCache = null; render(); announce(`已撤销 ${changes.length} 项批注。`);
}
function render() {
  const all = filtered(), size = +$('size').value;
  const allowed = new Set(all.map(issue => issue.id));
  for (const id of selected) if (!allowed.has(id)) selected.delete(id);
  if (!allowed.has(activeId)) activeId = anchorId = null;
  page = Math.max(0, Math.min(page, Math.max(0,Math.ceil(all.length/size)-1)));
  hoveredId = null; visibleRows.clear(); $('rows').replaceChildren();
  $('page').textContent = `筛选 ${all.length} 项 · 第 ${page+1} / ${Math.max(1,Math.ceil(all.length/size))} 页`;
  $('prev').disabled = page === 0; $('next').disabled = (page+1)*size >= all.length;
  for (const issue of all.slice(page*size,(page+1)*size)) {
    const tr = node('tr', undefined, 'review-row'); tr.tabIndex = 0;
    const checkCell = node('td'), checkbox = node('input'); checkbox.type = 'checkbox'; checkbox.setAttribute('aria-label', '选择 '+issue.input); checkCell.append(checkbox);
    checkbox.onclick = event => event.stopPropagation();
    checkbox.onchange = () => { if (checkbox.checked) selected.add(issue.id); else selected.delete(issue.id); activeId = anchorId = issue.id; paintSelection(); };
    tr.onclick = event => {
      if (event.target.closest('input,textarea,select,button,a,summary,details')) return;
      selectIssue(issue.id, event.shiftKey, event.ctrlKey || event.metaKey); focusActive();
    };
    // Pointer movement (not DOM insertion under a stationary pointer) establishes hover targeting.
    tr.onpointermove = () => { hoveredId = issue.id; };
    tr.onpointerleave = () => { if (hoveredId === issue.id) hoveredId = null; };
    const a = node('td', undefined, 'priority '+issue.priority); a.append(node('div',issue.priority),node('div',issue.kind),node('div',issue.id,'small'));
    const b = node('td', undefined, 'target'); b.append(node('div',issue.expected.join(' / ')),node('code',issue.input));
    for (const tag of issue.tags) b.append(node('span',tag,'small badge'));
    const observations = relevant(issue), c = node('td',undefined,'details'), o = observations[0];
    c.append(node('div','实际首选（'+o.runtime+'）：'+(o.first_final||'（无）')),node('div',`基础排名 ${o.base_rank??'未命中'} / 最终首页 ${o.final_rank??'未命中'} · 当前筛选 ${observations.length} / 存档 ${issue.observations.length} 条观测`,'small'));
    const detail = node('details'); detail.append(node('summary','查看配置、候选与差异证据'));
    let evidenceLoaded = false;
    detail.ontoggle = () => { if (detail.open && !evidenceLoaded) { detail.append(node('pre',JSON.stringify({test:issue.test,observations},null,2))); evidenceLoaded = true; } };
    c.append(detail);
    const e = node('td'), select = node('select'); select.setAttribute('aria-label','批注 '+issue.input);
    for (const status of statuses) { const option = node('option',status); option.value = status; select.append(option); }
    select.value = state(issue).status;
    const notes = node('textarea'); notes.placeholder = '人工判断依据 / 复现备注'; notes.value = state(issue).notes; notes.maxLength = 5000;
    select.onchange = () => applyReview([issue.id],select.value);
    notes.oninput = () => { reviews[issue.id] = {status:state(issue).status,notes:notes.value.slice(0,5000)}; persist(); };
    e.append(select,notes); tr.append(checkCell,a,b,c,e); $('rows').append(tr); visibleRows.set(issue.id,{tr,checkbox,select,notes});
  }
  paintSelection();
}
function resetSelection() { selected.clear(); activeId = anchorId = hoveredId = null; }
for (const id of ['search','runtime','category','priority','kind','status','size']) $(id).addEventListener(id==='search'?'input':'change', () => { page=0; filterCache=null; resetSelection(); render(); });
$('prev').onclick = () => turnPage(-1); $('next').onclick = () => turnPage(1);
$('reset').onclick = () => { for (const id of ['search','category','priority','kind','status']) $(id).value=''; $('runtime').value='active'; page=0; filterCache=null; resetSelection(); render(); };
function selectMany(ids) {
  for (const id of ids) selected.add(id);
  if (!activeId || !selected.has(activeId)) activeId = ids[0] || null;
  anchorId = activeId; paintSelection(); focusActive();
}
$('select-page').onclick = () => selectMany([...visibleRows.keys()]);
$('select-filtered').onclick = () => selectMany(filtered().map(issue => issue.id));
$('clear-selection').onclick = () => { resetSelection(); paintSelection(); };
$('bulk-apply').onclick = () => {
  if (!$('bulk-status').value && !$('bulk-notes').value.trim()) { announce('请选择批注状态或填写批量备注。'); return; }
  applyReview([...selected], $('bulk-status').value, $('bulk-notes').value.trim(), $('notes-mode').value); focusActive();
};
$('undo').onclick = undo;
document.addEventListener('keydown', event => {
  if (event.isComposing || event.keyCode === 229 || event.altKey || event.metaKey) return;
  const target = event.target, control = target?.closest?.('input,textarea,select,[contenteditable]');
  if (target?.isContentEditable || (control && !(control.tagName.toLowerCase()==='input' && control.type==='checkbox'))) return;
  if (event.ctrlKey) { if (event.key.toLowerCase()==='z') { event.preventDefault(); undo(); } return; }
  if (event.key==='ArrowUp' || event.key==='ArrowDown') { event.preventDefault(); navigate(event.key==='ArrowUp'?-1:1,event.shiftKey); return; }
  if (!event.shiftKey && (event.key==='ArrowLeft' || event.key==='ArrowRight')) { event.preventDefault(); turnPage(event.key==='ArrowLeft'?-1:1); return; }
  const status = event.key===' ' || event.code==='Space' ? '确认问题' : event.key.toLowerCase()==='z' ? '可接受歧义' : event.key==='Backspace' ? '待批阅' : null;
  if (status) { event.preventDefault(); if (!event.repeat) applyReview(shortcutTargets(),status); }
});
$('save').onclick = () => {
  const result = {schema_version:1,report:'OwO quality review 2026-10-29',saved_at:new Date().toISOString(),reviews:data.issues.map(issue=>({id:issue.id,input:issue.input,expected:issue.expected,...state(issue)}))};
  const url = URL.createObjectURL(new Blob([JSON.stringify(result,null,2)],{type:'application/json;charset=utf-8'}));
  const link = node('a'); link.href=url; link.download='owo-quality-human-review.json'; link.click(); setTimeout(()=>URL.revokeObjectURL(url),1000);
};
$('load').onclick = () => $('file').click();
$('file').onchange = async () => {
  try {
    const file=$('file').files[0]; if (!file || file.size>128*1024*1024) throw Error('文件超过128 MiB或未选择');
    const input=JSON.parse(await file.text()); if (input.schema_version!==1 || !Array.isArray(input.reviews)) throw Error('不是批阅JSON');
    let count=0;
    for (const item of input.reviews) if (byId.has(item.id) && statuses.includes(item.status) && typeof item.notes==='string' && item.notes.length<=5000) { reviews[item.id]={status:item.status,notes:item.notes}; count++; }
    persist(); undoStack.length=0; filterCache=null; resetSelection(); render(); alert(`已导入 ${count} 项；未匹配的旧问题没有覆盖新问题`);
  } catch(error) { alert('导入失败：'+error.message); }
};
render();
