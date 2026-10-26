import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url),D=require("../core/dependency-inspector.js");
const source={task_id:"src",worker:"m-writer",role:"任意上游",status:"Succeeded"};
const target={task_id:"dst",worker:"m-reader",role:"任意下游",status:"Pending",depends_on:["src"]};
const record=(n,more={})=>({handoff_id:"run:src:v"+n,from_member:"m-writer",to_member:"m-reader",completed_summary:"真实交接 "+n,output_artifact_refs:["artifact-"+n],created_at:"now",...more});
const page=(records,more=false)=>({team_id:"run",handoffs:records,next_handoff_before:records[0]?.handoff_id??null,handoffs_has_more:more});
test("exact host identities locate declarations; worker name or prefix coincidence cannot fabricate an edge",()=>{
 assert.equal(D.related(record(1),"run",source,target),true);
 for(const h of [
  record(1,{handoff_id:"run:src:other:v1"}),record(1,{handoff_id:"foreign:src:v1"}),
  record(1,{handoff_id:"legacy-unbound"}),record(1,{from_member:"m-other"}),record(1,{to_member:"m-other"})
 ])assert.equal(D.related(h,"run",source,target),false);
 assert.equal(D.related(record(1,{to_member:"*"}),"run",source,target),true);
 assert.equal(D.related(record(1,{handoff_id:"run:src:manual:123"}),"run",source,target),true);
});
test("edge history replaces bounded pages and retains authoritative boundaries even when page has no matching declarations",async()=>{
 const paths=[],replies=[page([record(4),record(5)],true),page([record(2,{handoff_id:"run:other:v2"})],true),page([record(1)],false)];
 const view=D.create({runId:"run",api:{get:async path=>{paths.push(path);return replies.shift();}},cursors:()=>({"run-worker-x":7})});
 await view.select(source,target);assert.equal(view.state.records.length,2);
 await view.navigate("older");assert.equal(view.state.records.length,0);assert.equal(view.state.nextBefore,"run:other:v2");
 await view.navigate("older");assert.equal(view.state.records[0].handoff_id,"run:src:v1");
 assert.equal(view.state.more,false);assert.equal(paths.length,3);
 assert.match(paths[1],/handoff_before=run%3Asrc%3Av4/);
 assert.match(paths[2],/handoff_before=run%3Aother%3Av2/);
 assert.match(view.html(),/不表示下游模型已收到/);view.dispose();
});
test("metadata reads only recorded ArtifactRef, validates team/task identity, and escapes public text",async()=>{
 const paths=[];
 const view=D.create({runId:"run",api:{get:async path=>{
  paths.push(path);
  return path.startsWith("/teams")?page([record(1,{completed_summary:'<img onerror="x">',output_artifact_refs:["artifact-1"]})]):{
   artifact_id:"artifact-1",team_id:"run",task_id:"src",attempt_id:"actual-attempt",version:2,sha256:"actual-hash",size_bytes:17,format:"text",evidence_refs:["actual-evidence"]};
 }}});
 await view.select(source,target);await view.inspectArtifact("unrecorded");assert.equal(paths.length,1);
 await view.inspectArtifact("artifact-1");assert.equal(view.state.artifact.attempt_id,"actual-attempt");
 assert.match(view.html(),/actual-evidence/);assert.doesNotMatch(view.html(),/<img/);
 assert.match(view.html(),/&lt;img/);view.dispose();
});
test("foreign metadata and oversized pages report failure without presenting them as evidence",async()=>{
 let reply=page([record(1)]);
 const view=D.create({runId:"run",api:{get:async path=>path.startsWith("/teams")?reply:{artifact_id:"artifact-1",team_id:"foreign",task_id:"src"}}});
 await view.select(source,target);await view.inspectArtifact("artifact-1");
 assert.equal(view.state.artifact,null);assert.match(view.state.error,/身份/);
 reply=page(Array.from({length:65},(_,i)=>record(i+1)));
 await view.navigate("latest");assert.equal(view.state.records.length,1);assert.match(view.state.error,/协议或大小/);
 reply=page([record(1,{completed_summary:"大".repeat(400000)})]);
 await view.navigate("latest");assert.match(view.state.error,/协议或大小/);view.dispose();
});
test("switching edges cancels previous request and late responses cannot replace current selection",async()=>{
 const callbacks=[];
 const view=D.create({runId:"run",api:{get:(path,opts)=>new Promise(resolve=>callbacks.push({resolve,signal:opts.signal}))}});
 const first=view.select(source,target),second=view.select({...source,task_id:"new"},{...target,depends_on:["new"]});
 assert.equal(callbacks[0].signal.aborted,true);
 callbacks[1].resolve(page([record(2,{handoff_id:"run:new:v2"})]));await second;
 callbacks[0].resolve(page([record(1)]));await first;
 assert.equal(view.state.records[0].handoff_id,"run:new:v2");view.dispose();
});
test("disposal aborts metadata reads; no later results or notification timers survive",async()=>{
 let finish,signal;
 const view=D.create({runId:"run",api:{get:async(path,opts)=>{
  if(path.startsWith("/teams"))return page([record(1)]);
  signal=opts.signal;return new Promise(resolve=>finish=resolve);
 }}});
 await view.select(source,target);const metadata=view.inspectArtifact("artifact-1");view.notice({event_type:"handoff.saved"});
 view.dispose();assert.equal(signal.aborted,true);
 finish({artifact_id:"artifact-1",team_id:"run",task_id:"src"});await metadata;
 assert.equal(view.state.artifact,null);assert.equal(view.state.records.length,0);
});
test("invalid graph pairs do not read arbitrary records",async()=>{
 let calls=0;const view=D.create({runId:"run",api:{get:()=>{calls++;}}});
 await view.select(source,{...target,depends_on:[]});assert.equal(calls,0);view.dispose();
});

test("steer or dependency removal invalidates the inspector and cancels stale evidence reads",async()=>{
 const callbacks=[];const view=D.create({runId:"run",api:{get:(path,opts)=>new Promise(resolve=>callbacks.push({resolve,signal:opts.signal}))}});
 const pending=view.select(source,target);
 view.refreshTasks([source,{...target,depends_on:[]}]);
 assert.equal(callbacks[0].signal.aborted,true);assert.equal(view.state.source,null);
 callbacks[0].resolve(page([record(1)]));await pending;assert.equal(view.state.records.length,0);
 const reload=view.select(source,target);callbacks[1].resolve(page([record(1)]));await reload;
 view.refreshTasks([{...source,status:"Failed"},target]);assert.match(view.html(),/Failed/);
 view.refreshTasks([{...source,worker:"replacement"},target]);assert.equal(view.state.source,null);view.dispose();
});

test("new notices arriving during a latest read trigger a bounded follow-up instead of disappearing",async()=>{
 let finish,calls=0;
 const view=D.create({runId:"run",api:{get:()=>{
  calls++;return calls===1?new Promise(resolve=>finish=resolve):Promise.resolve(page([record(2)]));
 }}});
 const initial=view.select(source,target);view.notice({event_type:"handoff.saved"});
 finish(page([record(1)]));await initial;
 assert.equal(view.state.newRecords,true);
 await new Promise(resolve=>setTimeout(resolve,300));
 assert.equal(calls,2);assert.equal(view.state.records[0].handoff_id,"run:src:v2");view.dispose();
});
test("manual history navigation cancels a queued live refresh and keeps its historical boundary",async()=>{
 const paths=[];
 const view=D.create({runId:"run",api:{get:async path=>{
  paths.push(path);return page([record(paths.length===1?3:1)],true);
 }}});
 await view.select(source,target);view.notice({event_type:"handoff.saved"});await view.navigate("older");
 await new Promise(resolve=>setTimeout(resolve,300));
 assert.equal(paths.length,2);assert.equal(view.state.before,"run:src:v3");view.dispose();
});
