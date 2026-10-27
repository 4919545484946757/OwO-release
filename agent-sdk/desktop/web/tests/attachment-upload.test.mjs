import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
const require=createRequire(import.meta.url),U=require("../core/attachment-upload.js");
const file=(name="note.txt",size=3,type="text/plain")=>({name,size,type});
const read=async f=>"data:"+(f.type||"application/octet-stream")+";base64,"+"A".repeat(4*Math.ceil(f.size/3));

test("image quota and unsupported formats fail before reading, while exactly 5 MiB is allowed",async()=>{
 assert.equal(U.plan(file("照片.PNG",U.imageBytes,"image/png")).image,true);
 let reads=0,requests=0;const errors=[];
 await U.upload([file("large.png",U.imageBytes+1,"image/png"),file("photo.heic",10,"image/heic"),file("empty.jpg",0,"image/jpeg")],{
  sessionId:"a",read:async()=>{reads++;},request:async()=>{requests++;},onError:(_,error)=>errors.push(error.message)
 });
 assert.equal(reads,0);assert.equal(requests,0);assert.equal(errors.length,3);assert.match(errors[0],/5 MiB/);assert.match(errors[1],/PNG、JPEG/);
});

test("raw file boundary includes base64 expansion and actual UTF-8 JSON metadata",()=>{
 const f=file("中文\"名称.txt",1,"application/octet-stream");
 const overhead=new TextEncoder().encode(JSON.stringify({name:f.name,mime:f.type,data_b64:""})).length;
 const largest=3*Math.floor((U.requestBytes-overhead)/4);
 const allowed=U.plan({...f,size:largest});assert(allowed.requestBytes<=U.requestBytes);
 assert.throws(()=>U.plan({...f,size:largest+1}),/32 MiB/);
 assert.throws(()=>U.plan(file("big.txt",50*1024*1024)),/尚未读取文件/);
 assert.equal(U.plan(file("empty.txt",0)).requestBytes,new TextEncoder().encode(JSON.stringify({name:"empty.txt",mime:"text/plain",data_b64:""})).length);
 for(const f of [file("x",NaN),file("x",-1),file("x",Number.MAX_SAFE_INTEGER+1),file("",1)])assert.throws(()=>U.plan(f));
});

test("upload pins the session path, uses authenticated client options, and keeps processing after one rejected file",async()=>{
 const paths=[],accepted=[],errors=[];
 await U.upload([file("big.png",U.imageBytes+1),file("small.txt",3)],{sessionId:"original/session",read,request:async(path,options)=>{
  paths.push(path);assert.equal(options.method,"POST");const body=JSON.parse(options.body);assert.equal(body.name,"small.txt");assert.equal(body.data_b64,"AAAA");return {id:"small.txt",name:body.name};
 },onUploaded:r=>accepted.push(r),onError:(_,e)=>errors.push(e.message)});
 assert.deepEqual(paths,["/session/original%2Fsession/attachments"]);assert.equal(accepted.length,1);assert.equal(errors.length,1);
});

test("switching while FileReader runs prevents any request and discards remaining files",async()=>{
 let current=true,finish,reads=0,requests=0;const stopped=[];
 const pending=U.upload([file(),file("later.txt")],{sessionId:"a",isCurrent:()=>current,read:()=>{reads++;return new Promise(r=>finish=r);},request:async()=>{requests++;},onStopped:s=>stopped.push(s)});
 current=false;finish(await read(file()));await pending;assert.equal(reads,1);assert.equal(requests,0);assert.deepEqual(stopped,[{requestStarted:false}]);
});

test("switching during an HTTP upload never adds the original session's reference to the new composer",async()=>{
 let current=true,finish;const accepted=[],stopped=[],paths=[];
 const pending=U.upload([file()],{sessionId:"session-a",isCurrent:()=>current,read,request:path=>{paths.push(path);return new Promise(r=>finish=r);},onUploaded:r=>accepted.push(r),onStopped:s=>stopped.push(s)});
 await new Promise(r=>setImmediate(r));current=false;finish({id:"original-file"});await pending;
 assert.deepEqual(paths,["/session/session-a/attachments"]);assert.equal(accepted.length,0);assert.deepEqual(stopped,[{requestStarted:true}]);
});

test("bad read data or server replies fail visibly without inventing attachment IDs",async()=>{
 for(const [readResult,response] of [["not-data",{id:"x"}],["data:text/plain;base64,AA",{id:"x"}],["data:text/plain;base64,AAAA",{}]]){
  const errors=[],accepted=[];let requests=0;
  await U.upload([file()],{sessionId:"a",read:async()=>readResult,request:async()=>{requests++;return response;},onError:(_,e)=>errors.push(e.message),onUploaded:r=>accepted.push(r)});
  assert.equal(errors.length,1);assert.equal(accepted.length,0);assert.equal(requests,readResult.endsWith("AAAA")?1:0);
 }
 const errors=[];await U.upload([file()],{sessionId:"a",read,request:async()=>{throw new Error("permission denied");},onError:(_,e)=>errors.push(e.message)});assert.deepEqual(errors,["permission denied"]);
});

test("main composer uses current generation, deduplicates references, and restores file input on completion",async()=>{
 const domain=readFileSync(new URL("../app-domain.js",import.meta.url),"utf8");
 const body=domain.slice(domain.indexOf("async function uploadAttachments(files) {"),domain.indexOf("// ---------- 审批条"));
 let pending,errors=[],toasts=[],renders=0;const state={sessionId:"a",attachments:[],selectionVersion:1},input={value:"selected"};
 const context={OwoAttachmentUpload:U,state,api:async()=>({id:"same",name:"note.txt",mime:"text/plain"}),$:()=>input,renderAttachmentChips:()=>renders++,addMessage:(_,text)=>errors.push(text),showToast:t=>toasts.push(t)};
 context.OwoAttachmentUpload={upload:(files,o)=>{pending=o;return Promise.resolve();}};
 const upload=runInNewContext(body+"\nuploadAttachments;",context);
 await upload([file()]);pending.onUploaded({id:"same"});pending.onUploaded({id:"same"});assert.equal(state.attachments.length,1);assert.equal(renders,2);assert.equal(input.value,"");
 assert.equal(pending.isCurrent(),true);state.selectionVersion++;assert.equal(pending.isCurrent(),false);
 state.sessionId="b";assert.equal(pending.isCurrent(),false);pending.onStopped({requestStarted:true});assert.match(toasts[0],/仍归原会话/);
 assert.equal(errors.length,0);
});


test("composer snapshots FileList before clearing it and an old upload completion cannot erase a newer selection",async()=>{
 const domain=readFileSync(new URL("../app-domain.js",import.meta.url),"utf8"),body=domain.slice(domain.indexOf("async function uploadAttachments(files) {"),domain.indexOf("// ---------- 审批条"));
 const files=[file()];let inputValue="initial",finish,passed;
 const input={get value(){return inputValue;},set value(v){inputValue=v;if(v==="")files.length=0;}};
 const context={state:{sessionId:"a",selectionVersion:1,attachments:[]},$:()=>input,OwoAttachmentUpload:{upload:(items)=>{passed=items;return new Promise(r=>finish=r);}},renderAttachmentChips(){},addMessage(){},showToast(){},api(){}};
 const upload=runInNewContext(body+"\nuploadAttachments;",context),pending=upload(files);
 assert.equal(files.length,0);assert.equal(passed.length,1);input.value="new-selection";finish();await pending;assert.equal(input.value,"new-selection");
});

const assetUpload=(type="image",id="input-12345678-1234-4321-9876-123456789abc")=>({id,name:"same.png",content_part:{type,asset:{artifact_id:id,sha256:"a".repeat(64)}}});
test("turn input preserves immutable hash references, separates legacy IDs and deduplicates",()=>{
 const first=assetUpload(),second=assetUpload("file","input-12345678-1234-4321-9876-123456789def");
 const inputs=U.turnInput([first,{id:"legacy.txt"},first,second]);
 assert.deepEqual(inputs,{attachments:["legacy.txt"],content_parts:[first.content_part,second.content_part]});
 inputs.content_parts[0].asset.sha256="b".repeat(64);assert.equal(first.content_part.asset.sha256,"a".repeat(64));
});
test("unsupported media and invalid asset references fail before a turn is submitted",()=>{
 for(const type of ["audio","video"])assert.throws(()=>U.turnInput([assetUpload(type)]),/输入适配器/);
 for(const mutate of [u=>u.content_part.asset.sha256="invalid",u=>u.content_part.asset.artifact_id="input-other",u=>u.content_part.type="unknown"]){const u=assetUpload();mutate(u);assert.throws(()=>U.turnInput([u]),/引用无效/);}
 assert.throws(()=>U.turnInput(Array.from({length:33},(_,i)=>({id:"legacy-"+i}))),/32/);
 assert.throws(()=>U.turnInput(Array.from({length:9},(_,i)=>assetUpload("image","input-12345678-1234-4321-9876-123456789ab"+i))),/8/);
});
