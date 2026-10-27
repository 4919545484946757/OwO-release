import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require=createRequire(import.meta.url);
const H=require("../core/run-history.js");
const row=(seq)=>({schema_version:1,run_id:"a",seq,event_id:"run:a:"+seq,event_type:"message.final",data:{payload_preview:{text:"message "+seq}}});
const range=(start,count=64)=>Array.from({length:count},(_,i)=>row(start+i));
const page=(events,more=false)=>({run_id:"a",events,has_more:more,next_before:events[0]?.seq??null,high_watermark:200,history_complete:true,task_history_complete:true});

test("twenty thousand live observations retain and render at most 64 canonical events",()=>{
 const view=H.create({runId:"a",api:{}});
 for(let i=1;i<=20000;i++)view.push(row(i));
 assert.equal(view.state.events.length,64);assert.equal(view.state.events[0].seq,19937);
 assert.equal((view.html().match(/data-run-event=/g)||[]).length,64);
 assert.equal(view.state.older,true);view.dispose();assert.equal(view.state.events.length,0);
});

test("older/newer navigation uses exact page edges, while live events leave history stable",async()=>{
 const paths=[];const responses=[page(range(100),true),page(range(36),true),{...page(range(100)),next_after:163}];
 const view=H.create({runId:"a",api:{get:async path=>{paths.push(path);return responses.shift();}}});
 await view.start();await view.navigate("older");
 assert.match(paths[1],/before=100$/);assert.equal(view.state.events[0].seq,36);
 view.push(row(1000));assert.equal(view.state.events[0].seq,36);assert.equal(view.state.newer,true);
 await view.navigate("newer");assert.match(paths[2],/after=99$/);assert.equal(view.state.events[0].seq,100);
 assert.equal(view.state.follow,false);view.dispose();
});

test("latest page load merges concurrent SSE observations without duplicates or growth",async()=>{
 let finish;const view=H.create({runId:"a",api:{get:()=>new Promise(resolve=>finish=resolve)}});
 const loading=view.start();for(let i=100;i<=300;i++)view.push(row(i));
 finish(page(range(200)));await loading;
 assert.equal(view.state.events.length,64);assert.equal(view.state.events.at(-1).seq,300);
 assert.equal(new Set(view.state.events.map(e=>e.event_id)).size,64);view.dispose();
});

test("empty boundary keeps current page and disables the corresponding direction",async()=>{
 const replies=[page(range(1),true),page([])];
 const view=H.create({runId:"a",api:{get:async()=>replies.shift()}});
 await view.start();await view.navigate("older");assert.equal(view.state.events.length,64);
 assert.equal(view.state.older,false);view.dispose();
});

test("oversized, foreign and malformed pages show an explicit error instead of replacing data",async()=>{
 const replies=[page(range(100),true),page([row(4),row(3)])];
 const view=H.create({runId:"a",api:{get:async()=>replies.shift()}});
 await view.start();await view.navigate("older");assert.match(view.state.error,/顺序/);assert.equal(view.state.events[0].seq,100);
 assert.throws(()=>view.push({...row(200),run_id:"foreign"}),/身份/);
 assert.throws(()=>view.push({...row(200),data:{text:"x".repeat(70000)}}),/大小/);view.dispose();
});

test("disposing aborts a pending history read and ignores late responses",async()=>{
 let finish,signal;const view=H.create({runId:"a",api:{get:(_,o)=>{signal=o.signal;return new Promise(resolve=>finish=resolve);}}});
 const reading=view.start();view.dispose();assert.equal(signal.aborted,true);finish(page(range(100)));await reading;
 assert.equal(view.state.events.length,0);
});

test("replacing an in-flight read cancels it; stale reply cannot overwrite latest",async()=>{
 const pending=[];const view=H.create({runId:"a",api:{get:(_,o)=>new Promise(resolve=>pending.push({resolve,signal:o.signal}))}});
 const first=view.start();const second=view.navigate("latest");assert.equal(pending[0].signal.aborted,true);
 pending[1].resolve(page(range(100)));await second;pending[0].resolve(page(range(1)));await first;
 assert.equal(view.state.events[0].seq,100);view.dispose();
});

test("public preview and real causal identities are escaped and incomplete history stays explicit",async()=>{
 const event={...row(3),task_id:"t",attempt:"attempt",causation_id:"run:a:1",data:{payload_complete:false,payload_preview:{text:"<script>bad</script>"}}};
 const view=H.create({runId:"a",api:{get:async()=>({...page([event]),history_complete:false})}});
 await view.start();const html=view.html();assert.match(html,/&lt;script&gt;/);assert.doesNotMatch(html,/<script>/);
 assert.match(html,/run:a:1/);assert.match(html,/早期历史不完整/);assert.match(html,/公开预览不完整/);view.dispose();
});


test("prepared Worker input history keeps invocation, source version and retry identity without claiming model receipt",async()=>{
 const prepared={...row(1),event_type:"worker.input.prepared",task_id:"reader",attempt:"attempt-2",data:{target:{step_id:"reader",worker_id:"m-reader"},input_sha256:"a".repeat(64),source_count:1,context_encoding:"structured_team_context",model_delivery:"not_proven"}};
 const source={...row(2),event_type:"worker.input.source",task_id:"reader",attempt:"attempt-2",causation_id:"run:a:1",artifact_refs:["asset-v2"],data:{target:prepared.data.target,source:{producer_step_id:"writer",producer_attempt_id:"writer-attempt-1",artifact_id:"asset-v2",artifact_version:2,preview_bytes:30,preview_sha256:"b".repeat(64),preview_truncated:true}}};
 const view=H.create({runId:"a",api:{get:async()=>page([prepared,source])}});
 await view.start();const html=view.html();
 assert.match(html,/Worker 输入已准备/);assert.match(html,/Worker 上游输入记录/);
 assert.match(html,/尚未证明模型已经接收或使用/);assert.match(html,/尚未编译为模型提示词/);
 assert.match(html,/产物 asset-v2 · 版本 2/);assert.match(html,/上游尝试 writer-attempt-1/);
 assert.match(html,/是否截断：是/);assert.match(html,/run:a:1/);assert.match(html,/attempt-2/);
 view.dispose();
});

test("input receipt fields are escaped and missing truncation does not become a complete preview",()=>{
 const view=H.create({runId:"a",api:{}});
 view.push({...row(1),event_type:"worker.input.source",data:{target:{step_id:'<img src=x onerror=alert(1)>',worker_id:'<script>x</script>'},source:{artifact_id:'<svg onload=alert(1)>'}}});
 const html=view.html();assert.doesNotMatch(html,/<img|<svg|<script>/);assert.match(html,/&lt;img/);
 assert.match(html,/是否截断：未记录/);assert.match(html,/版本 未记录/);view.dispose();
});
