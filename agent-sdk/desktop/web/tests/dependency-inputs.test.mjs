import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url),I=require("../core/dependency-inputs.js"),D=require("../core/dependency-inspector.js");
const source={task_id:"src",worker:"m-source"},target={task_id:"dst",worker:"m-target",depends_on:["src"]};
const hash="a".repeat(64),clone=value=>structuredClone(value);
const receiver={step_id:"dst",worker_id:"m-target",task_id:"assigned-task",attempt_id:"attempt-consumer-2",phase_epoch:2};
const common={invocation_id:"invocation-2",target:receiver,boundary:"before_worker_run",model_delivery:"not_proven"};
const row=(seq,type,data)=>({schema_version:1,run_id:"run",seq,event_id:"run:run:"+seq,event_type:type,timestamp:"now",task_id:receiver.task_id,attempt:receiver.attempt_id,artifact_refs:[],data});
const header=(seq=40)=>row(seq,"worker.input.prepared",{...common,input_sha256:hash,input_bytes:123,source_count:2,context_encoding:"structured_team_context",input_representation:"serde_json_worker_value"});
const input=(seq=41)=>({...row(seq,"worker.input.source",{...common,source:{producer_step_id:"src",producer_worker_id:"m-source",producer_task_id:"producer-task",producer_attempt_id:"attempt-producer-7",artifact_id:"asset-v7",artifact_version:7,artifact_sha256:hash,preview_bytes:18,preview_sha256:hash,preview_truncated:true}}),causation_id:"run:run:40",artifact_refs:["asset-v7"]});
const page=(events,more=false)=>({run_id:"run",events,next_before:events[0]?.seq??null,high_watermark:1000,has_more:more,history_complete:true,task_history_complete:true});
function view(api){const v=I.create({runId:"run",api});v.reset(source,target);return v;}

test("edge input view reads canonical history lazily and preserves exact producer/consumer retry and version",async()=>{
 const paths=[],v=view({get:async path=>{paths.push(path);return page([header(),input()]);}});
 assert.equal(paths.length,0);assert.match(v.html(),/查看本边输入准备记录/);
 await v.navigate("open");assert.equal(paths.length,1);assert.match(paths[0],/timeline\?format=json&limit=64&before=0$/);
 assert.deepEqual(v.state.events.map(e=>e.seq),[40,41]);const html=v.html();
 for(const text of ["attempt-consumer-2","attempt-producer-7","输入时版本 7","invocation-2","18 字节","尚未证明最终模型接收或使用","不代表都属于本边","run:run:40"])assert(html.includes(text),text);
 assert(!html.includes("data_url"));v.dispose();
});

test("source or consumer name coincidence never attributes a receipt to an edge; opaque/legacy source is not guessed",async()=>{
 const variants=[input(),input(),input(),input(),input(),input()];
 variants[0].data.target={...receiver,step_id:"other"};variants[1].data.target={...receiver,worker_id:"other"};
 variants[2].data.source.producer_step_id="other";variants[3].data.source.producer_worker_id="other";
 delete variants[4].data.source.producer_step_id;variants[5].event_type="tool.finished";
 variants.forEach((e,i)=>{e.seq=100+i;e.event_id="run:run:"+e.seq;assert.equal(I.related(e,source,target),false);});
 const v=view({get:async()=>page(variants,true)});await v.navigate("open");assert.equal(v.state.events.length,0);
 assert.equal(v.state.nextBefore,100);assert.equal(v.state.more,true);assert.match(v.html(),/不能据此判断未传递输入/);v.dispose();
});

test("global cursor advances over pages without matching input and retains only one bounded page",async()=>{
 const paths=[],replies=[page([row(300,"message.final",{})],true),page([input(100)],true),page([header(40)],false)];
 const v=view({get:async p=>{paths.push(p);return replies.shift();}});await v.navigate("open");
 assert.equal(v.state.events.length,0);await v.navigate("older");assert.equal(v.state.events[0].seq,100);
 await v.navigate("older");assert.equal(v.state.events[0].seq,40);assert.equal(v.state.more,false);
 assert.match(paths[1],/before=300$/);assert.match(paths[2],/before=100$/);v.dispose();
});

test("source records can locate a header outside the current page by exact cause and invocation",async()=>{
 const paths=[],v=view({get:async path=>{paths.push(path);return page(paths.length===1?[input(500)]:[header()]);}});
 await v.navigate("open");await v.navigate("header","run:run:500");assert.match(paths[1],/before=41$/);
 assert.equal(v.state.focus,"run:run:40");assert.match(v.html(),/data-edge-input-event="run:run:40" open/);v.dispose();
 const bad=header();bad.data.invocation_id="another-invocation";
 const broken=view({get:async path=>page(path.endsWith("before=0")?[input(500)]:[bad])});
 await broken.navigate("open");await broken.navigate("header","run:run:500");
 assert.equal(broken.state.events[0].seq,500);assert.match(broken.state.error,/精确准备头/);broken.dispose();
});

test("invalid page identity, cursor, order, total bytes and event size preserve the last accepted page",async()=>{
 let reply=page([header(),input()]);const v=view({get:async()=>clone(reply)});await v.navigate("open");
 const invalid=[{...reply,run_id:"foreign"},{...reply,next_before:999},{...reply,high_watermark:40},page([input(),header()]),
  page([],true),page(Array.from({length:65},(_,i)=>row(i+1,"message.delta",{}))),page([row(50,"message.delta",{text:"大".repeat(23000)})]),
  {...reply,extra:"大".repeat(400000)},page([{...input(),event_id:"wrong"}]),page([{...input(),schema_version:2}])];
 for(const candidate of invalid){reply=candidate;await v.navigate("latest");assert(v.state.error);assert.deepEqual(v.state.events.map(e=>e.seq),[40,41]);}
 reply=page([header(),input()]);await v.navigate("older");assert.match(v.state.error,/边界/);v.dispose();
});

test("malformed matching receipts are rejected rather than presented as verified inputs",async()=>{
 for(const change of [e=>e.data.model_delivery="delivered",e=>e.attempt="foreign",e=>e.data.invocation_id="",e=>e.artifact_refs=["unrelated"],e=>e.causation_id="run:foreign:40",e=>e.causation_id="run:run:42",e=>e.data.source.preview_sha256="invalid",e=>e.data.source.preview_bytes=-1,e=>e.data.source.preview_truncated="false"]){
  const bad=input();change(bad);const v=view({get:async()=>page([bad])});await v.navigate("open");assert(v.state.error);assert.equal(v.state.events.length,0);v.dispose();
 }
 for(const change of [e=>e.data.input_sha256="bad",e=>e.data.input_bytes=-1,e=>e.data.source_count=-1,e=>e.data.context_encoding="guessed",e=>e.data.input_representation="http_request"]){
  const bad=header();change(bad);const v=view({get:async()=>page([bad])});await v.navigate("open");assert(v.state.error);v.dispose();
 }
});

test("empty terminal page preserves current input window and legacy history has explicit uncertainty",async()=>{
 let next=page([input()],true);const v=view({get:async()=>next});await v.navigate("open");
 next=page([]);await v.navigate("older");assert.equal(v.state.events[0].seq,41);assert.equal(v.state.more,false);
 const old=input();delete old.data.source.preview_truncated;old.data.source.artifact_version=null;
 next={...page([old]),history_complete:false,task_history_complete:false};await v.navigate("latest");
 assert.match(v.html(),/早期历史不完整/);assert.match(v.html(),/截断 未记录/);assert.match(v.html(),/输入时版本 未记录/);v.dispose();
});

test("late input pages cannot survive switching edges, closing, or disposing",async()=>{
 const pending=[],v=view({get:(path,options)=>new Promise(resolve=>pending.push({resolve,signal:options.signal}))});
 const first=v.navigate("open");v.reset({...source,task_id:"other"},{...target,depends_on:["other"]});assert(pending[0].signal.aborted);
 const second=v.navigate("open");pending[1].resolve(page([]));await second;pending[0].resolve(page([input()]));await first;assert.equal(v.state.events.length,0);
 const third=v.navigate("latest");await v.navigate("close");assert(pending[2].signal.aborted);pending[2].resolve(page([input()]));await third;assert.equal(v.state.open,false);
 const fourth=v.navigate("open");v.dispose();assert(pending[3].signal.aborted);pending[3].resolve(page([input()]));await fourth;assert.equal(v.state.events.length,0);
});

test("live notices coalesce into a reread; historical navigation cancels a queued latest refresh",async()=>{
 let finish,calls=0;const v=view({get:()=>++calls===1?new Promise(resolve=>finish=resolve):Promise.resolve(page(calls>=3?[header()]:[input()],true))});
 const first=v.navigate("open");for(let i=0;i<30;i++)v.notice(input());finish(page([header()],true));await first;
 await new Promise(resolve=>setTimeout(resolve,300));assert.equal(calls,2);assert.equal(v.state.events[0].seq,41);
 v.notice(input());await v.navigate("older");await new Promise(resolve=>setTimeout(resolve,300));assert.equal(calls,3);assert.equal(v.state.before,41);
 v.notice(input());await new Promise(resolve=>setTimeout(resolve,300));assert.equal(calls,3);assert.equal(v.state.newRecords,true);v.dispose();
});

test("receipt fields are escaped and missing fields remain unknown",async()=>{
 const hostile=input();hostile.data.invocation_id='<img src=x onerror="bad">';hostile.data.source.producer_attempt_id="<script>bad</script>";
 delete hostile.data.source.preview_truncated;const v=view({get:async()=>page([hostile])});await v.navigate("open");
 assert.doesNotMatch(v.html(),/<img|<script>/);assert.match(v.html(),/&lt;img/);assert.match(v.html(),/截断 未记录/);v.dispose();
});

test("edge inspector integrates receipt reset, live notice and teardown without adding another event stream",async()=>{
 let reads=0;const view=D.create({runId:"run",api:{get:async path=>{reads++;return path.includes("/timeline")?page([input()]):{team_id:"run",handoffs:[],next_handoff_before:null,handoffs_has_more:false};}}});
 await view.select(source,target);assert.equal(reads,1);await view.inputs.navigate("open");assert.equal(reads,2);
 assert.match(view.html(),/本边上游输入/);view.notice(input());view.refreshTasks([source,{...target,depends_on:[]}]);
 assert.equal(view.inputs.state.open,false);assert.equal(view.inputs.state.events.length,0);await new Promise(resolve=>setTimeout(resolve,300));assert.equal(reads,2);view.dispose();
});


test("retry and fan-out history retains individual invocation/version and only the selected consumer",async()=>{
 const oldHeader=header(10),oldInput=input(11),other=input(42);
 for(const e of [oldHeader,oldInput]){e.data=clone(e.data);e.data.invocation_id="invocation-1";e.data.target={...receiver,attempt_id:"attempt-consumer-1"};e.attempt="attempt-consumer-1";}
 oldInput.causation_id="run:run:10";oldInput.data.source.artifact_version=3;oldInput.data.source.producer_attempt_id="attempt-producer-3";
 other.data=clone(other.data);other.data.target={...receiver,step_id:"fanout-sibling",worker_id:"m-sibling"};
 const v=view({get:async()=>page([oldHeader,oldInput,header(),input(),other])});await v.navigate("open");
 assert.deepEqual(v.state.events.map(e=>e.seq),[10,11,40,41]);
 for(const value of ["invocation-1","invocation-2","attempt-consumer-1","attempt-consumer-2","输入时版本 3","输入时版本 7"])assert(v.html().includes(value));
 assert(!v.html().includes("m-sibling"));v.dispose();
});

test("edge Worker header opens the same run journal by exact receipt; stale or source-only records cannot redirect it",async()=>{
 const inspected=[],v=I.create({runId:"run",api:{get:async()=>page([header(),input()])},inspect:e=>inspected.push(e)});
 v.reset(source,target);await v.navigate("open");assert.match(v.html(),/定位到运行记录/);
 await v.navigate("timeline","run:run:41");assert.equal(inspected.length,0);
 await v.navigate("timeline","run:run:40");assert.equal(inspected[0].event_id,"run:run:40");assert.equal(inspected[0].data.invocation_id,"invocation-2");
 v.reset();await v.navigate("timeline","run:run:40");assert.equal(inspected.length,1);v.dispose();
});
