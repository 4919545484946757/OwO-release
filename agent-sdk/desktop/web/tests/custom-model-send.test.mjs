import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
import routing from "../core/model-routing.js";
const source = readFileSync(new URL("../app-domain.js",import.meta.url),"utf8");
const start=source.indexOf("async function sendPrompt()");
const end=source.indexOf("\nfunction renderAttachmentChips()",start);
assert.ok(start>=0 && end>start);
async function submit(support) {
 const prompt={value:"hi"};const requests=[];const notices=[];
 const model={id:"custom-x",baseUrl:"http://127.0.0.1:7777/native",apiFormat:"anthropic",useFullUrl:true,temperature:0,timeoutSecs:4};
 const context={
  URL,AbortController,Map,Set,Date,crypto:{randomUUID:()=>"11111111-1111-4111-8111-111111111111"},ModelRouting:routing,
  state:{sessionId:"s",attachments:[],activeTurns:new Map()},
  $:id=>id==="prompt"?prompt:{value:"D:/workspace"},
  getComposerModel:()=>"custom-x",loadCustomModels:()=>[model],customModelKeys:()=>({"custom-x":"fixture-key"}),
  api:async()=>({constraints:{custom_model_connection:support,scoped_turn_cancellation:true}}),
  modelGateMissing:()=>false, currentTurn:()=>null, friendlyError:e=>e.message,
  showToast:(text)=>notices.push(text), addMessage:()=>({}),writeTargetSid:null,
  window:{OwoApi:{stream:async(path,request)=>{
   requests.push({path,json:request.json});
   throw Object.assign(new Error("controlled stop after request capture"),{name:"AbortError"});
  }}},
 };
 for(const name of ["updateComposerHint","updateScrollBottomBtn","updateComposerRunning","startRunStatus",
 "showTurnFailure","addEventChip","hideApproval","finishThinking","settlePendingQuestion","stopRunStatus","resetRunBlocks"]) context[name]=()=>{};
 runInNewContext(source.slice(start,end)+"\nthis.submit=sendPrompt;",context);
 await context.submit();
 return {requests,notices,prompt};
}
test("actual composer sends all custom connection fields to the turn route",async()=>{
 const {requests}=await submit(true);
 assert.equal(requests.length,1);
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0])),{path:"/session/s/turn",json:{
  prompt:"hi",attachments:[],turn_id:"11111111-1111-4111-8111-111111111111",model_connection:{model:"custom-x",base_url:"http://127.0.0.1:7777/native",
  api_format:"anthropic",use_full_url:true,api_key:"fixture-key",temperature:0,timeout_secs:4}
 }});
});
test("actual composer keeps draft and submits nothing against an old core",async()=>{
 const {requests,notices,prompt}=await submit(false);
 assert.equal(requests.length,0);assert.equal(prompt.value,"hi");assert.equal(notices.length,1);
});
