import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
import routing from "../core/model-routing.js";
import uploads from "../core/attachment-upload.js";
const source = readFileSync(new URL("../app-domain.js",import.meta.url),"utf8");
const start=source.indexOf("async function sendPrompt()");
const end=source.indexOf("\nfunction renderAttachmentChips()",start);
assert.ok(start>=0 && end>start);
async function submit(support, attachments=[],structured=true, options={}) {
 const prompt={value:options.prompt ?? "hi"};const requests=[];const notices=[];const preflights=[];
 const model={id:"custom-x",baseUrl:"http://127.0.0.1:7777/native",apiFormat:"anthropic",useFullUrl:true,temperature:0,timeoutSecs:4};
 const context={
  URL,AbortController,Map,Set,Date,crypto:{randomUUID:()=>"11111111-1111-4111-8111-111111111111"},ModelRouting:routing,
  state:{sessionId:options.newSession ? null : "s",selectionVersion:1,attachments,activeTurns:new Map()},
  promptSessionStart:{run:create=>create()},newSession:async()=>options.newSession(context,prompt),
  $:id=>id==="prompt"?prompt:{value:"D:/workspace"},
  getComposerModel:()=>"custom-x",loadCustomModels:()=>[model],customModelKeys:()=>({"custom-x":"fixture-key"}),
  api:async(path,request)=>{if(path==="/input/preflight"){preflights.push(JSON.parse(request.body));await options.duringInputPreflight?.(context,prompt);if(options.inputError)throw new Error(options.inputError);return options.inputResult ?? {ok:true};}await options.duringPreflight?.(context,prompt);return {constraints:{custom_model_connection:support,scoped_turn_cancellation:true,structured_input_parts:structured,model_input_preflight:options.inputPreflight===true}};},
  modelGateMissing:()=>options.unconfigured===true, openModelGateSettings:()=>notices.push("settings opened"), currentTurn:()=>null, friendlyError:e=>e.message,
  showToast:(text)=>notices.push(text), addMessage:()=>({}),writeTargetSid:null,
  window:{OwoAttachmentUpload:uploads,OwoApi:{stream:async(path,request)=>{
   requests.push({path,json:request.json});
   if(options.duringExecution){await options.duringExecution(context,prompt);return {};}
   throw Object.assign(new Error("controlled stop after request capture"),{name:"AbortError"});
  }},OwoTurnSse:{consumeResponse:async()=>({completionStatus:"response_complete"})}},
 };
 for(const name of ["updateComposerHint","updateScrollBottomBtn","updateComposerRunning","startRunStatus",
 "renderAttachmentChips","refreshSessions","refreshDiff","refreshSessionContext","showTurnFailure","addEventChip","hideApproval","finishThinking","settlePendingQuestion","stopRunStatus","resetRunBlocks"]) context[name]=()=>{};
 runInNewContext(source.slice(source.indexOf("async function preflightModelInput("),source.indexOf("async function rewindAndResend("))+source.slice(start,end)+"\nthis.submit=sendPrompt;",context);
 await context.submit();
 return {preflights,requests,notices,prompt,state:context.state};
}
test("actual composer sends all custom connection fields to the turn route",async()=>{
 const {requests}=await submit(true);
 assert.equal(requests.length,1);
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0])),{path:"/session/s/turn",json:{
  prompt:"hi",attachments:[],content_parts:[],turn_id:"11111111-1111-4111-8111-111111111111",model_connection:{model:"custom-x",base_url:"http://127.0.0.1:7777/native",
  api_format:"anthropic",use_full_url:true,api_key:"fixture-key",temperature:0,timeout_secs:4}
 }});
});
test("actual composer keeps draft and submits nothing against an old core",async()=>{
 const {requests,notices,prompt}=await submit(false);
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.equal(notices.length,1);
});

test("actual composer sends exact uploaded asset references instead of mutable filenames",async()=>{
 const id="input-12345678-1234-4321-9876-123456789abc",part={type:"image",asset:{artifact_id:id,sha256:"a".repeat(64)}};
 const {requests}=await submit(true,[{id,name:"same.png",content_part:part}]);assert.equal(requests.length,1);
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0].json.content_parts)),[part]);assert.deepEqual(JSON.parse(JSON.stringify(requests[0].json.attachments)),[]);
});
test("actual composer preserves draft and submits nothing for unsupported audio input",async()=>{
 const id="input-12345678-1234-4321-9876-123456789abc";
 const {requests,notices,prompt}=await submit(true,[{id,content_part:{type:"audio",asset:{artifact_id:id,sha256:"a".repeat(64)}}}]);
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.match(notices[0],/输入适配器/);
});

test("actual composer refuses immutable parts against a core without transport support",async()=>{
 const id="input-12345678-1234-4321-9876-123456789abc";
 const {requests,notices,prompt}=await submit(true,[{id,content_part:{type:"image",asset:{artifact_id:id,sha256:"a".repeat(64)}}}],false);
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.match(notices[0],/结构化附件输入/);
});


test("switching away and back during capability loading cannot send an earlier draft",async()=>{
 const {requests,notices,prompt}=await submit(true,[],true,{duringPreflight:async context=>{context.state.selectionVersion=(context.state.selectionVersion||0)+2;}});
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.match(notices[0],/输入|会话/);
});
test("capability loading cannot mix the original prompt with another session's attachments",async()=>{
 const {requests,prompt,state}=await submit(true,[],true,{duringPreflight:async context=>{context.state.sessionId="other";context.state.attachments=[{id:"other.txt"}];}});
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.equal(state.sessionId,"other");assert.equal(state.attachments[0].id,"other.txt");
});
test("editing the draft or uploading an attachment during capability loading requires a fresh submit",async()=>{
 for(const change of [(_context,prompt)=>{prompt.value="new draft";},context=>{context.state.attachments.push({id:"new.txt"});}]){
  const {requests,prompt}=await submit(true,[],true,{duringPreflight:async(context,prompt)=>change(context,prompt)});
  assert.equal(requests.length,0);assert(prompt.value);
 }
});
test("attachment-only input sends the exact parts without inventing prompt text",async()=>{
 const id="input-12345678-1234-4321-9876-123456789abc",part={type:"image",asset:{artifact_id:id,sha256:"a".repeat(64)}};
 const {requests}=await submit(true,[{id,name:"image.png",content_part:part}],true,{prompt:""});
 assert.equal(requests.length,1);assert.equal(requests[0].json.prompt,"");assert.deepEqual(JSON.parse(JSON.stringify(requests[0].json.content_parts)),[part]);
});

test("changing the selected model during capability loading preserves the draft",async()=>{
 const {requests,prompt}=await submit(true,[],true,{duringPreflight:async context=>{context.getComposerModel=()=>"another-model";}});
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");
});
test("background completion cannot discard a foreground attachment draft",async()=>{
 const {requests,state}=await submit(true,[{id:"original.txt"}],true,{duringExecution:async context=>{context.state.sessionId="other";context.state.selectionVersion++;context.state.attachments=[{id:"other.txt"}];}});
 assert.equal(requests.length,1);assert.equal(state.attachments[0].id,"other.txt");
});
test("attachments added while a turn runs survive completion",async()=>{
 const {state}=await submit(true,[{id:"original.txt"}],true,{duringExecution:async context=>{context.state.attachments.push({id:"next.txt"});}});
 assert.deepEqual(state.attachments.map(row=>row.id),["original.txt","next.txt"]);
});
test("first submit follows only the session it created, with the same input draft",async()=>{
 for(const changed of [false,true]){
  const {requests,prompt}=await submit(true,[],true,{newSession:async context=>{context.state.sessionId=changed?"other":"created";context.state.selectionVersion=2;return {id:"created",selectionVersion:2};}});
  assert.equal(requests.length,changed?0:1);if(changed)assert.equal(prompt.value,"hi");else assert.equal(requests[0].path,"/session/created/turn");
 }
});

test("actual session creation does not steal selection or clear pending settings after a late response",async()=>{
 const creation=source.slice(source.indexOf("async function newSession("),source.indexOf("// 进入时自动恢复上次的对话"));
 for(const switched of [false,true]){
  const state={sessionId:"old",selectionVersion:1,pendingModelOverride:"choice",pendingSystemPrompt:"keep me",sessionMode:"agent"},selected=[];
  const context={state,ModelRouting:routing,localStorage:{setItem(){}},$:()=>({value:"D:/workspace"}),currentSessionMode:()=>"agent",showToast:()=>{},syncSessionWelcome:()=>{},refreshSessions:async()=>{},
   api:async()=>{if(switched){state.sessionId="other";state.selectionVersion++;}return {id:"created"};},selectSession:async id=>{selected.push(id);state.sessionId=id;state.selectionVersion++;}};
  runInNewContext(creation+"\nthis.create=newSession;",context);const result=await context.create();
  assert.deepEqual(selected,switched?[]:["created"]);
  if(switched){assert.equal(result,null);assert.equal(state.pendingModelOverride,"choice");assert.equal(state.pendingSystemPrompt,"keep me");assert.equal(state.sessionId,"other");}
  else{assert.equal(result.id,"created");assert.equal(result.selectionVersion,state.selectionVersion);assert.equal(state.pendingSystemPrompt,"");}
 }
});

test("image admission rejection keeps the composer draft and never submits a turn or creates a session",async()=>{
 let created=0;
 const {requests,notices,prompt,preflights,state}=await submit(true,[],true,{
  inputPreflight:true,inputError:"input/image_capability_unknown",
  newSession:async()=>{created++;throw new Error("must not create");}
 });
 assert.equal(created,0);assert.equal(state.sessionId,null);assert.equal(requests.length,0);
 assert.equal(prompt.value,"hi");assert.match(notices[0],/image_capability_unknown/);
 assert.equal(preflights.length,1);assert.equal(preflights[0].has_images,false);
 assert.equal(preflights[0].model,"custom-x");assert.equal(preflights[0].model_connection.api_format,"anthropic");
});
test("actual send preflight includes images and uses authoritative session context",async()=>{
 const part={type:"image",asset:{artifact_id:"input-12345678-1234-4321-9876-123456789abc",sha256:"a".repeat(64)}};
 const {requests,preflights}=await submit(true,[{id:part.asset.artifact_id,name:"same.png",content_part:part}],true,{inputPreflight:true});
 assert.equal(requests.length,1);assert.equal(preflights[0].has_images,true);assert.equal(preflights[0].session_id,"s");
 assert(!Object.hasOwn(preflights[0],"model"));assert(!Object.hasOwn(preflights[0],"history_keep"));
});
test("draft edits or session switches during native admission preserve new input and never send",async()=>{
 for(const change of [(context,prompt)=>{prompt.value="new draft";},context=>{context.state.sessionId="other";context.state.selectionVersion++;}]){
  const {requests,prompt,notices}=await submit(true,[],true,{inputPreflight:true,duringInputPreflight:change});
  assert.equal(requests.length,0);assert(prompt.value.length);assert.match(notices[0],/变化/);
 }
});
test("preflight needs an explicit success receipt and does not infer an image from a file name",async()=>{
 for(const result of [{},null]){
  const {requests,prompt}=await submit(true,[],true,{inputPreflight:true,inputResult:result ?? {}});
  assert.equal(requests.length,0);assert.equal(prompt.value,"hi");
 }
 const part={type:"file",asset:{artifact_id:"input-12345678-1234-4321-9876-123456789abc",sha256:"a".repeat(64)}};
 const {requests,preflights}=await submit(true,[{id:part.asset.artifact_id,name:"documentation.png",content_part:part}],true,{inputPreflight:true});
 assert.equal(requests.length,1);assert.equal(preflights[0].has_images,false);
});

test("changing a custom connection while the first session is created cannot send an old admitted connection",async()=>{
 const {requests,prompt,notices}=await submit(true,[],true,{inputPreflight:true,
  newSession:async context=>{
   context.state.sessionId="created";context.state.selectionVersion++;
   context.loadCustomModels=()=>[{id:"custom-x",baseUrl:"http://127.0.0.1:8888/changed",apiFormat:"openai"}];
   return {id:"created",selectionVersion:context.state.selectionVersion};
  }});
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.match(notices[0],/输入已变化/);
});

test("unconfigured model still opens setup before native admission or automatic session creation",async()=>{
 let created=0;
 const {requests,notices,preflights,prompt}=await submit(true,[],true,{inputPreflight:true,unconfigured:true,
  newSession:async()=>{created++;throw new Error("must not create");}});
 assert.equal(created,0);assert.equal(requests.length,0);assert.equal(preflights.length,0);
 assert.equal(prompt.value,"hi");assert.deepEqual(notices,["settings opened"]);
});
