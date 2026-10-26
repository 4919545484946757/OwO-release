import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {createHash} from "node:crypto";
const require=createRequire(import.meta.url),S=require("../core/worker-input-snapshot.js"),P=require("../core/provider-call-inspector.js");
const hash=v=>createHash("sha256").update(v).digest("hex"),id="input-00000000-0000-4000-8000-000000000001";
const part={type:"image",asset:{artifact_id:id,sha256:"a".repeat(64)}};
const message={role:"user",content:"compiled suffix must not display",input:{prompt:'查看 "图片" 🧪\n原文',content_parts:[part,{type:"text",text:"额外上下文"}]}};
const identity=v=>{const text=JSON.stringify(v);return {bytes:Buffer.byteLength(text),sha256:hash(text)};};
function receipt(m=message){
 const input=JSON.parse(S.encoded(m));
 return {schema_version:1,run_id:"team",seq:8,event_id:"run:team:8",event_type:"model.input.prepared",
  data:{call_id:"call",worker_session_id:"team-worker-review",boundary:"before_provider_delegate",model_delivery:"not_proven",
   representation:"gateway_chat_message_v1_including_input_metadata",request:identity([]),message_count:2,tool_count:0,image_count:1,
   last_user_input:{identity:identity(input),prompt:identity(input.prompt),part_count:(input.content_parts||[]).length,
    parts:(input.content_parts||[]).slice(0,8).map(p=>p.type==="text"?{type:"text",identity:identity(p.text)}:p),
    parts_truncated:(input.content_parts||[]).length>8,legacy_attachment_count:(input.legacy_attachments||[]).length}}};
}
const row={id,name:"source.png",mime:"image/png",size:10,content_part:part};
const blob=v=>new Blob([JSON.stringify(v)],{type:"application/json"});
function harness(extra={}){
 const calls=[],session={id:"team-worker-review",messages:[{role:"user",input:{prompt:"newer input"}},message]};
 const view=S.create({runId:"team",digest:async v=>hash(v),api:{download:async(path,opts)=>{
  calls.push({path,opts});return path.endsWith("/attachments")?blob([row]):blob(session);
 }},...extra});
 return {view,calls,session};
}
test("matches exact saved input rather than newest message and uses bounded authenticated downloads",async()=>{
 const h=harness();await h.view.open(receipt());
 assert.equal(h.view.state.message.input.prompt,message.input.prompt);
 assert.deepEqual(h.calls.map(c=>c.path),["/session/team-worker-review","/session/team-worker-review/attachments"]);
 assert(h.calls.every(c=>c.opts.maxResponseBytes===4*1024*1024&&c.opts.signal));
 assert.match(h.view.html(),/额外上下文/);assert(!h.view.html().includes("compiled suffix"));
 assert.match(h.view.html(),/data-media-artifact/);assert.match(h.view.html(),/sha256/);h.view.dispose();
});
test("serde-compatible field ordering, omitted empty vectors, Unicode and text blocks are stable",()=>{
 assert.equal(S.encoded({input:{legacy_attachments:[],content_parts:[{asset:{sha256:"a".repeat(64),artifact_id:id},type:"image"},{text:"x",type:"text"}],prompt:"中"}}),
 '{"prompt":"中","content_parts":[{"type":"image","asset":{"artifact_id":"'+id+'","sha256":"'+"a".repeat(64)+'"}},{"type":"text","text":"x"}]}');
 assert.equal(S.encoded({input:{prompt:"",content_parts:[],legacy_attachments:[]}}),'{"prompt":""}');
});
test("invalid, foreign, missing and unsupported receipts never fetch a session",async()=>{
 for(const change of [e=>e.run_id="foreign",e=>e.event_id="wrong",e=>e.data.worker_session_id="foreign-worker-review",e=>e.data.last_user_input=null,e=>e.data.last_user_input.identity.sha256="bad",e=>e.event_type="model.request.completed"]){
  const h=harness(),e=receipt();change(e);await h.view.open(e);assert.equal(h.calls.length,0);assert.match(h.view.state.error,/身份/);h.view.dispose();
 }
});
test("compacted, missing or changed checkpoints never substitute latest input",async()=>{
 for(const messages of [[],[{role:"user",input:{prompt:"newer"}}],[{...message,input:{...message.input,prompt:"changed"}}],[{...message,role:"assistant"}]]){
  const h=harness();h.session.messages=messages;await h.view.open(receipt());assert.equal(h.view.state.message,null);assert.equal(h.calls.length,1);assert.match(h.view.state.error,/未找到摘要完全一致/);h.view.dispose();
 }
});
test("full identity and summary counters/prompt must agree before exposing saved text",async()=>{
 for(const change of [e=>e.data.last_user_input.part_count=3,e=>e.data.last_user_input.prompt=identity("different"),e=>e.data.last_user_input.legacy_attachment_count=1]){
  const h=harness(),e=receipt();change(e);
  if(e.data.last_user_input.part_count===3)e.data.last_user_input.parts.push({type:"text",identity:identity("x")});
  await h.view.open(e);assert.equal(h.view.state.message,null);assert.match(h.view.state.error,/不符/);assert.equal(h.calls.length,1);h.view.dispose();
 }
});
test("missing/denied/mismatched attachment metadata keeps original text without any media request",async()=>{
 for(const result of [[],[row,row],[{...row,content_part:{...part,asset:{...part.asset,sha256:"b".repeat(64)}}}]]){
  const h=harness({api:{download:async p=>p.endsWith("/attachments")?blob(result):blob({id:"team-worker-review",messages:[message]})}});
  await h.view.open(receipt());assert(h.view.state.message);assert(h.view.state.metadataError);assert(!h.view.html().includes("data-media-artifact"));assert.match(h.view.html(),/未加载内容/);h.view.dispose();
 }
});
test("response size, MIME, session identity, count and invalid JSON are rejected",async()=>{
 for(const data of [new Blob([" ".repeat(4*1024*1024+1)],{type:"application/json"}),new Blob(["{}"],{type:"text/html"}),blob({id:"other",messages:[]}),blob({id:"team-worker-review",messages:Array(4097).fill({})}),new Blob(["bad"],{type:"application/json"})]){
  const h=harness({api:{download:async()=>data}});await h.view.open(receipt());assert(h.view.state.error);assert.equal(h.view.state.message,null);h.view.dispose();
 }
});
test("switch/close/dispose abort in-flight canonical reads and ignore late data",async()=>{
 for(const action of ["close","dispose","switch"]){
  let finish,signal;const h=harness({api:{download:(_,opts)=>{signal=opts.signal;return new Promise(r=>finish=r);}}});
  const pending=h.view.open(receipt());assert.equal(h.view.state.busy,true);
  if(action==="switch"){const next=receipt();next.data.worker_session_id="foreign";await h.view.open(next);}else h.view[action]();
  assert.equal(signal.aborted,true);finish(blob({id:"team-worker-review",messages:[message]}));await pending;assert.equal(h.view.state.message,null);
 }
});
test("cancellation after hash computation cannot mount another attempt's saved input",async()=>{
 let finish;const h=harness({digest:()=>new Promise(r=>finish=r)});const pending=h.view.open(receipt());
 await new Promise(r=>setImmediate(r));assert(finish);h.view.close();finish(hash(S.encoded(message)));await pending;assert.equal(h.view.state.message,null);
});
test("legacy filenames stay explicit, no cached inline images or mutable-path fetch",async()=>{
 const m={role:"user",images:[{url:"data:image/png;base64,ignored"}],input:{prompt:"old",legacy_attachments:["old.png"]}};
 const h=harness();h.session.messages=[m];await h.view.open(receipt(m));assert(h.view.state.message);assert.match(h.view.html(),/旧附件缺少不可变引用/);assert(!h.view.html().includes("data:image"));assert.equal(h.calls.length,1);
});
test("plain and file inputs are readable without treating local paths as image sources",async()=>{
 const m={role:"user",input:{prompt:"file <script>alert(1)</script>",content_parts:[{type:"file",asset:part.asset}]}};
 const h=harness({api:{download:async p=>p.endsWith("/attachments")?blob([{...row,content_part:m.input.content_parts[0]}]):blob({id:"team-worker-review",messages:[m]})}});
 await h.view.open(receipt(m));assert(h.view.state.message);assert.match(h.view.html(),/&lt;script&gt;/);assert.match(h.view.html(),/文件引用已核对/);assert(!h.view.html().includes("data-media-artifact"));h.view.dispose();
 const plain=harness();plain.session.messages=[{role:"user",input:{prompt:"text"}}];await plain.view.open(receipt(plain.session.messages[0]));assert.equal(plain.calls.length,1);plain.view.dispose();
});
test("retry explicitly rereads missing checkpoint and can recover once persistence catches up",async()=>{
 const h=harness();h.session.messages=[];await h.view.open(receipt());assert(h.view.state.error);
 h.session.messages=[message];await h.view.open(h.view.state.receipt);assert.equal(h.view.state.error,"");assert(h.view.state.message);h.view.dispose();
});
test("receipt and recovered input do not alias caller-owned JSON",async()=>{
 const h=harness(),e=receipt();await h.view.open(e);e.data.last_user_input.identity.sha256="changed";h.session.messages[1].input.prompt="mutated";
 assert.notEqual(h.view.state.receipt.data.last_user_input.identity.sha256,"changed");assert.notEqual(h.view.state.message.input.prompt,"mutated");message.input.prompt='查看 "图片" 🧪\n原文';h.view.dispose();
});
test("nested model call action selects only canonical request and closing cancels snapshot",async()=>{
 const input=receipt();Object.assign(input,{seq:9,event_id:"run:team:9",causation_id:"run:team:8",node_id:"n",task_id:"t",attempt:"a"});
 const header={schema_version:1,run_id:"team",seq:8,event_id:"run:team:8",event_type:"worker.input.prepared",node_id:"n",task_id:"t",attempt:"a",data:{invocation_id:"i",boundary:"before_worker_run",model_delivery:"not_proven",input_sha256:"a".repeat(64),input_bytes:1,target:{step_id:"s",worker_id:"w",task_id:"t",attempt_id:"a"}}};
 let downloads=0;const calls=P.create({runId:"team",api:{get:async()=>({run_id:"team",events:[input],next_after:9,high_watermark:9,has_more:false,history_complete:true}),download:async()=>{downloads++;throw new Error("missing");}}});
 await calls.open(header);assert.match(calls.html(),/查看输入快照/);await calls.navigate("snapshot","forged");assert.equal(downloads,0);
 await calls.navigate("snapshot",input.event_id);assert.equal(downloads,1);assert.match(calls.html(),/输入快照/);calls.close();assert.equal(calls.snapshot.state.receipt,null);calls.dispose();
});
