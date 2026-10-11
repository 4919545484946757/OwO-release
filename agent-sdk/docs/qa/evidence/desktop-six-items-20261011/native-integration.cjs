
"use strict";
const {app,BrowserWindow,safeStorage,session,net}=require("electron");
const fs=require("node:fs"),path=require("node:path"),crypto=require("node:crypto"),assert=require("node:assert/strict");
const base=path.resolve(__dirname,"../../../../desktop/electron/src/main");
const {CustomModelVault}=require(path.join(base,"custom-model-vault.js"));
const {migrateOrigins}=require(path.join(base,"legacy-model-migration.js"));
app.on("window-all-closed",()=>{});
app.whenReady().then(async()=>{
 const phase=process.argv.includes("--restore")?"restore":"save";
 const vault=new CustomModelVault(path.join(__dirname,"native-vault.json"),safeStorage);
 try{
  assert.ok(safeStorage.isEncryptionAvailable());
  if(phase==="save"){
   const secret=crypto.randomBytes(32).toString("hex"),model={id:"native-fixture",baseUrl:"http://127.0.0.1:48881/v1"};
   await vault.upsert({...model,apiKey:secret});
   fs.writeFileSync(path.join(__dirname,"native-hash.json"),JSON.stringify({hash:crypto.createHash("sha256").update(secret).digest("hex")}));
   assert.ok(!fs.readFileSync(vault.file,"utf8").includes(secret));
   assert.ok(!JSON.stringify(vault.snapshot()).includes(secret));
   // Seed this isolated partition on a past loopback port, without contacting the network.
   session.defaultSession.protocol.handle("http",()=>new Response("<!doctype html><title>Fixture</title>",{headers:{"Content-Type":"text/html"}}));
   const w=new BrowserWindow({show:false,webPreferences:{sandbox:true,contextIsolation:true,nodeIntegration:false}});
   await w.loadURL("http://127.0.0.1:48882/");
   await w.webContents.executeJavaScript("localStorage.setItem('owo.custom-models',"+JSON.stringify(JSON.stringify([{id:"migrated-native",baseUrl:"http://127.0.0.1:48883/v1"}]))+");localStorage.setItem('owo.model-keys',"+JSON.stringify(JSON.stringify({"migrated-native":secret}))+")");
   w.destroy();session.defaultSession.protocol.unhandle("http");
   fs.writeFileSync(path.join(__dirname,"migration-fixture.log"),'[core] '+JSON.stringify({event:"core_ready",port:48882})+"\n");
   const result=await migrateOrigins({BrowserWindow,session,net,logFile:path.join(__dirname,"migration-fixture.log"),vault,userData:app.getPath("userData")});
   assert.equal(result.migrated,1);assert.equal(result.conflicts,0);
  }else{
   const hash=JSON.parse(fs.readFileSync(path.join(__dirname,"native-hash.json"))).hash;
   const values=Object.entries(vault.environment()).filter(([k])=>k.startsWith("OWO_CUSTOM_MODEL_SECRET_"));
   assert.equal(values.length,2);for(const [,value]of values)assert.equal(crypto.createHash("sha256").update(value).digest("hex"),hash);
   // A different port exposes the migrated model from the native vault.
   session.defaultSession.protocol.handle("http",()=>new Response("<!doctype html><title>Restored</title>",{headers:{"Content-Type":"text/html"}}));
   const w=new BrowserWindow({show:false,webPreferences:{sandbox:true,contextIsolation:true,nodeIntegration:false}});
   await w.loadURL("http://127.0.0.1:48884/");
   assert.equal(await w.webContents.executeJavaScript("localStorage.getItem('owo.model-keys')"),null);
   await w.loadURL("http://127.0.0.1:48882/");
   assert.equal(await w.webContents.executeJavaScript("localStorage.getItem('owo.model-keys')"),null);
   w.destroy();session.defaultSession.protocol.unhandle("http");
  }
  console.log("NATIVE_VAULT "+phase+" PASS (DPAPI, isolated partition, no cloud requests)");
  app.exit(0);
 }catch(error){console.error("NATIVE_VAULT FAIL "+phase+" "+error.name+" "+error.message);app.exit(1);}
});
