import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
const source = readFileSync(new URL("../panels/evidence.panel.js", import.meta.url), "utf8");
const escape = s => String(s).replace(/[&<>"]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const deferred = () => { let resolve, reject; const promise = new Promise((a,b) => { resolve=a; reject=b; }); return {promise,resolve,reject}; };
function setup(overrides = {}) {
  const listeners = new Map(), host = { innerHTML:"", addEventListener:(k,f)=>listeners.set(k,f), removeEventListener:(k,f)=>{if(listeners.get(k)===f)listeners.delete(k);}, querySelector:()=>null };
  const root = {crypto:{randomUUID:()=>"fixture-idempotency"}, OwoSessionFacts:{fromDetail:()=>({})}, OwoCompletionRecord:{summarize:()=>null}};
  runInNewContext(source, root);
  const panel=root.OwoPanels.evidence, calls=[];
  const helpers={esc:escape, renderMarkdown:escape, snapshot:()=>({sessions:[]}), friendlyError:e=>e.message, confirm:async()=>true,
    get:async path => { calls.push(path); if(path==="/teams")return {teams:[]}; if(path.endsWith("/artifacts"))return {artifacts:[]}; if(path.endsWith("/content"))return {content:"actual body"}; if(path.endsWith("/history"))return {reviews:[]}; return {artifact_id:path.split("/")[2],team_id:"team",version:1,review_state:"Draft",size_bytes:11}; },
    post:async(path,body)=>{calls.push([path,body]);return {};}, ...overrides };
  return {root,panel,t:panel._test,host,helpers,calls,listeners};
}
test("artifact states and absent schema checks never imply pending review or acceptance",async()=>{
  const r=setup(); await r.panel.mount(r.host,r.helpers);
  assert.equal(r.t.reviewText("Draft"),"草稿"); assert.equal(r.t.reviewText("pending_review"),"待评审");
  assert.equal(r.t.reviewText("Rejected"),"已拒绝"); assert.equal(r.t.validationText({}),"格式校验未返回");
  r.t.state.selected="p";r.t.state.manifest=[{artifact_id:"a",kind:"<unsafe>",version:1,review_state:"Draft"}];
  assert.match(r.t.artifactRows(),/草稿/); assert.doesNotMatch(r.t.artifactRows(),/待评审/);
  assert.match(r.t.artifactRows(),/&lt;unsafe&gt;/);
});
test("project refresh uses all version records, retains selected version and opens actual content",async()=>{
  const r=setup({get:async path=>{
    if(path==="/teams")return {teams:[{project_space_id:"p"}]};
    if(path.endsWith("/artifacts"))return {artifacts:[{artifact_id:"a",review_state:"Draft"},{artifact_id:"b",review_state:"Rejected"}]};
    if(path.endsWith("/history"))return {reviews:[]};
    if(path.endsWith("/content"))return {content:"actual"};
    return {artifact_id:path.split("/")[2],team_id:"t",version:1,size_bytes:6};
  }});
  await r.panel.mount(r.host,r.helpers); assert.equal(r.t.state.content,"actual");
  await r.t.openArtifact("b"); await r.panel.refresh();
  assert.equal(r.t.state.artifactId,"b"); assert.equal(r.t.state.manifest.length,2);
});
test("history failure does not hide metadata and preview",async()=>{
  const r=setup(); const get=r.helpers.get;
  r.helpers.get=path=>path.endsWith("/history")?Promise.reject(Error("history down")):get(path);
  await r.panel.mount(r.host,r.helpers); await r.t.openArtifact("a");
  assert.equal(r.t.state.detail.metadata.artifact_id,"a"); assert.equal(r.t.state.content,"actual body");
  assert.match(r.t.state.historyError,/history down/);
});
test("late preview from an older artifact cannot replace the latest selection",async()=>{
  const wait=deferred(),r=setup();const get=r.helpers.get;
  r.helpers.get=path=>path==="/artifacts/a/content"?wait.promise:get(path);
  await r.panel.mount(r.host,r.helpers); const old=r.t.openArtifact("a");
  // Metadata is ready and the preview is in flight.
  await new Promise(resolve=>setImmediate(resolve));
  await r.t.openArtifact("b");wait.resolve({content:"obsolete"});await old;
  assert.equal(r.t.state.artifactId,"b");assert.equal(r.t.state.content,"actual body");
});
test("review drafts survive preview and refresh, and remain isolated by artifact and version",async()=>{
  const r=setup();await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");
  const meta=r.t.state.detail.metadata;r.t.draftFor(meta).comment="keep this";r.t.draftFor(meta).reviewer="reviewer";
  await r.t.preview();await r.t.openArtifact("a");
  assert.equal(r.t.draftFor(r.t.state.detail.metadata).comment,"keep this");
  assert.equal(r.t.draftFor({artifact_id:"a",version:2}).comment,"");
  await r.t.openArtifact("b");assert.equal(r.t.draftFor(r.t.state.detail.metadata).comment,"");
});
test("failed review keeps the exact draft and reuses its idempotency key on retry",async()=>{
  const bodies=[],r=setup({post:async(path,body)=>{bodies.push(body);throw Error("403 denied");}});
  await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");r.t.draftFor(r.t.state.detail.metadata).comment="evidence";
  await r.t.review("approve");await r.t.review("approve");
  assert.match(r.t.state.error,/草稿已保留.*403 denied/);
  assert.equal(r.t.draftFor(r.t.state.detail.metadata).comment,"evidence");
  assert.equal(bodies[0].expected_version,1);assert.equal(bodies[0].comment,"evidence");
  assert.equal(bodies[0].idempotency_key,bodies[1].idempotency_key);
});
test("confirmation owns a version and blocks duplicate review submission",async()=>{
  const wait=deferred(),r=setup({confirm:()=>wait.promise});await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");
  const first=r.t.review("approve");await r.t.review("approve");await r.t.openArtifact("b");
  wait.resolve(true);await first;
  assert.equal(r.calls.filter(Array.isArray).length,0);assert.equal(r.t.state.artifactId,"b");
});
test("successful review updates both detail and list from host response and clears only submitted draft",async()=>{
  let approved=false;const r=setup({post:async()=>{approved=true;return {artifact:{artifact_id:"a",review_state:"Approved",version:1}};}});
  const get=r.helpers.get;r.helpers.get=async path=>{const data=await get(path);return path.endsWith("/metadata")?{...data,review_state:approved?"Approved":"Draft"}:data;};
  await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");
  r.t.state.manifest=[{artifact_id:"a",version:1,review_state:"Draft"}];r.t.draftFor(r.t.state.detail.metadata).comment="checked";
  await r.t.review("approve");assert.equal(r.t.state.detail.metadata.review_state,"Approved");
  assert.equal(r.t.state.manifest[0].review_state,"Approved");assert.equal(r.t.draftFor(r.t.state.detail.metadata).comment,"");
  assert.match(r.t.state.notice,/评审已提交/);
});
test("unmounted review cannot update a remounted page or duplicate event listeners",async()=>{
  const wait=deferred(),r=setup({post:()=>wait.promise});await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");
  const old=r.t.review("reject");await new Promise(resolve=>setImmediate(resolve));
  r.panel.dispose();assert.equal(r.listeners.size,0);await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("b");
  wait.resolve({artifact:{artifact_id:"a",review_state:"Rejected"}});await old;
  assert.equal(r.t.state.artifactId,"b");assert.equal(r.listeners.size,5);assert.equal(r.t.state.notice,"");
});
test("large or unknown content is never fetched automatically",async()=>{
  const r=setup();const get=r.helpers.get;r.helpers.get=async path=>path.endsWith("/metadata")?{artifact_id:"large",team_id:"t",version:1,size_bytes:262145}:get(path);
  await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("large");
  assert.equal(r.calls.some(path=>typeof path==="string"&&path.endsWith("/content")),false);
  assert.match(r.t.state.previewError,/256 KiB/);
});
test("tab arrow navigation wraps and keeps modifiers and inputs untouched",async()=>{
  const r=setup();await r.panel.mount(r.host,r.helpers);
  let prevented=0;const event={target:{closest:()=>({dataset:{evidenceTab:"preview"}})},key:"ArrowLeft",preventDefault:()=>prevented++};
  r.listeners.get("keydown")(event);assert.equal(r.t.state.tab,"review");assert.equal(prevented,1);
  r.listeners.get("keydown")({...event,ctrlKey:true,key:"Home"});assert.equal(r.t.state.tab,"review");assert.equal(prevented,1);
});

test("asynchronous preview repaint restores the review field caret for the same version",async()=>{
  const r=setup();await r.panel.mount(r.host,r.helpers);await r.t.openArtifact("a");
  const identity=r.t.draftKey(r.t.state.detail.metadata), focused={
    selectionStart:2,selectionEnd:4,hasAttribute:name=>name==="data-review-note",getAttribute:()=>null,
    closest:()=>({getAttribute:()=>identity})
  };
  const restored=[];r.root.document={activeElement:focused};r.host.contains=node=>node===focused;
  r.host.querySelector=selector=>selector==="[data-review-note]"?{disabled:false,focus:()=>restored.push("focus"),setSelectionRange:(a,b)=>restored.push([a,b])}:null;
  await r.t.preview();assert.equal(restored.filter(v=>v==="focus").length,2);
  assert.deepEqual(restored.filter(Array.isArray),[[2,4],[2,4]]);
});
test("search can match identity and real review state without displaying HTML",async()=>{
  const r=setup();await r.panel.mount(r.host,r.helpers);
  r.t.state.selected="p";r.t.state.manifest=[{artifact_id:"one",kind:"first",version:1,review_state:"Draft"},{artifact_id:"two",kind:"second",version:2,review_state:"Rejected"}];
  r.t.state.filter="已拒绝";const html=r.t.artifactRows();assert.match(html,/two/);assert.doesNotMatch(html,/data-artifact="one"/);
  r.t.state.filter="missing";assert.match(r.t.artifactRows(),/没有匹配/);
  assert.match(r.t.reviewText("constructor"),/未知状态/);
});
