import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/team.panel.js", import.meta.url), "utf8");

function harness() {
  const window = { OwoPanels: {}, location: { origin: "http://localhost" } };
  const document = { querySelector: () => null };
  vm.runInNewContext(source, { window, document, Promise, String, encodeURIComponent }, { filename: "team.panel.js" });
  const element = () => ({
    value: "",
    innerHTML: "",
    textContent: "",
    listeners: {},
    addEventListener(type, callback) { this.listeners[type] = callback; },
  });
  const root = () => {
    const nodes = new Map();
    return {
      nodes,
      innerHTML: "",
      querySelector(selector) {
        if (!nodes.has(selector)) nodes.set(selector, element());
        return nodes.get(selector);
      },
    };
  };
  return { panel: window.OwoPanels.team, root, window };
}

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

test("late team review response cannot overwrite the newly mounted panel", async () => {
  const { panel, root } = harness();
  const oldRoot = root();
  const newRoot = root();
  let finishOldReview;
  let finishNewReview;
  const oldHelpers = {
    get: async () => ({ entries: [] }),
    post: () => new Promise(resolve => { finishOldReview = resolve; }),
    esc: value => String(value == null ? "" : value),
    friendlyError: error => String(error),
  };
  const newHelpers = {
    get: async () => ({ entries: [] }),
    post: () => new Promise(resolve => { finishNewReview = resolve; }),
    esc: value => String(value == null ? "" : value),
    friendlyError: error => String(error),
  };

  panel.mount(oldRoot, oldHelpers);
  panel.doReview("old-package");
  panel.dispose();
  panel.mount(newRoot, newHelpers);
  panel.doReview("new-package");

  finishOldReview({ blocked: false, package: { id: "stale-package" }, findings: [] });
  await flush();
  assert.equal(newRoot.querySelector(".owo-team-findings").innerHTML, "");

  finishNewReview({ blocked: false, package: { id: "current-package" }, findings: [] });
  await flush();
  assert.match(newRoot.querySelector(".owo-team-findings").innerHTML, /current-package/);
  assert.doesNotMatch(newRoot.querySelector(".owo-team-findings").innerHTML, /stale-package/);
});

test("team import ignores duplicate clicks and unlocks the active mount after completion", async () => {
  const { panel, root } = harness();
  const firstRoot = root();
  const currentRoot = root();
  let finishImport;
  let importRequests = 0;
  const helpers = {
    get: async () => ({ entries: [] }),
    post: path => {
      if (path !== "/team/import") throw new Error("unexpected write: " + path);
      importRequests += 1;
      return new Promise(resolve => { finishImport = resolve; });
    },
    esc: value => String(value == null ? "" : value),
    friendlyError: error => String(error),
  };
  panel.mount(firstRoot, helpers);
  panel.doImport("encoded-package");
  panel.doImport("encoded-package");
  const firstButton = firstRoot.querySelector(".owo-team-importbtn");
  assert.equal(importRequests, 1);
  assert.equal(firstButton.disabled, true);
  assert.equal(firstButton.textContent, "正在导入…");

  panel.dispose();
  panel.mount(currentRoot, helpers);
  const button = currentRoot.querySelector(".owo-team-importbtn");
  assert.equal(button.disabled, true);

  finishImport({ blocked: false, package: { id: "team-package", version: "1" }, versions: [] });
  await flush();
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "导入（评审通过才落盘）");
  assert.match(currentRoot.querySelector(".owo-team-findings").textContent, /导入已在后台完成/);
});

test("late team review result cannot replace a newer review in the same mount", async () => {
  const { panel, root } = harness();
  const currentRoot = root();
  const finish = new Map();
  panel.mount(currentRoot, {
    get: async () => ({ entries: [] }),
    post: (_path, body) => new Promise(resolve => { finish.set(body.package_b64, resolve); }),
    esc: value => String(value == null ? "" : value),
    friendlyError: error => String(error),
  });
  panel.doReview("older-review");
  panel.doReview("newer-review");
  finish.get("newer-review")({ blocked: false, package: { id: "newer-result" }, findings: [] });
  await flush();
  finish.get("older-review")({ blocked: false, package: { id: "older-result" }, findings: [] });
  await flush();
  const findings = currentRoot.querySelector(".owo-team-findings").innerHTML;
  assert.match(findings, /newer-result/);
  assert.doesNotMatch(findings, /older-result/);
});


function helpers(post, get = async () => ({ entries: [] })) {
  return { post, get, esc: v => String(v ?? ""), friendlyError: e => e.message || String(e) };
}
test("export exposes the real package and clipboard failure reveals a manual copy field", async () => {
  const {panel, root} = harness(); const view = root();
  panel.mount(view, helpers(async () => ({manifest:{id:"actual",version:"2"},size_bytes:3,package_b64:"YWJj"})));
  await panel.doExport("actual");
  assert.equal(panel.exported.package_b64,"YWJj");
  assert.equal(view.querySelector(".owo-team-export-actions").hidden,false);
  await panel.copyExport();
  assert.equal(view.querySelector(".owo-team-export-code").value,"YWJj");
  assert.equal(view.querySelector(".owo-team-export-code").hidden,false);
  assert.match(view.querySelector(".owo-team-export-result").textContent,/手动复制/);
});
test("malformed export cannot keep a stale downloadable package", async () => {
  const {panel, root} = harness(); const view = root(); let calls=0;
  panel.mount(view,helpers(async () => ++calls === 1 ? {manifest:{id:"valid"},package_b64:"YWJj"} : {manifest:{id:"broken"}}));
  await panel.doExport("valid"); await panel.doExport("broken");
  assert.equal(panel.exported,null);
  assert.equal(view.querySelector(".owo-team-export-actions").hidden,true);
  assert.match(view.querySelector(".owo-team-export-result").textContent,/导出失败/);
});
test("blocked review with no findings never claims to pass; missing decision is rejected", () => {
  const {panel,root}=harness(); const view=root(); panel.mount(view,helpers(async()=>({})));
  panel.renderFindings({blocked:true,package:{id:"blocked"},findings:[]});
  const html=view.querySelector(".owo-team-findings").innerHTML;
  assert.match(html,/已阻断/); assert.doesNotMatch(html,/通过/);
  assert.throws(()=>panel.renderFindings({findings:[]}),/无法确认/);
});
test("editing package content invalidates a pending review and unlocks its button", async () => {
  const {panel,root}=harness(); const view=root(); let finish;
  panel.mount(view,helpers(()=>new Promise(resolve=>{finish=resolve;})));
  const pending=panel.doReview("before");
  panel.inputChanged();
  finish({blocked:false,package:{id:"obsolete"},findings:[]}); await pending;
  assert.doesNotMatch(view.querySelector(".owo-team-findings").innerHTML,/obsolete/);
  assert.match(view.querySelector(".owo-team-findings").textContent,/尚未评审/);
  assert.equal(view.querySelector(".owo-team-reviewbtn").disabled,false);
});
test("file reads neither submit nor overwrite edits made during asynchronous loading", async () => {
  const {panel,root}=harness(); const view=root(); let finish, posts=0;
  panel.mount(view,helpers(async()=>{posts++;}));
  const pending=panel.readFile({name:"package.json",size:30,text:()=>new Promise(resolve=>{finish=resolve;})});
  await flush(); view.querySelector(".owo-team-import-b64").value="manual-edit"; panel.inputChanged();
  finish('{"package_b64":"older-file"}'); await pending;
  assert.equal(view.querySelector(".owo-team-import-b64").value,"manual-edit"); assert.equal(posts,0);
  await panel.readFile({name:"current.json",size:30,text:async()=>'{"package_b64":"current-file"}'});
  assert.equal(view.querySelector(".owo-team-import-b64").value,"current-file"); assert.equal(posts,0);
});
test("old host import cannot notify or unlock a new host import", async () => {
  const {panel,root}=harness(); const finish=[];
  const h={...helpers(()=>new Promise(resolve=>finish.push(resolve))),baseUrl:"http://old"};
  panel.mount(root(),h); const old=panel.doImport("old");
  const view=root(); panel.mount(view,{...h,baseUrl:"http://new"}); const current=panel.doImport("new");
  finish[0]({blocked:false,package:{id:"old"},findings:[],versions:[]}); await old;
  assert.equal(panel.importBusy,true); assert.equal(view.querySelector(".owo-team-importbtn").disabled,true);
  assert.doesNotMatch(view.querySelector(".owo-team-findings").textContent,/后台完成/);
  finish[1]({blocked:false,package:{id:"new"},findings:[],versions:[]}); await current;
  assert.equal(panel.importBusy,false); assert.match(view.querySelector(".owo-team-findings").innerHTML,/new/);
});
test("audit failure is visible and a later refresh recovers", async () => {
  const {panel,root}=harness(); const view=root(); let fail=true;
  panel.mount(view,helpers(async()=>({}),async()=>{if(fail)throw new Error("offline");return {entries:["[1700000000] team/export: recovered"]};}));
  await flush(); assert.match(view.querySelector(".owo-team-audit").textContent,/审计读取失败/);
  fail=false; await panel.refreshAudit();
  assert.match(view.querySelector(".owo-team-audit").innerHTML,/recovered/);
});
