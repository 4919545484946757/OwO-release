import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {webcrypto} from "node:crypto";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
const require=createRequire(import.meta.url),U=require("../core/attachment-upload.js"),H=require("../core/history-input.js");
const cap={constraints:{structured_input_parts:true,scoped_turn_cancellation:true}};
const hash=async blob=>Buffer.from(await webcrypto.subtle.digest("SHA-256",await blob.arrayBuffer())).toString("hex");
const make=async()=>{const blob=new Blob(["original"],{type:"image/png"}),id="input-00000000-0000-4000-8000-000000000001",part={type:"image",asset:{artifact_id:id,sha256:await hash(blob)}};return {blob,part,row:{id,name:"same.png",mime:"image/png",size:blob.size,content_part:part},message:{role:"user",content:"model-only suffix",images:[{url:"cached"}],input:{prompt:"human text",content_parts:[part,{type:"text",text:"extra context"}]}}};};
const options=(f,extra={})=>({sessionId:"a",get:async path=>path==="/capabilities"?cap:[f.row],download:async()=>f.blob,digest:hash,...extra});
test("original text is independent of model suffix and exact asset refs/text blocks survive preparation",async()=>{const f=await make();assert.equal(H.displayText(f.message),"human text");const p=await H.prepare(f.message,options(f));assert.equal(p.attachments[0].name,"same.png");assert.deepEqual(U.turnInput(p.attachments),{attachments:[],content_parts:f.message.input.content_parts});p.attachments[0].content_part.asset.sha256="changed";assert.equal(f.part.asset.sha256,await hash(f.blob));});
test("missing assets, changed hashes and wrong actual bytes fail without silently dropping media",async()=>{const f=await make();for(const rows of [[],[{...f.row,content_part:{...f.part,asset:{...f.part.asset,sha256:"a".repeat(64)}}}]])await assert.rejects(H.prepare(f.message,options(f,{get:async path=>path==="/capabilities"?cap:rows})),/不可用或版本不符/);await assert.rejects(H.prepare(f.message,options(f,{download:async()=>new Blob(["tampered"])})),/校验失败/);});
test("old core and legacy mutable history are explicitly rejected",async()=>{const f=await make();await assert.rejects(H.prepare(f.message,options(f,{get:async()=>({})})),/不支持历史附件/);for(const message of [{content:"old",images:[{url:"old"}]},{content:".owo-attachments/a/note.txt"},{input:{prompt:"x",legacy_attachments:["old.txt"]}}])await assert.rejects(H.prepare(message,options(f)),/旧附件/);assert.deepEqual(await H.prepare({content:"plain"},options(f)),{record:{prompt:"plain",content_parts:[],legacy_attachments:[]},attachments:[]});});
test("switching during metadata or bytes loading invalidates the preflight",async()=>{const f=await make();for(const stage of ["get","download"]){let current=true;const extra={isCurrent:()=>current};if(stage==="get")extra.get=async()=>{current=false;return cap;};else extra.download=async()=>{current=false;return f.blob;};await assert.rejects(H.prepare(f.message,options(f,extra)),/会话已切换/);}});
test("invalid text, duplicate metadata, image budget and oversized assets fail before download",async()=>{const f=await make();let downloads=0;const extra={download:async()=>{downloads++;return f.blob;}};for(const input of [{prompt:"x",content_parts:[{type:"text",text:"中".repeat(11000)}]},{prompt:"x",content_parts:"not-array"},{prompt:"x",content_parts:Array(33).fill(f.part)}])assert.throws(()=>H.read({input}));await assert.rejects(H.prepare(f.message,options(f,{...extra,get:async path=>path==="/capabilities"?cap:[f.row,f.row]})),/重复引用/);await assert.rejects(H.prepare(f.message,options(f,{...extra,get:async path=>path==="/capabilities"?cap:[{...f.row,size:6*1024*1024}]})),/大小无效/);assert.equal(downloads,0);});
test("local text chip keys never leak into legacy attachment IDs",()=>{assert.deepEqual(U.turnInput([{id:"@text:0",content_part:{type:"text",text:"source text"}}]),{attachments:[],content_parts:[{type:"text",text:"source text"}]});assert.throws(()=>U.turnInput([{id:"@text:0",content_part:{type:"text",text:4}}]),/附加文本/);});
const domain=readFileSync(new URL("../app-domain.js",import.meta.url),"utf8");
const replaySource=domain.slice(domain.indexOf("async function prepareHistoryReplay("),domain.indexOf("\n/// 历史用户消息的操作入口"));
async function harness(f,extra={}){const events=[],prompt={value:"existing draft",dispatchEvent(){},focus(){}};const state={sessionId:"a",selectionVersion:1,attachments:[]};const context={state,Event:class{},ModelRouting:{buildCustomModelConnection:()=>null},getComposerModel:()=>"model",loadCustomModels:()=>[],customModelKeys:()=>({}),modelGateMissing:()=>false,currentTurn:()=>null,$:()=>prompt,showToast:text=>events.push(["toast",text]),renderAttachmentChips:()=>events.push(["chips"]),window:{OwoHistoryInput:H,OwoAttachmentUpload:U,OwoApi:{download:async()=>{events.push(["download"]);return extra.corrupt?new Blob(["tampered"]):f.blob;}}},api:async(path,request)=>{events.push([path,request]);if(path==="/input/preflight"){await extra.duringInputPreflight?.(context);if(extra.preflightError)throw new Error(extra.preflightError);return extra.preflightResult ?? {ok:true};}return path==="/capabilities"?{constraints:{...cap.constraints,...extra.constraints}}:path.endsWith("/attachments")?[f.row]:{};},selectSession:async()=>{state.selectionVersion++;state.attachments=[];if(extra.switchOnReload){state.sessionId="b";state.selectionVersion++;}},sendPrompt:async()=>events.push(["send",U.turnInput(state.attachments)]),...extra.context};runInNewContext(replaySource+"\nthis.replay=rewindAndResend;",context);return {context,events,state,prompt};}
test("actual rewind path verifies bytes before destructive rewind, restores refs after reload and sends exact parts",async()=>{const f=await make();const h=await harness(f);await h.context.replay("a",2,"edited",f.message);const paths=h.events.map(e=>e[0]);assert(paths.indexOf("download")<paths.indexOf("/session/a/rewind"));assert.equal(h.prompt.value,"edited");assert.deepEqual(h.events.find(e=>e[0]==="send")[1].content_parts,f.message.input.content_parts);});
test("actual rewind leaves conversation and draft intact when bytes are corrupt",async()=>{const f=await make();const h=await harness(f,{corrupt:true});await assert.rejects(h.context.replay("a",2,"edited",f.message),/校验失败/);assert.equal(h.prompt.value,"existing draft");assert(!h.events.some(e=>String(e[0]).endsWith("/rewind")||e[0]==="send"));});
test("actual rewind cannot populate another conversation after history reload",async()=>{const f=await make();const h=await harness(f,{switchOnReload:true});await h.context.replay("a",2,"edited",f.message);assert.equal(h.state.sessionId,"b");assert.deepEqual(h.state.attachments,[]);assert(!h.events.some(e=>e[0]==="send"));assert(h.events.some(e=>e[0]==="toast"&&e[1].includes("附件未加入其他会话")));});
test("unconfigured model refuses before rollback",async()=>{const f=await make();const h=await harness(f,{context:{modelGateMissing:()=>true}});await assert.rejects(h.context.replay("a",0,"edited",f.message),/模型尚未配置/);assert.equal(h.events.length,0);});


test("an older failure card retains its own input when a later turn replaces the session cache",async()=>{
 const f=await make(),h=await harness(f);
 h.state.lastSessionInputs=new Map([["a",f.message]]);
 h.context.document={createElement(){return {children:[],appendChild(child){this.children.push(child);},addEventListener(name,handler){this[name]=handler;}};}};
 h.context.askConfirm=async()=>true;h.context.friendlyError=e=>e.message;
 const recovery=domain.slice(domain.indexOf("function turnRecoveryActions("),domain.indexOf("async function selectSession("));
 runInNewContext(recovery+"\nthis.recovery=turnRecoveryActions;",h.context);
 const row=h.context.recovery("a","human text","old failed turn");
 h.state.lastSessionInputs.set("a",{input:{prompt:"later prompt",content_parts:[{type:"image",asset:{artifact_id:"input-00000000-0000-4000-8000-000000000002",sha256:"b".repeat(64)}}]}});
 h.prompt.value="";
 await row.children[2].click();
 const sent=h.events.find(e=>e[0]==="send");assert(sent,"the old card must remain retryable");
 assert.equal(h.prompt.value,"human text");assert.deepEqual(sent[1].content_parts,f.message.input.content_parts);
});

test("image admission refuses edit before rewind and preserves existing conversation and draft",async()=>{
 const f=await make(),h=await harness(f,{constraints:{model_input_preflight:true},preflightError:"input/image_capability_unknown"});
 await assert.rejects(h.context.replay("a",2,"edited",f.message),/image_capability_unknown/);
 assert.equal(h.prompt.value,"existing draft");assert(!h.events.some(e=>String(e[0]).endsWith("/rewind")||e[0]==="send"));
 const request=JSON.parse(h.events.find(e=>e[0]==="/input/preflight")[1].body);
 assert.deepEqual(request,{has_images:true,session_id:"a",history_keep:2});
});
test("admitted replay preflight includes retained history boundary before rewind",async()=>{
 const f=await make(),h=await harness(f,{constraints:{model_input_preflight:true}});
 await h.context.replay("a",1,"edited",f.message);
 const paths=h.events.map(e=>e[0]);assert(paths.indexOf("/input/preflight")<paths.indexOf("/session/a/rewind"));
 assert(h.events.some(e=>e[0]==="send"));
});
test("unknown preflight response and a model change during preflight leave history intact",async()=>{
 for(const extra of [{preflightResult:{}},{duringInputPreflight:async context=>{context.getComposerModel=()=>"other";}}]){
  const f=await make(),h=await harness(f,{constraints:{model_input_preflight:true},...extra});
  await assert.rejects(h.context.replay("a",1,"edited",f.message),/预检|变化/);
  assert.equal(h.prompt.value,"existing draft");assert(!h.events.some(e=>String(e[0]).endsWith("/rewind")||e[0]==="send"));
 }
});
