import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {createHash,webcrypto} from "node:crypto";
const require=createRequire(import.meta.url),H=require("../core/run-history.js"),S=require("../core/worker-input-snapshot.js");
const hash=v=>createHash("sha256").update(v).digest("hex");
const ident=v=>({sha256:hash(JSON.stringify(v)),bytes:Buffer.byteLength(JSON.stringify(v))});
const message={role:"user",input:{prompt:"root Worker 保存的输入 🧪"}};
function receipt(seq=10){
 return {schema_version:1,run_id:"root",seq,event_id:"run:root:"+seq,event_type:"model.input.prepared",node_id:"root-node",task_id:"root-task",attempt:"a",data:{
  call_id:"request-"+seq,worker_session_id:"root-worker-task-a",boundary:"before_provider_delegate",model_delivery:"not_proven",
  representation:"gateway_chat_message_v1_including_input_metadata",request:ident([]),message_count:1,tool_count:0,image_count:0,
  last_user_input:{identity:ident(message.input),prompt:ident(message.input.prompt),part_count:0,parts:[],parts_truncated:false,legacy_attachment_count:0}
 }};
}
const event=seq=>({schema_version:1,run_id:"root",seq,event_id:"run:root:"+seq,event_type:"member.progress",data:{}});
const page=events=>({run_id:"root",events,next_before:events[0]?.seq??null,has_more:true,high_watermark:300,history_complete:true,task_history_complete:true});
function harness(extra={}){
 const requests=[],v=H.create({runId:"root",api:{get:async path=>{requests.push(path);return page([receipt()]);},download:async(path,options)=>{
  requests.push({path,options});return new Blob([JSON.stringify({id:"root-worker-task-a",messages:[message]})],{type:"application/json"});
 },...extra}});return {v,requests};
}
test("a root Worker with no causal parent or dependency edge can inspect its exact request input",async()=>{
 const {v,requests}=harness();v.push(receipt());assert.match(v.html(),/data-history-action="snapshot"/);
 await v.navigate("snapshot","run:root:10");assert.equal(v.snapshot.state.message.input.prompt,message.input.prompt);
 assert.equal(requests.length,1);assert.equal(requests[0].path,"/session/root-worker-task-a");
 assert.equal(requests[0].options.maxResponseBytes,4*1024*1024);
 assert.match(v.html(),/data-history-action="snapshot-retry"/);assert(!v.snapshot.html().includes("data-edge-input-action"));
 assert(!v.html().includes("data-history-action=\"cause\""));v.dispose();
});
test("invalid, result-only, missing and foreign session witnesses never expose a snapshot action or fetch",async()=>{
 for(const change of [e=>e.data.worker_session_id="other-worker-task",e=>e.data.worker_session_id=null,e=>e.data.last_user_input=null,e=>e.data.last_user_input.identity.sha256="bad",e=>e.event_type="message.final"]){
  const {v,requests}=harness(),e=receipt();change(e);v.push(e);
  assert(!v.html().includes('data-history-action="snapshot"'));await v.navigate("snapshot",e.event_id);
  assert.equal(requests.length,0);v.dispose();
 }
});
test("forged or evicted event IDs cannot access arbitrary session input",async()=>{
 const {v,requests}=harness();v.push(receipt());await v.navigate("snapshot","run:other:10");assert.equal(requests.length,0);
 for(let i=11;i<100;i++)v.push(event(i));
 await v.navigate("snapshot","run:root:10");assert.equal(requests.length,0);assert.equal(v.state.events.length,64);v.dispose();
});
test("explicit timeline navigation closes and aborts a snapshot before another page loads",async()=>{
 for(const action of ["latest","older","newer","retry"]){
  let finish,signal;const {v}=harness({download:(_,o)=>{signal=o.signal;return new Promise(r=>finish=r);}});
  v.push(receipt());v.state.older=true;v.state.newer=true;
  const reading=v.navigate("snapshot","run:root:10");assert(v.snapshot.state.busy);
  await v.navigate(action);assert(signal.aborted);assert.equal(v.snapshot.state.receipt,null);
  finish(new Blob([JSON.stringify({id:"root-worker-task-a",messages:[message]})],{type:"application/json"}));await reading;
  assert.equal(v.snapshot.state.message,null);v.dispose();
 }
});
test("selecting another request cancels late input without changing the selected identity",async()=>{
 const pending=[],{v}=harness({download:(path,o)=>new Promise(resolve=>pending.push({resolve,signal:o.signal}))});
 v.push(receipt());v.push(receipt(11));const first=v.navigate("snapshot","run:root:10"),second=v.navigate("snapshot","run:root:11");
 assert(pending[0].signal.aborted);const blob=()=>new Blob([JSON.stringify({id:"root-worker-task-a",messages:[message]})],{type:"application/json"});
 pending[1].resolve(blob());await second;pending[0].resolve(blob());await first;
 assert.equal(v.snapshot.state.receipt.event_id,"run:root:11");assert(v.snapshot.state.message);v.dispose();
});
test("SSE window eviction preserves only one explicitly identified snapshot and does not fetch again",async()=>{
 const {v,requests}=harness();v.push(receipt());await v.navigate("snapshot","run:root:10");
 for(let i=11;i<1000;i++)v.push(event(i));
 assert.equal(v.state.events.length,64);assert.equal(requests.length,1);
 assert.match(v.html(),/选中请求已移出当前事件窗口/);assert.match(v.html(),/仍属于 run:root:10/);
 assert.equal(v.snapshot.state.receipt.event_id,"run:root:10");await v.navigate("snapshot-close");assert.equal(v.snapshot.state.receipt,null);v.dispose();
});
test("missing checkpoint is explicit and retry reads only the same immutable receipt",async()=>{
 let ready=false,downloads=0;const {v}=harness({download:async()=>{downloads++;return new Blob([JSON.stringify({id:"root-worker-task-a",messages:ready?[message]:[]})],{type:"application/json"});}});
 v.push(receipt());await v.navigate("snapshot","run:root:10");assert.match(v.html(),/未找到摘要完全一致/);
 ready=true;await v.navigate("snapshot-retry");assert.equal(downloads,2);assert(v.snapshot.state.message);
 await v.navigate("snapshot-close");await v.navigate("snapshot-retry");assert.equal(downloads,2);v.dispose();
});
test("disposal cancels snapshot and history and refuses all later action requests",async()=>{
 let finish,signal;const {v,requests}=harness({download:(_,o)=>{signal=o.signal;return new Promise(r=>finish=r);}});
 v.push(receipt());const reading=v.navigate("snapshot","run:root:10");v.dispose();assert(signal.aborted);
 finish(new Blob(["{}"],{type:"application/json"}));await reading;await v.navigate("snapshot-retry");await v.navigate("latest");
 assert.equal(v.snapshot.state.message,null);assert.equal(requests.length,0);
});
test("timeline witness validation uses UTF-8 bytes and does not accept an oversized Chinese payload",()=>{
 const {v}=harness(),e=event(1);e.data.text="中".repeat(23000);assert(JSON.stringify(e).length<65536);
 assert.throws(()=>v.push(e),/大小/);v.dispose();
});
test("snapshot availability is the same shared validation in both entry surfaces",()=>{
 const e=receipt();assert(S.canInspect(e,"root"));assert(!S.canInspect(e,"other"));
 e.data.last_user_input=null;assert(!S.canInspect(e,"root"));
});
