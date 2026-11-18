import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
const { create, patchPairing, gatewayFailure } = createRequire(import.meta.url)("../src/main/ime-connection.js");
test("connection cleanup restores only its own pairing fields and keeps user edits", () => {
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"owo-ime-"));
  try {
    const file=path.join(dir,"settings.json");
    fs.writeFileSync(file,JSON.stringify({require_pairing:false,pairing_credential_file:"old",send_session_context:false}));
    const restore=patchPairing(file,"new");
    const settings=JSON.parse(fs.readFileSync(file));settings.send_session_context=true;
    fs.writeFileSync(file,JSON.stringify(settings));restore();
    assert.deepEqual(JSON.parse(fs.readFileSync(file)),{require_pairing:false,pairing_credential_file:"old",send_session_context:true});
    const second=patchPairing(file,"second");
    fs.writeFileSync(file,JSON.stringify({require_pairing:true,pairing_credential_file:"user-changed"}));
    second();
    assert.equal(JSON.parse(fs.readFileSync(file)).pairing_credential_file,"user-changed");
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
test("previously absent pairing keys are removed on disconnect",()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"owo-ime-"));
  try {const file=path.join(dir,"settings.json");fs.writeFileSync(file,"{}");patchPairing(file,"fixture")();assert.deepEqual(JSON.parse(fs.readFileSync(file)),{});}
  finally {fs.rmSync(dir,{recursive:true,force:true});}
});

// Child lifecycle regressions: failed starts must remain visible and expose no output.
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
test("preflight failure is retained in connection status and permits retry", async () => {
  const connection = create({ runtime: () => null });
  assert.equal((await connection.start()).state, "failed");
  assert.match(connection.status().message, /重启核心/);
  assert.equal((await connection.start()).state, "failed");
});
test("fragmented pairing errors become fixed messages without leaking child output", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owo-ime-start-"));
  try {
    const proc = new EventEmitter(); proc.stdout = new PassThrough(); proc.stderr = new PassThrough();
    proc.kill = () => {}; proc.exitCode = null;
    const connection = create({
      runtime: () => ({ owned: true, port: 1234, dataRoot: dir }),
      executable: () => process.execPath, localAppData: () => dir, dataRoot: () => dir,
      originKey: "synthetic", discoverConnector: async () => ({ pid: 1, sha256: "synthetic" }),
      spawnGateway: () => proc,
    });
    assert.equal((await connection.start()).state, "connecting");
    proc.stderr.write("sensitive-child-output pairing not "); proc.stderr.write("approved");
    proc.emit("exit", 5);
    assert.equal(connection.status().state, "failed");
    assert.equal(connection.status().message, gatewayFailure(5, "declined"));
    assert.doesNotMatch(JSON.stringify(connection.status()), /sensitive-child-output/);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
test("specific configuration failures survive subsequent child exit", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owo-ime-error-"));
  try {
    const proc = new EventEmitter(); proc.stdout = new PassThrough(); proc.stderr = new PassThrough();
    proc.kill = () => {}; proc.exitCode = null;
    const connection = create({
      runtime: () => ({ owned: true, port: 1234, dataRoot: dir }), executable: () => process.execPath,
      localAppData: () => dir, dataRoot: () => dir, originKey: "synthetic",
      discoverConnector: async () => ({ pid: 1, sha256: "synthetic" }), spawnGateway: () => proc,
    });
    await connection.start();
    proc.stdout.write("configure connector credential: outside.owokey\n");
    proc.emit("exit", 1);
    assert.match(connection.status().message, /插件设置文件/);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});

test("fragmented unified task references stay separate from legacy sessions and expose no child text", async () => {
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"owo-ime-task-link-"));
  const tasks=[],sessions=[];
  try {
    const proc=new EventEmitter();proc.stdout=new PassThrough();proc.stderr=new PassThrough();
    proc.exitCode=null;proc.kill=()=>{};
    const connection=create({runtime:()=>({owned:true,port:1234,dataRoot:dir}),
      executable:()=>process.execPath,localAppData:()=>dir,dataRoot:()=>dir,originKey:"synthetic",
      discoverConnector:async()=>({pid:1,sha256:"synthetic"}),spawnGateway:()=>proc,
      onOpenTask:id=>tasks.push(id),onOpenSession:id=>sessions.push(id)});
    await connection.start();
    proc.stdout.write('private-child-text\nOWO_DESKTOP_OPEN_T');
    proc.stdout.write('ASK {"task_id":"11111111-1111-4111-8111-111111111111"}\n');
    proc.stdout.write('OWO_DESKTOP_OPEN_TASK {"task_id":"../../session/escape"}\n');
    proc.stdout.write('OWO_DESKTOP_OPEN_TASKS\n');
    assert.deepEqual(tasks,["11111111-1111-4111-8111-111111111111",null]);
    assert.deepEqual(sessions,[]);
    assert.doesNotMatch(JSON.stringify(connection.status()),/private-child-text/);
    proc.emit("exit",0);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
