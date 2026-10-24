import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/notes.panel.js", import.meta.url), "utf8");

function makeNotesPanel(getOverride, confirmOverride, deleteOverride) {
  function element() {
    return {
      hidden: false,
      disabled: false,
      value: "",
      textContent: "",
      innerHTML: "",
      dataset: {},
      style: {},
      handlers: {},
      children: [],
      setAttribute(name, value) { this[name] = value; },
      removeAttribute(name) { delete this[name]; },
      addEventListener(name, handler) { this.handlers[name] = handler; },
      appendChild(child) { this.children.push(child); return child; },
      querySelector(selector) { return this.selectors && this.selectors[selector] || null; },
    };
  }
  const selectors = [
    ".owo-notes-btn-new", ".owo-notes-btn-save", ".owo-notes-btn-cancel",
    ".owo-notes-search", ".owo-notes-list", ".owo-notes-detail",
    ".owo-notes-editor", ".owo-notes-title", ".owo-notes-md",
    ".owo-notes-detail-title", ".owo-notes-detail-actions", ".owo-notes-detail-count",
    ".owo-notes-tree", ".owo-notes-btn-export-md", ".owo-notes-btn-export-html",
    ".owo-notes-btn-del", ".owo-notes-btn-rename", ".owo-notes-meta-item",
  ];
  const section = element();
  section.selectors = Object.fromEntries(selectors.map((selector) => [selector, element()]));
  const editor = section.selectors[".owo-notes-editor"];
  editor.selectors = {
    ".owo-notes-title": section.selectors[".owo-notes-title"],
    ".owo-notes-md": section.selectors[".owo-notes-md"],
  };
  const detail = section.selectors[".owo-notes-detail"];
  detail.hidden = false;
  detail.dataset.id = "existing-note";
  detail.selectors = {
    ".owo-notes-detail-title": section.selectors[".owo-notes-detail-title"],
    "#owo-notes-detail-count": section.selectors[".owo-notes-meta-item"],
    ".owo-notes-tree": section.selectors[".owo-notes-tree"],
  };
  const panelRoot = element();
  panelRoot.querySelector = () => section;
  const document = {
    getElementById(id) { return id === "panelRoot" ? panelRoot : null; },
    createElement() { return element(); },
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" } };
  runInNewContext(source, { window, document, Promise, Object, String, JSON, Blob: function () {} });
  const posts = [];
  const panel = window.OwoPanels.notes;
  panel.mount(section, {
    get(path) {
      if (typeof getOverride === "function") return getOverride(path);
      if (path === "/notes") return Promise.resolve({ notes: [] });
      return Promise.reject(new Error("unexpected GET: " + path));
    },
    post(path, body) { posts.push([path, body]); return Promise.resolve({ ok: true }); },
    put() { throw new Error("unexpected PUT"); },
    del(path) {
      if (typeof deleteOverride === "function") return deleteOverride(path);
      throw new Error("unexpected DELETE");
    },
    confirm: confirmOverride || (() => Promise.resolve(false)),
    notify() {},
    esc: String,
    friendlyError: (error) => error.message,
  });
  return { panel, section, editor, detail, posts };
}

test("creating a note after viewing another clears the stale note identity and saves typed Markdown", async () => {
  const h = makeNotesPanel();
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  assert.equal(h.editor.dataset.returnNoteId, "existing-note");
  assert.equal(h.detail.hidden, true);
  assert.equal(h.detail.dataset.id, "");

  h.section.selectors[".owo-notes-title"].value = "新笔记";
  h.section.selectors[".owo-notes-md"].value = `# 新正文

这里是编辑框里的内容`;
  await h.section.selectors[".owo-notes-btn-save"].handlers.click();

  assert.deepEqual(JSON.parse(JSON.stringify(h.posts)), [[
    "/notes",
    { title: "新笔记", markdown: `# 新正文

这里是编辑框里的内容` },
  ]]);
  assert.equal(h.editor.hidden, true);
  assert.equal(h.detail.hidden, true);
  assert.equal(h.section.selectors[".owo-notes-btn-save"].disabled, false);
});

test("cancelling note creation restores the previously selected note", () => {
  const h = makeNotesPanel();
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-btn-cancel"].handlers.click();
  assert.equal(h.detail.dataset.id, "existing-note");
  assert.equal(h.detail.hidden, false);
  assert.equal(h.editor.hidden, true);
});

test("deleting a note does not hide a different selection when the response arrives late", async () => {
  let resolveDelete;
  const h = makeNotesPanel(
    (path) => path === "/notes" ? Promise.resolve({ notes: [] }) : Promise.reject(new Error("unexpected GET: " + path)),
    () => Promise.resolve(true),
    (path) => {
      assert.equal(path, "/notes/existing-note");
      return new Promise((resolve) => { resolveDelete = resolve; });
    },
  );
  const detailTitle = h.section.selectors[".owo-notes-detail-title"];
  detailTitle.textContent = "原笔记";
  const deleting = h.section.selectors[".owo-notes-btn-del"].handlers.click();
  await Promise.resolve();
  await Promise.resolve();
  h.detail.dataset.id = "another-note";
  h.detail.hidden = false;
  resolveDelete({ ok: true });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.detail.hidden, false, "the currently selected note must remain visible");
  assert.equal(h.detail.dataset.id, "another-note");
  await deleting;
});

test("unmounting while delete confirmation is open prevents the delayed delete", async () => {
  let resolveConfirm;
  let deletes = 0;
  const h = makeNotesPanel(
    (path) => path === "/notes" ? Promise.resolve({ notes: [] }) : Promise.reject(new Error("unexpected GET: " + path)),
    () => new Promise((resolve) => { resolveConfirm = resolve; }),
    async () => { deletes += 1; },
  );
  const deleting = h.section.selectors[".owo-notes-btn-del"].handlers.click();
  h.panel.dispose();
  resolveConfirm(true);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(deletes, 0, "a confirmation resolved after unmount must not delete");
  await deleting;
});

test("note list shows a retryable inline error and recovers on retry", async () => {
  let attempts = 0;
  const h = makeNotesPanel((path) => {
    assert.equal(path, "/notes");
    attempts += 1;
    if (attempts === 1) return Promise.reject(new Error("service unavailable"));
    return Promise.resolve({ notes: [{ id: "note-1", title: "Recovered note" }] });
  });
  const list = h.section.selectors[".owo-notes-list"];
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /列表加载失败：service unavailable/);
  assert.match(list.innerHTML, /data-notes-retry/);

  const target = {
    closest(selector) {
      return selector === "[data-notes-retry]" ? { dataset: {} } : null;
    },
  };
  list.handlers.click({ target });
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(attempts, 2);
  assert.match(list.innerHTML, /Recovered note/);
  assert.doesNotMatch(list.innerHTML, /列表加载失败/);
});

test("note list announces its loading state before data arrives", async () => {
  let resolveNotes;
  const h = makeNotesPanel(() => new Promise((resolve) => { resolveNotes = resolve; }));
  const list = h.section.selectors[".owo-notes-list"];
  assert.match(list.innerHTML, /正在加载笔记/);
  await Promise.resolve();
  resolveNotes({ notes: [] });
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /暂无笔记/);
});

test("note full-text search is gated and explained when the core is disconnected", () => {
  const h = makeNotesPanel();
  const markup = h.panel.nav();
  const search = markup.match(/<input class="owo-notes-search"[^>]*>/)?.[0] || "";
  assert.match(search, /data-core-action/);
  assert.match(search, /aria-describedby="owo-notes-core-hint"/);
  assert.match(markup, /id="owo-notes-core-hint" data-core-action-hint/);
  assert.match(markup, /连接并授权本地核心后可搜索笔记全文和管理笔记/);
});

test("search uses latest-result-wins and exposes searching state", async () => {
  let resolveFirst;
  let resolveSecond;
  const h = makeNotesPanel((path) => {
    if (path === "/notes") return Promise.resolve({ notes: [] });
    if (path.endsWith("q=first")) return new Promise((resolve) => { resolveFirst = resolve; });
    if (path.endsWith("q=second")) return new Promise((resolve) => { resolveSecond = resolve; });
    return Promise.reject(new Error("unexpected GET: " + path));
  });
  const search = h.section.selectors[".owo-notes-search"];
  const list = h.section.selectors[".owo-notes-list"];
  await new Promise((resolve) => setImmediate(resolve));

  search.value = "first";
  search.handlers.keydown({ key: "Enter" });
  await Promise.resolve();
  search.value = "second";
  search.handlers.keydown({ key: "Enter" });
  await Promise.resolve();
  assert.match(list.innerHTML, /正在搜索/);

  resolveSecond({ hits: [{ doc_id: "n2", snippet: "new result" }] });
  await new Promise((resolve) => setImmediate(resolve));
  resolveFirst({ hits: [{ doc_id: "n1", snippet: "old result" }] });
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /new result/);
  assert.doesNotMatch(list.innerHTML, /old result/);
});

test("dispose prevents in-flight list response from mutating the unmounted page", async () => {
  let resolveNotes;
  const h = makeNotesPanel(() => new Promise((resolve) => { resolveNotes = resolve; }));
  const list = h.section.selectors[".owo-notes-list"];
  await Promise.resolve();
  h.panel.dispose();
  resolveNotes({ notes: [{ id: "late", title: "Late response" }] });
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /正在加载笔记/);
  assert.doesNotMatch(list.innerHTML, /Late response/);
});

test("slow create response does not close or replace a newer draft", async () => {
  const h = makeNotesPanel();
  let resolveCreated;
  h.panel.post = () => new Promise((resolve) => { resolveCreated = resolve; });
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-title"].value = "第一篇";
  const saving = h.section.selectors[".owo-notes-btn-save"].handlers.click();
  await Promise.resolve();

  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-title"].value = "第二篇";
  resolveCreated({ id: "first-note" });
  await saving;

  assert.equal(h.editor.hidden, false);
  assert.equal(h.section.selectors[".owo-notes-title"].value, "第二篇");
  assert.equal(h.detail.hidden, true);
});


test("pending note creation keeps the save lock across panel remounts", async () => {
  let resolveCreate;
  const h = makeNotesPanel((path) => path === "/notes"
    ? Promise.resolve({ notes: [] })
    : Promise.reject(new Error("unexpected GET: " + path)));
  h.panel.post = (path, body) => {
    h.posts.push([path, body]);
    return new Promise((resolve) => { resolveCreate = resolve; });
  };
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-title"].value = "只创建一次";
  const firstSave = h.section.selectors[".owo-notes-btn-save"].handlers.click();
  await Promise.resolve();
  assert.equal(h.posts.length, 1);

  h.panel.dispose();
  // Re-entering creates a fresh DOM button, which starts enabled unless the
  // panel carries the outstanding request state across its lifecycle.
  h.section.selectors[".owo-notes-btn-save"].disabled = false;
  h.section.selectors[".owo-notes-btn-save"].textContent = "保存";
  h.panel.mount(h.section, {
    get(path) { return path === "/notes" ? Promise.resolve({ notes: [] }) : Promise.reject(new Error(path)); },
    post(path, body) { h.posts.push([path, body]); return Promise.resolve({ ok: true }); },
    notify() {}, esc: String, friendlyError: (error) => error.message,
  });
  const saveButton = h.section.selectors[".owo-notes-btn-save"];
  assert.equal(saveButton.disabled, true);
  assert.equal(saveButton.textContent, "保存中…");
  h.section.selectors[".owo-notes-title"].value = "重复草稿";
  await saveButton.handlers.click();
  assert.equal(h.posts.length, 1, "新挂载页不能并发重复创建笔记");

  resolveCreate({ ok: true });
  await firstSave;
  assert.equal(saveButton.disabled, false);
  assert.equal(saveButton.textContent, "保存");
});

test("note list results are keyboard-focusable buttons with accessible names", async () => {
  const h = makeNotesPanel((path) => path === "/notes"
    ? Promise.resolve({ notes: [{ id: "note-1", title: "可访问笔记", updated_at: "2026-10-09T10:00:00Z" }] })
    : Promise.resolve({
        id: "note-1", title: "可访问笔记", root: "root",
        blocks: { root: { id: "root", kind: { Paragraph: { text: "正文" } }, children: [] } },
      }));
  const list = h.section.selectors[".owo-notes-list"];
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /<button type="button" class="owo-notes-open"/);
  assert.match(list.innerHTML, /data-notes-open/);
  assert.match(list.innerHTML, /aria-label="打开笔记：可访问笔记"/);

  const target = {
    closest(selector) {
      return selector === "[data-notes-open]" ? { dataset: { id: "note-1" } } : null;
    },
  };
  list.handlers.click({ target });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.detail.dataset.id, "note-1");
  assert.equal(h.detail.hidden, false);
});

test("search failures expose an in-place retry that repeats the current query", async () => {
  let attempts = 0;
  const h = makeNotesPanel((path) => {
    if (path === "/notes") return Promise.resolve({ notes: [] });
    if (path.includes("/notes/search?")) {
      attempts += 1;
      if (attempts === 1) return Promise.reject(new Error("temporary failure"));
      return Promise.resolve({ hits: [{ doc_id: "note-2", snippet: "匹配内容" }] });
    }
    return Promise.reject(new Error("unexpected GET: " + path));
  });
  const search = h.section.selectors[".owo-notes-search"];
  const list = h.section.selectors[".owo-notes-list"];
  await new Promise((resolve) => setImmediate(resolve));
  search.value = "query";
  search.handlers.keydown({ key: "Enter" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(list.innerHTML, /data-notes-search-retry/);
  list.handlers.click({
    target: {
      closest(selector) {
        return selector === "[data-notes-search-retry]" ? { dataset: {} } : null;
      },
    },
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(attempts, 2);
  assert.match(list.innerHTML, /匹配内容/);
  assert.match(list.innerHTML, /data-notes-open/);
});

test("note title rename has a visible button and ignores a response after switching notes", async () => {
  const h = makeNotesPanel((path) => {
    if (path === "/notes") return Promise.resolve({ notes: [] });
    if (path === "/notes/existing-note") return Promise.resolve({ id: "existing-note", title: "改名后", root: "", blocks: {} });
    return Promise.reject(new Error("unexpected GET: " + path));
  });
  assert.match(h.panel.nav(), /owo-notes-btn-rename/);
  assert.match(h.panel.nav(), /修改标题/);
  h.panel.prompt = () => Promise.resolve("改名后");
  const updates = [];
  h.panel.put = async (path, body) => { updates.push([path, body]); return { ok: true }; };
  h.section.selectors[".owo-notes-btn-rename"].handlers.click();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(JSON.parse(JSON.stringify(updates)), [["/notes/existing-note", { title: "改名后" }]]);
  assert.equal(h.detail.querySelector(".owo-notes-detail-title").textContent, "改名后");
});

test("semantic reading preserves headings, list structure, tables and multiline code", () => {
  const h=makeNotesPanel();
  const html=h.panel.renderDocument({root:"root",blocks:{
    root:{kind:{Paragraph:{text:""}},children:["heading","list","code","table"]},
    heading:{kind:{Heading:{level:2,text:"验证范围"}},children:[]},
    list:{kind:{List:{ordered:true}},children:["item"]},
    item:{kind:{ListItem:{text:"第一步"}},children:[]},
    code:{kind:{Code:{language:"js",text:"first\\nsecond"}},children:[]},
    table:{kind:{Table:{rows:[["A","B"],["1","2"]]}},children:[]}
  }});
  assert.match(html,/<h2>验证范围<\/h2>/);
  assert.match(html,/<ol><li>第一步<\/li><\/ol>/);
  assert.match(html,/<pre><code>first\\nsecond<\/code><\/pre>/);
  assert.match(html,/<td>A<\/td><td>B<\/td>/);
  assert.doesNotMatch(html,/owo-notes-block-id/);
});

test("untrusted embedded markup is escaped and cyclic document trees terminate", () => {
  const h=makeNotesPanel();
  h.panel.esc=value=>String(value).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;");
  const doc={root:"root",blocks:{
    root:{kind:{Paragraph:{text:""}},children:["embed","loop"]},
    embed:{kind:{HtmlEmbed:{html:"<img src=x onerror=bad()>"}},children:[]},
    loop:{kind:{Paragraph:{text:"safe"}},children:["loop"]}
  }};
  const html=h.panel.renderDocument(doc);
  assert.doesNotMatch(html,/<img/);
  assert.match(html,/&lt;img/);
  assert.match(html,/引用循环/);
  assert.match(h.panel.blockList(doc.blocks,["root"]),/引用循环/);
});

test("malformed successful list responses remain a read error rather than verified empty", async () => {
  const h=makeNotesPanel(async()=>({unexpected:[]}));
  await new Promise(resolve=>setImmediate(resolve));
  assert.match(h.section.selectors[".owo-notes-list"].innerHTML,/列表加载失败/);
  assert.doesNotMatch(h.section.selectors[".owo-notes-list"].innerHTML,/暂无笔记/);
});

test("search groups multiple matching blocks under their real document title", async () => {
  const h=makeNotesPanel(async path=>path==="/notes"?
    {notes:[{id:"n",title:"真实标题"}]}:{hits:[{doc_id:"n",snippet:"第一处"},{doc_id:"n",snippet:"第二处"}]});
  await new Promise(resolve=>setImmediate(resolve));
  const search=h.section.selectors[".owo-notes-search"];
  search.value="matching";
  search.handlers.keydown({key:"Enter"});
  await new Promise(resolve=>setImmediate(resolve));
  const html=h.section.selectors[".owo-notes-list"].innerHTML;
  assert.equal((html.match(/data-notes-open/g)||[]).length,1);
  assert.match(html,/真实标题/); assert.match(html,/第一处/); assert.match(html,/第二处/);
});

test("same-host remount restores a local note draft and host change clears it without a write", () => {
  const h=makeNotesPanel();
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-title"].value="草稿";
  h.section.selectors[".owo-notes-md"].value="first\\nsecond";
  const helpers=h.panel.helpers;
  h.panel.dispose();
  h.editor.hidden=true;h.section.selectors[".owo-notes-title"].value="";
  h.panel.mount(h.section,helpers);
  assert.equal(h.editor.hidden,false);
  assert.equal(h.section.selectors[".owo-notes-title"].value,"草稿");
  assert.equal(h.section.selectors[".owo-notes-md"].value,"first\\nsecond");
  h.panel.dispose();h.editor.hidden=true;
  h.panel.mount(h.section,{...helpers,baseUrl:"http://other-core"});
  assert.equal(h.panel.draft,null);
  assert.equal(h.editor.hidden,true);
  assert.equal(h.posts.length,0);
});

test("typing after a create starts preserves the newer draft when the response returns", async () => {
  const h=makeNotesPanel();
  h.section.selectors[".owo-notes-btn-new"].handlers.click();
  h.section.selectors[".owo-notes-title"].value="原始";
  h.section.selectors[".owo-notes-md"].value="submitted";
  let resolveCreate;
  h.panel.post=()=>new Promise(resolve=>{resolveCreate=resolve;});
  const pending=h.section.selectors[".owo-notes-btn-save"].handlers.click();
  h.section.selectors[".owo-notes-md"].value="new unsaved text";
  resolveCreate({id:"created"});
  await pending;
  assert.equal(h.editor.hidden,false);
  assert.equal(h.section.selectors[".owo-notes-md"].value,"new unsaved text");
});

test("local timestamps never render an invalid or missing date as a recent update", () => {
  const h=makeNotesPanel();
  assert.equal(h.panel.localTime(null),"时间未知");
  assert.equal(h.panel.localTime("not-a-date"),"时间未知");
  assert.doesNotMatch(h.panel.localTime("2026-10-24T03:00:00+08:00"),/时间未知|Invalid/);
});
