import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url), C=require("../core/provider-call-inspector.js"), I=require("../core/dependency-inputs.js");
const hash="a".repeat(64), ident={sha256:hash,bytes:123};
const row=(seq,type,data={},cause=null)=>({schema_version:1,run_id:"run",seq,event_id:"run:run:"+seq,node_id:"node",task_id:"task",attempt:"attempt",event_type:type,causation_id:cause,data});
const header=()=>row(1,"worker.input.prepared",{invocation_id:"inv-1",boundary:"before_worker_run",model_delivery:"not_proven",target:{step_id:"dst",worker_id:"worker",task_id:"task",attempt_id:"attempt"},input_sha256:hash,input_bytes:11,source_count:0,input_representation:"serde_json_worker_value",context_encoding:"structured_team_context"});
const begin=(seq=2,id="call")=>row(seq,"model.input.prepared",{call_id:id,boundary:"before_provider_delegate",model_delivery:"not_proven",representation:"gateway_chat_message_v1_including_input_metadata",request:ident,message_count:1,tool_count:0,image_count:0,last_user_input:null},"run:run:1");
const end=(seq=3,parent=2,id="call",success=true)=>row(seq,"model.request.completed",{call_id:id,boundary:"provider_returned",model_delivery:"not_proven",provider_succeeded:success,usage:null,latency_ms:null},"run:run:"+parent);
const stop=(seq=3,parent=2,id="call")=>row(seq,"model.request.interrupted",{call_id:id,boundary:"observer_dropped_before_result_receipt",model_delivery:"unknown",provider_succeeded:null},"run:run:"+parent);
const page=(events,after=1,high=events.at(-1)?.seq??after,more=false)=>({run_id:"run",events,next_after:events.at(-1)?.seq??after,high_watermark:high,has_more:more,history_complete:true,task_history_complete:true});
function fromRows(rows,options={}){
 const paths=[],v=C.create({runId:"run",api:{get:async path=>{
  paths.push(path);const after=Number(new URL("http://fixture"+path).searchParams.get("after")),remaining=rows.filter(e=>e.seq>after),events=remaining.slice(0,64);
  return page(events,after,rows.at(-1)?.seq??1,remaining.length>64);
 }},...options});return {v,paths};
}
test("pairs only this exact invocation and keeps failed/interrupted/unknown results distinct",async()=>{
 const foreign=begin(4,"other");foreign.causation_id="run:run:999";
 const {v}=fromRows([begin(),end(),foreign,end(5,4,"other"),begin(6,"failed"),end(7,6,"failed",false),begin(8,"interrupted"),stop(9,8,"interrupted"),begin(10,"pending")]);
 await v.open(header());assert.equal(v.state.calls.length,4);assert.equal(v.state.calls[0].result.seq,3);
 assert.equal(v.state.calls.at(-1).result,null);assert.equal(v.state.unattributedResults,1);
 for(const text of ["Provider 返回成功","Provider 返回失败","Provider 观测中断，结果未知","尚无结果回执","截至已读游标","不计作本次结果"])assert(v.html().includes(text),text);
 assert(!v.html().includes("other ·"));v.dispose();
});
test("reads at most four bounded pages per action and preserves manual continuation across nonmatching history",async()=>{
 const rows=Array.from({length:256},(_,i)=>row(i+2,"message.final")),b=begin(258),e=end(259,258);rows.push(b,e);
 const {v,paths}=fromRows(rows);await v.open(header());assert.equal(paths.length,4);assert.equal(v.state.cursor,257);assert.equal(v.state.more,true);assert.equal(v.state.calls.length,0);
 await v.navigate("continue");assert.equal(paths.length,5);assert.match(paths[4],/after=257$/);assert.equal(v.state.calls[0].result.seq,259);assert.equal(v.state.more,false);v.dispose();
});
test("results can arrive on a later page without changing the request or retry identity",async()=>{
 const rows=[begin(),...Array.from({length:130},(_,i)=>row(i+3,"tool.started")),end(133)];
 const {v}=fromRows(rows);await v.open(header());assert.equal(v.state.calls.length,1);assert.equal(v.state.calls[0].input.seq,2);assert.equal(v.state.calls[0].result.seq,133);v.dispose();
});
test("long runs retain only 64 request/result pairs and explicitly disclose eviction",async()=>{
 const rows=Array.from({length:1500},(_,i)=>[begin(i*2+2,"c"+i),end(i*2+3,i*2+2,"c"+i)]).flat();
 const {v}=fromRows(rows);await v.open(header());while(v.state.more)await v.navigate("continue");
 assert.equal(v.state.calls.length,64);assert.equal(v.state.evicted,1436);assert.equal(v.state.calls[0].input.data.call_id,"c1436");
 assert(v.state.calls.every(c=>c.result));assert.equal((v.html().match(/data-provider-call=/g)||[]).length,64);assert.match(v.html(),/不能据此判断全部调用已完成/);v.dispose();
});
test("conflicting call, attempt, node and causal result identities reject the entire new page",async()=>{
 for(const change of [e=>e.data.call_id="wrong",e=>e.attempt="wrong",e=>e.node_id="wrong",e=>e.task_id="wrong",e=>e.causation_id="run:run:99",e=>e.data.provider_succeeded="true"]){
  const bad=end();change(bad);let n=0;
  const v=C.create({runId:"run",api:{get:async()=>++n===1?page([begin()]):page([bad],2)}});
  await v.open(header());await v.navigate("refresh");assert(v.state.error);assert.equal(v.state.cursor,2);assert.equal(v.state.calls[0].result,null);v.dispose();
 }
});
test("duplicate request IDs and multiple result receipts never silently overwrite prior evidence",async()=>{
 for(const events of [[begin(),begin(3)],[begin(),end(),stop(4)]]){
  const {v}=fromRows(events);await v.open(header());assert(v.state.error);assert.equal(v.state.cursor,1);assert.equal(v.state.calls.length,0);v.dispose();
 }
});
test("malformed pages, byte limits, sequence regressions and nonadvancing more flag preserve accepted data",async()=>{
 const original=page([begin()]);let reply=original;const v=C.create({runId:"run",api:{get:async()=>reply}});await v.open(header());
 const invalid=[{...page([end()],2),run_id:"foreign"},{...page([end()],2),next_after:88},{...page([end()],2),high_watermark:2},
  page([end(4),end(3)],2),page([],2,2,true),page(Array.from({length:65},(_,i)=>row(i+3,"message.final")),2),page([row(3,"message.final",{text:"中".repeat(23000)})],2),{...page([end()],2),extra:"中".repeat(1500000)}];
 for(const bad of invalid){reply=bad;await v.navigate("refresh");assert(v.state.error);assert.equal(v.state.cursor,2);assert.equal(v.state.calls[0].result,null);}
 reply=page([end()],2);await v.navigate("retry");assert.equal(v.state.error,"");assert.equal(v.state.calls[0].result.seq,3);v.dispose();
});
test("legacy incomplete history and missing request parents never become fabricated success",async()=>{
 const v=C.create({runId:"run",api:{get:async()=>({...page([end(3,2)]),history_complete:false,task_history_complete:false})}});
 await v.open(header());assert.equal(v.state.calls.length,0);assert.equal(v.state.unattributedResults,1);assert.match(v.html(),/早期历史不完整/);assert.match(v.html(),/这不证明没有模型调用/);v.dispose();
});
test("new notices coalesce into canonical rereads and a stale journal uses bounded backoff",async()=>{
 const timers=[],cleared=[];let reads=0,rows=[begin()];
 const v=C.create({runId:"run",setTimeout:(fn,ms)=>{timers.push({fn,ms});return timers.length;},clearTimeout:id=>cleared.push(id),api:{get:async path=>{
  reads++;const after=Number(new URL("http://fixture"+path).searchParams.get("after"));return page(rows.filter(e=>e.seq>after),after,rows.at(-1).seq);
 }}});
 await v.open(header());for(let n=0;n<50;n++)v.notice(end());assert.equal(timers.length,1);assert.equal(timers[0].ms,250);
 timers[0].fn();await new Promise(resolve=>setImmediate(resolve));assert.equal(reads,2);assert.equal(timers[1].ms,500);assert.equal(v.state.newRecords,true);
 rows=[begin(),end()];timers[1].fn();await new Promise(resolve=>setImmediate(resolve));assert.equal(v.state.calls[0].result.seq,3);assert.equal(v.state.newRecords,false);v.dispose();
});
test("new events do not trigger automatic unbounded scanning while manual continuation is needed",async()=>{
 const rows=Array.from({length:400},(_,i)=>row(i+2,"message.final"));const timers=[],{v,paths}=fromRows(rows,{setTimeout:(fn,ms)=>{timers.push({fn,ms});return 1;},clearTimeout:()=>{}});
 await v.open(header());v.notice(begin(900));assert.equal(paths.length,4);assert.equal(timers.length,0);assert.equal(v.state.newRecords,true);v.dispose();
});
test("switching invocation, closing and disposal abort pending requests and ignore late results",async()=>{
 const pending=[],v=C.create({runId:"run",api:{get:(_,o)=>new Promise(resolve=>pending.push({resolve,signal:o.signal}))}});
 const first=v.open(header()),nextHeader=header();nextHeader.seq=10;nextHeader.event_id="run:run:10";nextHeader.data.invocation_id="inv-2";
 const second=v.open(nextHeader);assert(pending[0].signal.aborted);pending[1].resolve(page([],10));await second;pending[0].resolve(page([begin(),end()]));await first;assert.equal(v.state.header.seq,10);assert.equal(v.state.calls.length,0);
 const third=v.navigate("refresh");v.close();assert(pending[2].signal.aborted);pending[2].resolve(page([],10));await third;assert.equal(v.state.header,null);
 const fourth=v.open(header());v.dispose();assert(pending[3].signal.aborted);pending[3].resolve(page([begin(),end()]));await fourth;assert.equal(v.state.calls.length,0);
});
test("invalid headers never fetch and labels safely escape user-controlled identities",async()=>{
 let reads=0;const v=C.create({runId:"run",api:{get:async()=>{reads++;return page([]);}}}),bad=header();bad.run_id="other";
 await v.open(bad);assert(v.state.error);assert.equal(reads,0);v.dispose();
 const h=header();h.data.invocation_id='<img src=x onerror="x">';h.data.target.worker_id="<svg onload=x>";
 const {v:escaped}=fromRows([begin()]);await escaped.open(h);assert.doesNotMatch(escaped.html(),/<img|<svg/);assert.match(escaped.html(),/&lt;img/);escaped.dispose();
});
test("edge reset and close tear down the nested call inspector and all reads use the shared timeline API",async()=>{
 const paths=[],h=header(),v=I.create({runId:"run",api:{get:async path=>{
  paths.push(path);if(path.includes("before="))return {run_id:"run",events:[h],next_before:1,high_watermark:3,has_more:false,history_complete:true,task_history_complete:true};
  return page([begin(),end()]);
 }}});
 v.reset({task_id:"src",worker:"upstream"},{task_id:"dst",worker:"worker",depends_on:["src"]});await v.navigate("open");await v.navigate("provider-open",h.event_id);
 assert.equal(v.calls.state.calls[0].result.seq,3);assert.match(v.html(),/本次 Worker 模型调用/);assert(paths.every(p=>p.includes("/timeline?format=json&limit=64&")));
 await v.navigate("close");assert.equal(v.calls.state.header,null);v.reset();v.dispose();
});

test("a valid large page within the 64-event contract is accepted without weakening each-event limits",async()=>{
 const rows=[begin(),...Array.from({length:62},(_,i)=>row(i+3,"message.final",{text:"x".repeat(32000)})),end(65)];
 const {v}=fromRows(rows);await v.open(header());assert.equal(v.state.error,"");assert.equal(v.state.calls[0].result.seq,65);v.dispose();
});
