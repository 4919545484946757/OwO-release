
import { test } from "node:test";
import assert from "node:assert/strict";
import "../core/turn-sse.js";
const api = globalThis.OwoTurnSse;
const frame = (seq, type, payload = {}) => "id: " + seq + "\nevent: " + type + "\ndata: " + JSON.stringify({type, ...payload}) + "\n\n";
const response = (text, turnId = null) => new Response(text, {headers: turnId ? {"x-owo-turn-id":turnId} : {}});
const final = frame(1, "final", {text:"候选回答"});
const stats = frame(2, "turn_stats", {completion_status:"accepted", total_tokens:21});
test("final is provisional; persisted host stats close acceptance", async () => {
 const completion = api.createCompletion();
 completion.observe("final", {text:"候选"});
 assert.equal(completion.terminal, false);
 assert.throws(() => completion.finish(), /缺少宿主完成/);
 completion.observe("turn_stats", {completion_status:"accepted"});
 assert.equal(completion.finish().completionStatus, "accepted");
});
test("save failure after final overrides text completion", async () => {
 const events=[];
 await assert.rejects(api.consumeResponse(response(final+frame(2,"turn_failed",{message:"save failed",completion_status:"unverified"})),{onEvent:(t)=>events.push(t)}), e=>e.completionStatus==="unverified"&&/save failed/.test(e.message));
 assert.deepEqual(events,["final","turn_failed"]);
});
test("EOF after final resumes to host stats using durable cursor", async () => {
 const calls=[],events=[];
 const result = await api.consumeResponse(response(final,"turn-1"),{onEvent:t=>events.push(t), replay:async(id,cursor)=>{
 calls.push([id,cursor]);
 return {active:false,state:"completed",events:[{turn_id:id,seq:1,payload:{type:"final",text:"duplicate"}},{turn_id:id,seq:2,payload:{type:"turn_stats",completion_status:"accepted"}}]};
 }});
 assert.equal(result.completionStatus,"accepted");assert.deepEqual(calls,[["turn-1",1]]);assert.deepEqual(events,["final","turn_stats"]);
});
test("completed replay drains multiple pages and excludes foreign/duplicate records", async () => {
 let calls=0;const events=[];
 const result=await api.consumeResponse(response("","turn"),{onEvent:t=>events.push(t), replay:async()=> {
 calls++; return calls===1 ? {active:false,state:"completed",events:[{turn_id:"foreign",seq:9,payload:{type:"turn_failed"}},{turn_id:"turn",seq:1,payload:{type:"final",text:"answer"}}]} :
 {active:false,state:"completed",events:[{turn_id:"turn",seq:2,payload:{type:"turn_stats",completion_status:"response_complete"}}]};
 }});
 assert.equal(result.completionStatus,"response_complete");assert.equal(calls,2);assert.deepEqual(events,["final","turn_stats"]);
});
test("candidate host status is exposed without being promoted to accepted", async () => {
 const result=await api.consumeResponse(response(final+frame(2,"turn_stats",{completion_status:"candidate"})),{onEvent:()=>{}});
 assert.equal(result.completionStatus,"candidate");
});
test("missing/unknown completion receipts fail closed", async () => {
 await assert.rejects(api.consumeResponse(response(final),{onEvent:()=>{}}),/缺少宿主完成/);
 await assert.rejects(api.consumeResponse(response(final+frame(2,"turn_stats",{})),{onEvent:()=>{}}),/有效的宿主/);
});
test("callback exceptions propagate and never trigger replay", async () => {
 let replayed=false;
 await assert.rejects(api.consumeResponse(response(final,"turn"),{onEvent:()=>{throw new Error("consumer bug");},replay:async()=>{replayed=true;}}),/consumer bug/);
 assert.equal(replayed,false);
});
test("split UTF-8, CRLF and multiline JSON survive framing", async () => {
 const raw = new TextEncoder().encode('event: final\r\ndata: {"text":\r\ndata: "中文"}\r\n\r\nevent: turn_stats\r\ndata: {"completion_status":"accepted"}');
 const stream=new ReadableStream({start(c){ for(const byte of raw)c.enqueue(Uint8Array.of(byte));c.close(); }});
 const result=await api.consumeResponse(new Response(stream),{onEvent:()=>{}});
 assert.equal(result.finalText,"中文");assert.equal(result.completionStatus,"accepted");
});
test("oversized frames and malformed JSON cannot become successful results", async () => {
 await assert.rejects(api.consumeResponse(response("data: "+ "x".repeat(1024*1024+1)),{onEvent:()=>{}}),/1 MiB/);
 await assert.rejects(api.consumeResponse(response("data: {oops}\n\n"),{onEvent:()=>{}}),/格式无效/);
});
test("cancel while waiting for replay exits without accepting provisional text", async () => {
 const controller=new AbortController();
 await assert.rejects(api.consumeResponse(response(final,"turn"),{signal:controller.signal,onEvent:()=>{},replay:async()=>({active:true,events:[]}),wait:async()=>{controller.abort();}}),e=>e.name==="AbortError");
});
test("consumer failures cancel the unfinished browser SSE body", async () => {
 let cancelled=false;
 const body=new ReadableStream({
  start(controller){controller.enqueue(new TextEncoder().encode(final));},
  cancel(){cancelled=true;},
 });
 await assert.rejects(api.consumeResponse(response(body),{onEvent:()=>{throw new Error("consumer bug");}}),/consumer bug/);
 assert.equal(cancelled,true);
});

test("persisted terminal stats complete an open SSE without waiting for EOF", async () => {
 let cancelled=false;
 const body=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode(final+stats));},cancel(){cancelled=true;}});
 const result=await api.consumeResponse(response(body,"turn"),{onEvent:()=>{},idleTimeoutMs:5});
 assert.equal(result.completionStatus,"accepted");assert.equal(cancelled,true);
});
test("stalled open stream recovers durable stats and deduplicates final", async () => {
 let reads=0,cancelled=false;const events=[];
 const body=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode(final));},cancel(){cancelled=true;}});
 const result=await api.consumeResponse(response(body,"turn"),{onEvent:t=>events.push(t),idleTimeoutMs:5,replay:async(id,cursor)=>{
  reads++;assert.equal(cursor,1);
  return {active:false,state:"completed",events:[{turn_id:id,seq:1,payload:{type:"final",text:"duplicate"}},{turn_id:id,seq:2,payload:{type:"turn_stats",completion_status:"accepted"}}]};
 }});
 assert.equal(result.completionStatus,"accepted");assert.equal(reads,1);assert.deepEqual(events,["final","turn_stats"]);assert.equal(cancelled,true);
});
test("active replay retains the original stream until it produces completion", async () => {
 let replayed=false,cancelled=false,controller;
 const body=new ReadableStream({start(c){controller=c;c.enqueue(new TextEncoder().encode(final));},cancel(){cancelled=true;}});
 const result=await api.consumeResponse(response(body,"turn"),{onEvent:()=>{},idleTimeoutMs:5,replay:async()=>{
  replayed=true;assert.equal(cancelled,false);controller.enqueue(new TextEncoder().encode(stats));return {active:true,events:[]};
 }});
 assert.equal(replayed,true);assert.equal(result.completionStatus,"accepted");assert.equal(cancelled,true);
});
test("abort interrupts a stalled stream without accepting provisional output", async () => {
 const controller=new AbortController();let cancelled=false;
 const body=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode(final));},cancel(){cancelled=true;}});
 const timer=setTimeout(()=>controller.abort(),5);
 try { await assert.rejects(api.consumeResponse(response(body,"turn"),{signal:controller.signal,onEvent:()=>{},idleTimeoutMs:100}),e=>e.name==="AbortError"); }
 finally { clearTimeout(timer); }
 assert.equal(cancelled,true);
});

test("replay timeout cancels its request and cannot accept provisional output", async () => {
 let replaySignal;
 await assert.rejects(api.consumeResponse(response(final,"turn"),{onEvent:()=>{},replayTimeoutMs:5,replay:async(id,cursor,signal)=>{
  replaySignal=signal;return new Promise(()=>{});
 }}),/turn\/replay_timeout/);
 assert.equal(replaySignal.aborted,true);
});

// 思考增量是纯增量通道：旧版服务端把连续 reasoning_delta 合并进同一行、复用该行 seq，
// SSE id 因此重复。若按 id 去重，第一条之后的思考全部被丢弃——线上表现就是
// 「思考过程只剩开头几个字符」。增量事件必须放行，非增量事件继续按游标去重。
test("repeated SSE ids on reasoning deltas never drop increments", async () => {
 const seen=[];
 const body = frame(15,"reasoning_delta",{delta:"The"}) + frame(15,"reasoning_delta",{delta:" user"})
   + frame(15,"reasoning_delta",{delta:" asks who"}) + frame(16,"final",{text:"ok"})
   + frame(17,"turn_stats",{completion_status:"accepted"});
 const result = await api.consumeResponse(response(body),{onEvent:(type,payload)=>{ if(type==="reasoning_delta") seen.push(payload.delta); }});
 assert.equal(seen.join(""),"The user asks who");
 assert.equal(result.completionStatus,"accepted");
});

test("non-incremental frames still collapse on a repeated cursor id", async () => {
 const seen=[];
 const body = frame(5,"token_delta",{delta:"a"}) + frame(5,"token_delta",{delta:"b"})
   + frame(6,"final",{text:"ab"}) + frame(7,"turn_stats",{completion_status:"accepted"});
 await api.consumeResponse(response(body),{onEvent:(type,payload)=>{ if(type==="token_delta") seen.push(payload.delta); }});
 assert.deepEqual(seen,["a"],"重复游标的非增量事件仍是重复投递，必须丢弃");
});

test("replayed reasoning is flagged as a snapshot for replace-rendering", async () => {
 const seen=[];
 const result = await api.consumeResponse(response(frame(1,"final",{text:"答案"}),"turn-1"),{
  onEvent:(type,payload,meta)=>{ if(type==="reasoning_delta") seen.push([payload.delta,meta.replay,meta.id]); },
  replay:async(id)=>({active:false,state:"completed",events:[
   {turn_id:id,seq:2,payload:{type:"reasoning_delta",delta:"完整思考"}},
   {turn_id:id,seq:3,payload:{type:"turn_stats",completion_status:"accepted"}},
  ]}),
 });
 assert.deepEqual(seen,[["完整思考",true,2]]);
 assert.equal(result.completionStatus,"accepted");
});
