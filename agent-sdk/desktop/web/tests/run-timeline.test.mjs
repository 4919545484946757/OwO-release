import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { create } = require("../core/run-timeline.js");
const tick = async () => { for (let i=0;i<12;i++) await new Promise(resolve=>setImmediate(resolve)); };
const event = (seq, extra={}) => ({schema_version:1,run_id:"a",seq,event_id:"run:a:"+seq,event_type:"message.final",data:{},...extra});
const frame = row => ({event:"run_event",id:row.event_id,data:JSON.stringify(row)});
const page = (rows=[],more=false) => ({run_id:"a",events:rows,has_more:more,next_after:rows.at(-1)?.seq||0,history_complete:true});
function setup(api, extra={}) {
  const timers=[],states=[],seen=[];
  const client=create({runId:"a",api,onEvent:e=>seen.push(e.seq),onState:s=>states.push(s),
    setTimeout:(fn,delay)=>{const t={fn,delay};timers.push(t);return t;},
    clearTimeout:t=>{t.cancelled=true;},...extra});
  return {client,timers,states,seen,async next(){const t=timers.find(t=>!t.cancelled&&!t.used);assert.ok(t,"expected scheduled retry");t.used=true;t.fn();await tick();}};
}

test("SSE resume uses canonical identities, deduplicates repeats and allows global sequence gaps",async()=>{
  const streams=[],gets=[];
  const h=setup({
    openEventStream:async(path,options)=>{streams.push({path,options});if(streams.length===1){options.onEvent(frame(event(3)));options.onEvent(frame(event(3)));throw new Error("disconnect");}options.onEvent(frame(event(9)));return new Promise(()=>{});},
    get:async path=>{gets.push(path);return path.includes("/timeline")?page([event(7)]):{team:{status:"running"}};}
  });
  h.client.start();await tick();assert.deepEqual(h.seen,[3]);await h.next();
  assert.deepEqual(h.seen,[3,7]);assert.match(gets[0],/after=3$/);await h.next();
  assert.equal(streams[1].options.lastEventId,"run:a:7");assert.match(streams[1].path,/after=7$/);
  assert.deepEqual(h.seen,[3,7,9]);h.client.stop();assert.equal(streams[1].options.signal.aborted,true);
});

test("EOF drains events committed between empty page and terminal snapshot",async()=>{
  let reads=0;const h=setup({
    openEventStream:async(_,o)=>{o.onOpen();o.onEvent(frame(event(2)));},
    get:async path=>path.includes("/timeline")?(++reads===1?page():page([event(5,{event_type:"team.status_changed",data:{status:"succeeded"}})])):{team:{status:"succeeded"}}
  });
  h.client.start();await tick();assert.deepEqual(h.seen,[2,5]);assert.equal(h.client.cursor(),5);
  assert.equal(h.states.at(-1).mode,"done");assert.equal(h.timers.length,0);
});

test("terminal drain continues across bounded pages before reporting done",async()=>{
  let reads=0;const h=setup({
    openEventStream:async()=>{},
    get:async path=>path.includes("/timeline")?(++reads===1?page():reads===2?page([event(1)],true):reads===3?page([event(2)]):page()):{team:{status:"failed"}}
  });h.client.start();await tick();assert.deepEqual(h.seen,[1,2]);assert.equal(h.states.at(-1).mode,"done");
});

for (const change of [{run_id:"other"},{event_id:"run:a:02"},{seq:Number.MAX_SAFE_INTEGER+1},{schema_version:2}]) {
 test("invalid event is explicit and never acknowledges cursor: "+JSON.stringify(change),async()=>{
  let signal;const h=setup({openEventStream:async(_,o)=>{signal=o.signal;o.onEvent(frame(event(1)));o.onEvent(frame(event(2,change)));}});
  h.client.start();await tick();assert.equal(h.client.cursor(),1);assert.deepEqual(h.seen,[1]);
  assert.equal(h.states.at(-1).mode,"error");assert.equal(signal.aborted,true);assert.equal(h.timers.length,0);
 });
}

test("malformed JSON and frame/data identity disagreement fail visibly",async()=>{
  for(const broken of [{event:"run_event",id:"run:a:2",data:"{"},{...frame(event(2)),id:"run:a:3"}]) {
   const h=setup({openEventStream:async(_,o)=>o.onEvent(broken)});
   h.client.start();await tick();assert.equal(h.states.at(-1).mode,"error");assert.equal(h.client.cursor(),0);
  }
});

test("only unsupported endpoints use compatibility callback; authentication and cursor errors do not",async()=>{
  for(const status of [404,501,400,401,403]) {
   let fallback=0;const h=setup({openEventStream:async()=>{throw Object.assign(new Error("HTTP "+status),{status});}},{onUnsupported:()=>fallback++});
   h.client.start();await tick();assert.equal(fallback,[404,501].includes(status)?1:0);assert.equal(h.timers.length,0);
   assert.equal(h.states.at(-1).mode,fallback?"unsupported":"error");
  }
});

test("leaving aborts pending JSON catchup and ignores its late events",async()=>{
  let resolve,signal;const h=setup({
    openEventStream:async()=>{},
    get:async(_,o)=>{signal=o.signal;return new Promise(r=>resolve=r);}
  });
  h.client.start();await tick();h.client.stop();assert.equal(signal.aborted,true);resolve(page([event(4)]));await tick();
  assert.deepEqual(h.seen,[]);assert.equal(h.timers.length,0);
});

test("durable pending pages yield after eight reads and never fabricate a cursor",async()=>{
  let reads=0;const h=setup({openEventStream:async()=>{},get:async()=>{reads++;return page([],true);}});
  h.client.start();await tick();assert.equal(reads,8);assert.equal(h.client.cursor(),0);
  assert.equal(h.timers[0].delay,0);h.client.stop();assert.equal(h.timers[0].cancelled,true);
});

test("stop cancels backoff; restarting the same subscription retains the acknowledged cursor",async()=>{
  const streams=[];const h=setup({openEventStream:async(_,o)=>{streams.push(o);o.onEvent(frame(event(4)));throw new Error("offline");}});
  h.client.start();await tick();h.client.stop();assert.equal(h.timers[0].cancelled,true);h.client.start();await tick();
  assert.equal(streams[1].lastEventId,"run:a:4");assert.deepEqual(h.seen,[4]);h.client.stop();
});

test("observer failure does not acknowledge an unaccepted event",async()=>{
  const h=setup({openEventStream:async(_,o)=>o.onEvent(frame(event(6)))},{onEvent:()=>{throw new Error("render unavailable");}});
  h.client.start();await tick();assert.equal(h.client.cursor(),0);assert.equal(h.states.at(-1).mode,"poll");h.client.stop();
});

test("invalid or regressing JSON pages never silently reset the last accepted cursor",async()=>{
  const h=setup({
    openEventStream:async(_,o)=>{o.onEvent(frame(event(5)));},
    get:async()=>page([event(4)])
  });h.client.start();await tick();assert.equal(h.client.cursor(),5);assert.equal(h.states.at(-1).mode,"error");
});
