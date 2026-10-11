
import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
const read=p=>readFileSync(new URL("../"+p,import.meta.url),"utf8");
function runtime(files,extra={}){const root={...extra};runInNewContext(files.map(read).join("\n"),root);return root;}
test("durable facts restore host model, measured zero usage and receipt identities",()=>{
 const r=runtime(["core/completion-record.js","core/session-facts.js"]);
 const stats={turn_id:"a",payload:{type:"turn_stats",model_calls:[{model:"actual",usage:{total_tokens:0}}],completion_record:{status:"accepted",evidence_receipt_ids:["receipt"]}}};
 const f=r.OwoSessionFacts.fromDetail({turn_history:{last_turn_id:"a",last_terminal:stats,last_stats:stats}});
 assert.equal(f.lastExecutionModel,"actual");assert.equal(f.outcome,"completed");assert.match(r.OwoCompletionRecord.usageFromTurnStats(f.stats).text,/0 Tokens/);
 assert.equal(f.lastCompletionRecord.evidence_receipt_ids[0],"receipt");
});
test("an interrupted newer turn does not inherit an old successful receipt",()=>{
 const r=runtime(["core/completion-record.js","core/session-facts.js"]);
 const f=r.OwoSessionFacts.fromDetail({turn_history:{last_turn_id:"new",last_terminal:{turn_id:"old",payload:{type:"turn_stats",completion_record:{status:"accepted"}}}}});
 assert.equal(f.outcome,"interrupted");assert.equal(f.lastCompletionRecord,null);
});
test("new failure cannot reuse earlier model metrics and cancellation is authoritative",()=>{
 const r=runtime(["core/completion-record.js","core/session-facts.js"]);
 const h={last_turn_id:"new",last_terminal:{turn_id:"new",payload:{type:"turn_failed",completion_status:"aborted"}},last_stats:{turn_id:"old",payload:{model_calls:[{model:"old"}]}}};
 const f=r.OwoSessionFacts.fromDetail({turn_history:h});assert.equal(f.outcome,"cancelled");assert.equal(f.lastExecutionModel,"");
 assert.equal(r.OwoSessionFacts.fromDetail({turn_history:h,active_turn_id:"next"}).outcome,"running");
});
test("legacy keys are removed only after a successful migration response",async()=>{
 const values=new Map([["owo.custom-models",JSON.stringify([{id:"legacy",baseUrl:"http://localhost/v1"}])],["owo.model-keys",JSON.stringify({legacy:"test-only"})]]);
 const localStorage={getItem:k=>values.get(k),setItem:(k,v)=>values.set(k,v),removeItem:k=>values.delete(k)};
 const r=runtime(["core/custom-model-store.js"],{localStorage,owo:{customModels:{list:async()=>({models:[],encryptionAvailable:true}),migrate:async()=>{throw Error("migration failed");}}}});
 await r.OwoCustomModels.init();assert.equal(values.has("owo.model-keys"),true);assert.equal(r.OwoCustomModels.models()[0].migrationPending,true);
 r.owo.customModels.migrate=async()=>({models:[{id:"legacy",hasKey:true}],migratedIds:["legacy"]});
 await r.OwoCustomModels.init();assert.equal(values.has("owo.model-keys"),false);assert.equal(values.has("owo.custom-models"),false);
});
test("background replay sends no requests; resume drains cursors and stops at host end",async()=>{
 const r=runtime(["core/turn-sse.js","core/turn-recovery.js"],{setTimeout,clearTimeout,AbortController,URLSearchParams});
 let visible=false,calls=0,ended=false;const seq=[];
 const observer=r.OwoTurnRecovery.observe({sessionId:"s",turnId:"t",interval:5,visible:()=>visible,getPage:async path=>{calls++;assert.ok(path.includes("after_seq="+(calls===1?0:1)));return {active:calls===1,events:[{turn_id:"t",seq:calls,payload:{type:"token_delta"}}]};},onPage:records=>seq.push(...records.map(e=>e.seq)),onEnd:()=>ended=true,onError:e=>{throw e;}});
 await new Promise(resolve=>setTimeout(resolve,15));assert.equal(calls,0);visible=true;
 await new Promise(resolve=>setTimeout(resolve,30));observer.stop();assert.equal(ended,true);assert.deepEqual(seq,[1,2]);assert.equal(calls,2);
});
test("obsolete artifact reads cannot replace the newly selected delivery",async()=>{
 const r=runtime(["panels/evidence.panel.js"]);const p=r.OwoPanels.evidence,s=p._test.state;
 const listeners={},el={innerHTML:"",addEventListener:(e,h)=>listeners[e]=h};
 let resolveA;const delayed=new Promise(resolve=>resolveA=resolve);
 const H={esc:String,snapshot:()=>({sessions:[]}),friendlyError:e=>e.message,get:async path=>path==="/teams"?{teams:[]}:path.includes("/a/")?delayed:path.includes("/metadata")?{artifact_id:"b",version:1}:{}};
 await p.mount(el,H);const a=p._test.openArtifact("a");await p._test.openArtifact("b");resolveA({artifact_id:"a"});await a;
 assert.equal(s.detail.metadata.artifact_id,"b");p.dispose();
});
test("evidence distinguishes schema validity from absent behavioral receipts",()=>{
 const r=runtime(["panels/evidence.panel.js"]);
 assert.equal(r.OwoPanels.evidence._test.validationText({valid:true}),"格式校验通过");
 assert.match(r.OwoPanels.evidence._test.validationText(null),/未返回/);
});

test("project home excludes archived sessions and compares canonical Windows paths",()=>{
 const r=runtime(["panels/home.panel.js"]),filter=r.OwoPanels.home._test.projectSessions;
 const value=filter([{id:"yes",workspace:"D:\\Project\\",archived:false},{id:"other",workspace:"E:/Other"},{id:"archived",workspace:"d:/project",archived:true}],"d:/project");
 assert.deepEqual(Array.from(value,m=>m.id),["yes"]);
});
test("secure model routing sends a reference and never a renderer key",()=>{
 const context=runtime(["core/model-routing.js"],{URL});
 const request=context.OwoModelRouting.buildCustomModelConnection("fixture",[{id:"fixture",baseUrl:"http://127.0.0.1/v1",apiKeyEnv:"OWO_CUSTOM_MODEL_SECRET_"+"a".repeat(32)}],{fixture:"legacy-synthetic"});
 assert.equal(request.api_key,undefined);assert.equal(request.api_key_env,"OWO_CUSTOM_MODEL_SECRET_"+"a".repeat(32));
});
test("stop waits for host confirmation and only submits the displayed turn identity",async()=>{
 const source=read("app.js"),start=source.indexOf("async function abortTurn()"),end=source.indexOf("\n// ---------- 运行状态",start),entry={turnId:"wanted"};
 let resolve,removed=false;const calls=[];
 const pending=new Promise(r=>resolve=r),state={sessionId:"s",activeTurns:new Map([["s",entry]])};
 const c={state,currentTurn:()=>state.activeTurns.get("s"),api:async(path,options)=>{calls.push([path,JSON.parse(options.body)]);return pending;},encodeURIComponent,updateComposerRunning(){},setRunPhase(){},showToast(){},friendlyError:e=>e.message,window:{OwoStatusBar:{repaint(){}}}};
 runInNewContext(source.slice(start,end)+"\nthis.stop=abortTurn;",c);
 const wait=c.stop();assert.equal(entry.stopping,true);assert.equal(state.activeTurns.has("s"),true);
 assert.deepEqual(JSON.parse(JSON.stringify(calls)),[["/session/s/abort",{turn_id:"wanted"}]]);
 resolve({state:"abort_requested"});await wait;assert.equal(state.activeTurns.has("s"),true);
});
