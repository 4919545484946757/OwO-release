import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url);
require("../core/attachment-upload.js");
const A=require("../core/input-assets.js");
const cap={constraints:{structured_input_parts:true}};
const asset=(n=1,type="image",size=10)=>{const id="input-00000000-0000-4000-8000-"+String(n).padStart(12,"0");return {id,name:"same.png",mime:type==="image"?"image/png":"text/plain",size,content_part:{type,asset:{artifact_id:id,sha256:String(n%10).repeat(64)}}};};
const snapshot=(id="a",workspace="T:/workspace")=>({sessionId:id,workspace,sessions:[{id,workspace}]});
const tick=()=>new Promise(r=>setImmediate(r));
const container=()=>({innerHTML:"",events:{},addEventListener(name,fn){this.events[name]=fn;},removeEventListener(name,fn){if(this.events[name]===fn)delete this.events[name];},querySelector(){return null;}});
function setup(rows=[asset()],extra={}){let context=snapshot();const paths=[];const controller=A.create({snapshot:()=>context,get:async path=>{paths.push(path);return path==="/capabilities"?cap:rows;},post:async()=>asset(10),...extra});return {controller,paths,setContext:v=>context=v};}

test("source context requires a real selected session in the current workspace",()=>{
 assert.equal(A.sourceContext({sessions:{}}),null);assert.equal(A.sourceContext({sessions:[null],sessionId:"a",workspace:"x"}),null);
 assert.equal(A.sourceContext({...snapshot(),workspace:"other"}),null);
 assert.equal(A.sourceContext({...snapshot(),sessionId:"missing"}),null);
 assert.deepEqual(A.sourceContext(snapshot()),{sessionId:"a",workspace:"T:/workspace"});
});
test("list is session scoped, selection is opt-in, and exact refs are cloned into the team request",async()=>{
 const row=asset(),{controller:c,paths}=setup([row]);await c.refresh();assert.deepEqual(paths,["/capabilities","/session/a/attachments"]);
 assert.deepEqual(c.applyTo({objective:"test"}),{objective:"test"});assert.equal(c.select(row.id,true),true);
 const body=c.applyTo({workspace:{root:"T:/workspace"}});assert.equal(body.parent_session_id,"a");assert.deepEqual(body.content_parts,[row.content_part]);
 body.content_parts[0].asset.sha256="changed";assert.equal(c.applyTo({}).content_parts[0].asset.sha256,row.content_part.asset.sha256);
 assert.throws(()=>c.applyTo({parent_session_id:"b"}),/来源对话不一致/);assert.throws(()=>c.applyTo({workspace:{root:"other"}}),/工作区不一致/);
 c.dispose();assert.throws(()=>c.applyTo({}),/已关闭/);
});
test("legacy, audio, invalid refs and bad sizes remain visible but cannot be selected",async()=>{
 const bad=asset(4);bad.content_part.asset.sha256="broken";
 const {controller:c}=setup([{id:"old.txt",name:"old",mime:"text/plain",size:1},asset(2,"audio"),bad,{...asset(5),size:-1}]);await c.refresh();
 assert(c.view().rows.every(r=>r.unavailable));for(const row of c.view().rows)assert.equal(c.select(row.id,true),false);assert.equal(c.applyTo({}).content_parts,undefined);
});
test("old core disables task inputs without preventing ordinary attachment-free creation",async()=>{
 let calls=0;const {controller:c}=setup([],{get:async()=>{calls++;return {constraints:{}};}});await c.refresh();assert.equal(calls,1);assert.match(c.view().error,/不会仅发送文字/);assert.equal(c.view().ready,false);assert.deepEqual(c.applyTo({objective:"text"}),{objective:"text"});
});
test("refresh retains only matching id and hash; a changed hash never silently retargets selection",async()=>{
 let rows=[asset()];const {controller:c}=setup([],{get:async path=>path==="/capabilities"?cap:rows});await c.refresh();c.select(rows[0].id,true);await c.refresh();assert.equal(c.view().count,1);
 rows=[{...asset(),content_part:{...asset().content_part,asset:{...asset().content_part.asset,sha256:"f".repeat(64)}}}];await c.refresh();assert.equal(c.view().count,0);assert.equal(c.applyTo({}).content_parts,undefined);
});
test("scope changes reject nonempty selection, discard stale list responses, and allow new text-only requests",async()=>{
 let finish;let delayed=false;const {controller:c,setContext}=setup([],{get:async path=>path==="/capabilities"?cap:delayed?new Promise(r=>finish=r):[asset()]});await c.refresh();c.select(asset().id,true);setContext(snapshot("b"));assert.throws(()=>c.applyTo({}),/已切换/);
 delayed=true;const pending=c.refresh();await tick();setContext(snapshot("c"));finish([asset(3)]);await pending;assert.equal(c.view().rows.length,0);assert.equal(c.applyTo({}).content_parts,undefined);
});
test("bounded lists reject duplicate IDs, too many rows and oversized metadata",async()=>{
 for(const rows of [[asset(),asset()],Array.from({length:257},(_,i)=>asset(i+1)),[{...asset(),name:"x".repeat(512*1024)}]]){const {controller:c}=setup(rows);await c.refresh();assert.equal(c.view().ready,false);assert(c.view().error);}
});
test("selection enforces image count, combined bytes and total content parts without losing the previous selection",async()=>{
 for(const [rows,limit,message] of [[Array.from({length:9},(_,i)=>asset(i+1)),8,/8 张/],[Array.from({length:5},(_,i)=>asset(i+1,"image",5*1024*1024)),4,/20 MiB/],[Array.from({length:33},(_,i)=>asset(i+1,"file")),32,/32 个/]]){
  const {controller:c}=setup(rows);await c.refresh();for(let i=0;i<limit;i++)assert.equal(c.select(rows[i].id,true),true);assert.equal(c.select(rows[limit].id,true),false);assert.equal(c.view().count,limit);assert.match(c.view().error,message);assert.equal(c.applyTo({}).content_parts.length,limit);
 }
});
test("rendering escapes names and mounts the production media renderer payload",async()=>{
 const row={...asset(),name:'<img src=x onerror="alert(1)">'};const {controller:c}=setup([row]);const node=container();await c.mount(node);assert(node.innerHTML.includes("&lt;img"));assert(node.innerHTML.includes("版本 "+row.content_part.asset.sha256.slice(0,12)));assert(!node.innerHTML.includes('<img src=x'));c.select(row.id,true);assert(node.innerHTML.includes("data-media-artifact"));assert(node.innerHTML.includes("&quot;outputs&quot;"));assert(node.innerHTML.includes("&quot;tool&quot;:&quot;generate_image&quot;"));c.dispose();assert.deepEqual(node.events,{});
});
test("real upload helper pins the owner and auto-selects only the explicitly uploaded immutable asset",async()=>{
 const sent=[];const uploaded=asset(10);const {controller:c}=setup([asset()],{read:async()=>"data:image/png;base64,AAAA",post:async(path,body)=>{sent.push({path,body});return uploaded;}});await c.refresh();await c.uploadFiles([{name:"same.png",size:3,type:"image/png"}]);assert.equal(sent[0].path,"/session/a/attachments");assert.equal(sent[0].body.data_b64,"AAAA");assert.deepEqual(c.applyTo({}).content_parts,[uploaded.content_part]);assert.equal(c.view().rows.length,2);
});
test("pending read blocks submission, refresh cannot steal its state, and a scope switch prevents HTTP upload",async()=>{
 let release,posts=0;const {controller:c,setContext}=setup([],{read:()=>new Promise(r=>release=r),post:async()=>{posts++;return asset();}});await c.refresh();const upload=c.uploadFiles([{name:"same.png",size:3,type:"image/png"}]);assert.throws(()=>c.applyTo({}),/正在上传/);await c.refresh();assert.equal(c.view().busy,true);setContext(snapshot("b"));release("data:image/png;base64,AAAA");await upload;assert.equal(posts,0);assert.equal(c.view().context.sessionId,"b");assert.equal(c.view().count,0);
});
test("in-flight HTTP stays with the original session, and disposal cannot populate a reopened panel",async()=>{
 let release;const paths=[];const {controller:c,setContext}=setup([],{read:async()=>"data:image/png;base64,AAAA",post:path=>{paths.push(path);return new Promise(r=>release=r);}});await c.refresh();const upload=c.uploadFiles([{name:"same.png",size:3,type:"image/png"}]);await tick();setContext(snapshot("b"));c.dispose();release(asset());await upload;assert.deepEqual(paths,["/session/a/attachments"]);assert.equal(c.view().count,0);assert.equal(c.view().rows.length,0);
});

test("malformed hash metadata remains readable without crashing panel rendering",async()=>{
 const row=asset();row.content_part.asset.sha256=42;const {controller:c}=setup([row]);const node=container();await c.mount(node);assert.match(node.innerHTML,/资产引用无效/);assert.equal(c.view().ready,true);assert.equal(c.select(row.id,true),false);c.dispose();
});

test("user can explicitly remove a selected asset after capability loss and create a text-only team",async()=>{
 let available=true;const {controller:c}=setup([],{get:async path=>path==="/capabilities"?(available?cap:{}):[asset()]});await c.refresh();c.select(asset().id,true);available=false;await c.refresh();assert.throws(()=>c.applyTo({}),/尚未就绪/);assert.equal(c.clearSelection(),true);assert.deepEqual(c.applyTo({objective:"text"}),{objective:"text"});assert.match(c.view().error,/不会仅发送文字/);c.dispose();assert.equal(c.clearSelection(),false);
});
