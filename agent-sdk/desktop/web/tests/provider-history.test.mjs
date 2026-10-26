import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url), H=require("../core/run-history.js");
const identity={sha256:"a".repeat(64),bytes:123}, clone=structuredClone;
const row=(seq,type,data,cause)=>({schema_version:1,run_id:"run",seq,event_id:"run:run:"+seq,event_type:type,node_id:"node",task_id:"task",attempt:"attempt",causation_id:cause||null,data});
const worker=()=>row(10,"worker.input.prepared",{boundary:"before_worker_run",model_delivery:"not_proven",invocation_id:"worker-1",target:{step_id:"step",worker_id:"worker",task_id:"task",attempt_id:"attempt"},input_sha256:identity.sha256});
const prepared=()=>row(80,"model.input.prepared",{call_id:"call-1",worker_session_id:"child-session",boundary:"before_provider_delegate",model_delivery:"not_proven",representation:"gateway_chat_message_v1_including_input_metadata",request:clone(identity),message_count:3,tool_count:2,image_count:1,last_user_input:{identity:clone(identity),prompt:clone(identity),part_count:1,parts:[{type:"image",asset:{artifact_id:"child-asset",sha256:identity.sha256}}],parts_truncated:false,legacy_attachment_count:0}},"run:run:10");
const completed=(success=true)=>row(200,"model.request.completed",{call_id:"call-1",boundary:"provider_returned",model_delivery:"not_proven",provider_succeeded:success,latency_ms:45,request_id:clone(identity),model:clone(identity),usage:{prompt_tokens:12,completion_tokens:7,total_tokens:19}},"run:run:80");
const interrupted=()=>row(201,"model.request.interrupted",{call_id:"call-1",boundary:"observer_dropped_before_result_receipt",model_delivery:"unknown",provider_succeeded:null},"run:run:80");
const page=events=>({run_id:"run",events,has_more:false,next_before:events[0]?.seq??null,high_watermark:1000,history_complete:true,task_history_complete:true});

test("Provider input summaries expose bounded custody and gateway identity without inventing wire delivery",()=>{
 const view=H.create({runId:"run",api:{}});view.push(prepared());
 for(const text of ["Provider 请求输入已准备","消息 3 条","工具 2 个","图片 1 张","child-session","child-asset","123 字节","不是最终网络请求","不等同于最初建队原话","尚未证明最终模型接收或使用","定位本次 Worker 输入"])assert(view.html().includes(text),text);
 assert(!view.html().includes("data_url"));view.dispose();
});
test("success, failure, absent usage and interrupted observations keep their distinct meanings",()=>{
 for(const success of [true,false]){
  const view=H.create({runId:"run",api:{}});view.push(completed(success));
  assert(view.html().includes(success?"Provider 返回成功":"Provider 返回失败"));assert.match(view.html(),/输入 12 \/ 输出 7 \/ 总计 19 tokens/);assert.match(view.html(),/45 ms/);assert.match(view.html(),/不代表任务已完成/);view.dispose();
 }
 const view=H.create({runId:"run",api:{}}), c=completed();c.data.usage=null;c.data.latency_ms=null;
 view.push(c);view.push(interrupted());assert.match(view.html(),/用量未提供，不能按零计费/);assert.match(view.html(),/耗时 未提供/);assert.match(view.html(),/Provider 观测中断，结果未知/);assert.match(view.html(),/不能据此认定网络已取消/);view.dispose();
});
test("malformed Provider observations remain raw evidence rather than a successful call or navigable proof",()=>{
 const changes=[e=>e.data.call_id="",e=>e.data.model_delivery="delivered",e=>e.data.boundary="wire_sent",e=>e.data.usage.total_tokens=-1,e=>e.data.provider_succeeded="true",e=>e.data.latency_ms=-1,e=>e.data.request_id.sha256="bad"];
 for(const change of changes){const e=completed();change(e);const view=H.create({runId:"run",api:{}});view.push(e);assert.match(view.html(),/Provider 回执字段无效/);assert(!view.html().includes("Provider 返回成功"));assert(!view.html().includes('data-history-action="cause"'));view.dispose();}
 const e=prepared();e.data.last_user_input.parts=Array.from({length:9},()=>({type:"text",identity}));const view=H.create({runId:"run",api:{}});view.push(e);assert.match(view.html(),/回执字段无效/);view.dispose();
});
test("bounded text and media summaries escape content and preserve truncation and missing-session uncertainty",()=>{
 const e=prepared();e.data.call_id='<img src=x onerror="bad">';e.data.worker_session_id=null;
 e.data.last_user_input={identity,prompt:identity,part_count:10,parts:Array.from({length:8},(_,i)=>i===0?{type:"text",identity}:{type:"file",asset:{artifact_id:'<svg onload="bad">',sha256:identity.sha256}}),parts_truncated:true,legacy_attachment_count:2};
 const view=H.create({runId:"run",api:{}});view.push(e);const html=view.html();assert.match(html,/仅展示前 8 项/);assert.match(html,/Worker 会话 未记录/);assert.match(html,/旧附件 2 个/);assert.doesNotMatch(html,/<img|<svg/);assert.match(html,/&lt;svg/);view.dispose();
});
test("cross-page navigation resolves completed request to exact input and then exact Worker header",async()=>{
 const paths=[],replies=[page([completed()]),page([prepared()]),page([worker()])];
 const view=H.create({runId:"run",api:{get:async path=>{paths.push(path);return replies.shift();}}});
 await view.start();await view.navigate("cause","run:run:200");assert.match(paths[1],/before=81$/);assert.equal(view.state.focus,"run:run:80");assert.equal(view.state.follow,false);
 await view.navigate("cause","run:run:80");assert.match(paths[2],/before=11$/);assert.equal(view.state.focus,"run:run:10");
 view.push({...completed(),seq:1001,event_id:"run:run:1001"});assert.equal(view.state.events[0].seq,10);assert.equal(view.state.newer,true);view.dispose();
});
test("a result cannot jump to another call, task, retry, node, event identity or out-of-bound parent",async()=>{
 for(const change of [e=>e.data.call_id="other",e=>e.task_id="other",e=>e.attempt="other",e=>e.node_id="other",e=>e.event_id="run:foreign:80",e=>e.data.request.sha256="bad",e=>{e.seq=82;e.event_id="run:run:82";},e=>e.event_type="message.final"]){
  const parent=prepared();change(parent);let n=0;
  const view=H.create({runId:"run",api:{get:async()=>++n===1?page([completed()]):page([parent])}});
  await view.start();await view.navigate("cause","run:run:200");assert(view.state.error);assert.equal(view.state.events[0].seq,200);assert.equal(view.state.focus,null);view.dispose();
 }
});
test("missing exact cause preserves the visible page and retry retains the original identity requirement",async()=>{
 let response=page([completed()]);const paths=[],view=H.create({runId:"run",api:{get:async path=>{paths.push(path);return response;}}});
 await view.start();response=page([]);await view.navigate("cause","run:run:200");assert.match(view.state.error,/精确前因/);assert.equal(view.state.events[0].seq,200);
 response=page([prepared()]);await view.navigate("retry");assert.match(paths.at(-1),/before=81$/);assert.equal(view.state.focus,"run:run:80");view.dispose();
});
test("foreign, forward, looping and unsafe causal IDs never launch requests",async()=>{
 let requests=0;
 for(const cause of ["run:other:80","run:run:200","run:run:201","run:run:080","run:run:9007199254740991"]){
  const view=H.create({runId:"run",api:{get:async()=>{requests++;return page([]);}}}),e=completed();e.causation_id=cause;view.push(e);await view.navigate("cause",e.event_id);assert(!view.html().includes('data-history-action="cause"'));view.dispose();
 }assert.equal(requests,0);
});
test("late causal reads are aborted by newer navigation or disposal and cannot replace the page",async()=>{
 const pending=[],view=H.create({runId:"run",api:{get:(_,options)=>new Promise(resolve=>pending.push({resolve,signal:options.signal}))}});
 view.push(completed());const first=view.navigate("cause","run:run:200"),latest=view.navigate("latest");assert(pending[0].signal.aborted);
 pending[1].resolve(page([interrupted()]));await latest;pending[0].resolve(page([prepared()]));await first;assert.equal(view.state.events[0].seq,201);
 const second=view.navigate("cause","run:run:201");view.dispose();assert(pending[2].signal.aborted);pending[2].resolve(page([prepared()]));await second;assert.equal(view.state.events.length,0);
});
test("live Provider events share the existing bounded journal and never open another fetch or stream",()=>{
 let reads=0;const view=H.create({runId:"run",api:{get:()=>{reads++;}}});
 for(let i=300;i<500;i++)view.push({...completed(),seq:i,event_id:"run:run:"+i});
 assert.equal(reads,0);assert.equal(view.state.events.length,64);assert.equal((view.html().match(/data-run-event=/g)||[]).length,64);view.dispose();
});

test("an edge's exact Worker input can locate canonical history without substituting another invocation",async()=>{
 const paths=[];let reply=page([worker()]);
 const view=H.create({runId:"run",api:{get:async path=>{paths.push(path);return reply;}}});
 await view.locate(worker());assert.match(paths[0],/before=11$/);assert.equal(view.state.focus,"run:run:10");
 const bad=worker();bad.data.invocation_id="different";reply=page([bad]);await view.locate(worker());
 assert.match(view.state.error,/精确前因/);assert.equal(view.state.events[0].data.invocation_id,"worker-1");
 const count=paths.length;await view.locate(completed());assert.equal(paths.length,count);view.dispose();
});
