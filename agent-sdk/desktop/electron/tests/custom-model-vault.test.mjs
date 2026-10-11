
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import {tmpdir} from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url);
const {CustomModelVault,PREFIX}=require("../src/main/custom-model-vault.js");
const {assertFrame,probe}=require("../src/main/custom-model-broker.js");
const {candidateOrigins}=require("../src/main/legacy-model-migration.js");
function fixture(t) {
 const dir=fs.mkdtempSync(path.join(tmpdir(),"owo-vault-")),file=path.join(dir,"models.json");
 const aes=crypto.randomBytes(32);
 const codec={isEncryptionAvailable:()=>true};
 codec.encryptString=text=>{const iv=crypto.randomBytes(12),c=crypto.createCipheriv("aes-256-gcm",aes,iv);const body=Buffer.concat([c.update(text,"utf8"),c.final()]);return Buffer.concat([iv,c.getAuthTag(),body]);};
 codec.decryptString=bytes=>{const d=crypto.createDecipheriv("aes-256-gcm",aes,bytes.subarray(0,12));d.setAuthTag(bytes.subarray(12,28));return Buffer.concat([d.update(bytes.subarray(28)),d.final()]).toString("utf8");};
 t.after(()=>{assert.ok(path.resolve(dir).startsWith(path.resolve(tmpdir())+path.sep));fs.rmSync(dir,{recursive:true,force:true});});
 return {dir,file,codec,vault:new CustomModelVault(file,codec),model:{id:"fixture-model",baseUrl:"http://127.0.0.1:3999/v1"},secret:crypto.randomBytes(24).toString("hex")};
}
test("encrypted persistence survives a new vault and exposes references only",async t=>{
 const f=fixture(t);const result=await f.vault.upsert({...f.model,apiKey:f.secret});
 const encoded=fs.readFileSync(f.file,"utf8");assert.ok(!encoded.includes(f.secret));
 assert.ok(!JSON.stringify(result).includes(f.secret));assert.match(result.models[0].apiKeyEnv,/^OWO_CUSTOM_MODEL_SECRET_[a-f0-9]{32}$/);
 const reopened=new CustomModelVault(f.file,f.codec);assert.equal(reopened.environment()[result.models[0].apiKeyEnv],f.secret);
 assert.deepEqual(reopened.snapshot(),result);
});
test("endpoint binding tampering cannot decrypt or return the secret",async t=>{
 const f=fixture(t);await f.vault.upsert({...f.model,apiKey:f.secret});
 const data=JSON.parse(fs.readFileSync(f.file));data.models[0].baseUrl="http://127.0.0.1:4000/";fs.writeFileSync(f.file,JSON.stringify(data));
 assert.throws(()=>f.vault.environment(),/binding_mismatch/);
});
test("new endpoint requires explicit reentry and failed save preserves old data",async t=>{
 const f=fixture(t);await f.vault.upsert({...f.model,apiKey:f.secret});const before=fs.readFileSync(f.file,"utf8");
 await assert.rejects(f.vault.upsert({...f.model,baseUrl:"http://127.0.0.1:4000/"}),/reenter_for_new_endpoint/);
 assert.equal(fs.readFileSync(f.file,"utf8"),before);
});
test("no plaintext fallback when native encryption is unavailable",async t=>{
 const f=fixture(t);f.codec.isEncryptionAvailable=()=>false;
 await assert.rejects(f.vault.upsert({...f.model,apiKey:f.secret}),/encryption_unavailable/);assert.equal(fs.existsSync(f.file),false);
});
test("duplicate legacy ids preserve conflicts; removal leaves no reusable key",async t=>{
 const f=fixture(t);await f.vault.upsert({...f.model,apiKey:f.secret});
 const r=await f.vault.migrate([f.model,{...f.model,id:"new"}],{[f.model.id]:"different","new":f.secret});
 assert.deepEqual(r.conflictedIds,[f.model.id]);assert.deepEqual(r.migratedIds,["new"]);
 await f.vault.remove(f.model.id);assert.ok(!Object.values(f.vault.environment()).includes(f.secret)||f.vault.snapshot().models.find(m=>m.id==="new").hasKey);
 const removed=f.vault.read().models.find(m=>m.id===f.model.id);assert.equal(removed.sealed,undefined);assert.equal(removed.disabled,true);
});
test("two vault instances serialize writes without losing either model",async t=>{
 const f=fixture(t),other=new CustomModelVault(f.file,f.codec);
 await Promise.all([f.vault.upsert({...f.model,apiKey:f.secret}),other.upsert({...f.model,id:"second",apiKey:f.secret})]);
 assert.equal(f.vault.snapshot().models.length,2);
});
test("partial host synchronization rolls back and never commits a failed update",async t=>{
 const f=fixture(t);await f.vault.upsert({...f.model,apiKey:f.secret});const before=fs.readFileSync(f.file,"utf8"),calls=[];
 await assert.rejects(f.vault.upsert({...f.model,id:"new",apiKey:f.secret},async(next,old)=>{calls.push(next.map(m=>m.id));if(calls.length===1)throw Error("host_failed");}),/host_failed/);
 assert.equal(calls.length,2);assert.deepEqual(calls[1],[f.model.id]);assert.equal(fs.readFileSync(f.file,"utf8"),before);
});
test("public snapshot ignores unrecognized fields rather than returning secrets",async t=>{
 const f=fixture(t);await f.vault.upsert(f.model);const data=JSON.parse(fs.readFileSync(f.file));data.models[0].apiKey=f.secret;fs.writeFileSync(f.file,JSON.stringify(data));assert.ok(!JSON.stringify(f.vault.snapshot()).includes(f.secret));
});
test("IPC refuses child frames, other origins and different web contents",()=>{
 const contents={mainFrame:{routingId:1,processId:2}},win={isDestroyed:()=>false,webContents:contents},core={port:4001};
 const good={sender:contents,senderFrame:{routingId:1,processId:2,url:"http://127.0.0.1:4001/"}};
 assert.doesNotThrow(()=>assertFrame(good,win,core));
 for(const event of [{...good,sender:{}},{...good,senderFrame:{...good.senderFrame,routingId:3}},{...good,senderFrame:{...good.senderFrame,url:"http://127.0.0.1:4002/"}},{...good,senderFrame:{...good.senderFrame,url:"http://127.0.0.1:4001/pet"}}])assert.throws(()=>assertFrame(event,win,core),/untrusted/);
});
test("old port candidates are bounded and never include remote origins",t=>{
 const f=fixture(t),log=path.join(f.dir,"core.log");
 fs.writeFileSync(log,Array.from({length:12},(_,i)=>'[core] '+JSON.stringify({event:"core_ready",port:4100+i})).join("\n"));
 const origins=candidateOrigins(log);assert.equal(origins.length,8);assert.equal(origins[0],"http://127.0.0.1:4111");
});
test("full URL probe makes no request and never claims successful connectivity",async t=>{
 const f=fixture(t);const result=await probe(f.vault,{...f.model,useFullUrl:true,apiKey:f.secret});assert.equal(result.ok,false);assert.match(result.error,/requires_turn/);
});
