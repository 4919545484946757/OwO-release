import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {createTeamFiles,relativePath}=require('../src/main/team-files.js');
async function setup(t) {
  const root=await fs.mkdtemp(path.join(os.tmpdir(),'owo-delivery-'));
  t.after(()=>fs.rm(root,{recursive:true,force:true}));
  await fs.mkdir(path.join(root,'outputs'));await fs.writeFile(path.join(root,'outputs','poem.txt'),'An actual poem');
  const plan={tasks:[{write_paths:['outputs/poem.txt','outputs/missing.txt','outputs'],verification:{requirements:[]}}]};
  const opened=[],revealed=[];
  const get=async route=>{
    if(route==='/teams/team-test')return {team:{team_id:'team-test',status:'succeeded',project_space_id:'proj-test'}};
    if(route.endsWith('/workspace'))return {workspace:{root,write_allowed_paths:['outputs']}};
    if(route.endsWith('/artifacts'))return {artifacts:[{artifact_id:'team-test:lead:v1',kind:'lead',version:1}]};
    if(route.endsWith('/content'))return {content:JSON.stringify(plan)};
    if(route.endsWith('/changes'))return {changed_files:[]};
    throw Error(route);
  };
  const service=createTeamFiles({get,open:async file=>{opened.push(file);return '';},reveal:file=>revealed.push(file)});
  return {root,plan,service,opened,revealed};
}
test('real files open and reveal the generated file, not the execution report',async t=>{
  const f=await setup(t);const data=await f.service.describe('team-test');
  assert.equal(data.files.length,2);assert.equal(data.files[0].available,true);assert.equal(data.files[1].available,false);
  await f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/poem.txt'});
  await f.service.act({teamId:'team-test',action:'reveal',relativePath:'outputs/poem.txt'});
  assert.deepEqual(f.opened,[path.join(f.root,'outputs','poem.txt')]);assert.deepEqual(f.revealed,f.opened);
});
test('missing, removed and unrelated files never launch',async t=>{
  const f=await setup(t);await fs.writeFile(path.join(f.root,'outputs','other.txt'),'other');
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/missing.txt'}));
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/other.txt'}));
  await fs.unlink(path.join(f.root,'outputs','poem.txt'));
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/poem.txt'}));assert.equal(f.opened.length,0);
});
test('traversal, absolute paths, URLs and Windows alternate streams are rejected',()=>{
  for(const invalid of ['../secret.txt','outputs/../secret.txt','C:\\secret.txt','/secret','\\\\server\\file','outputs/poem.txt:evil','https://example.com','outputs/./a.txt','outputs/\0a'])assert.throws(()=>relativePath(invalid));
  assert.equal(relativePath('outputs\\poem.txt'),'outputs/poem.txt');
});
test('registered executable files may be located but cannot run',async t=>{
  const f=await setup(t);await fs.writeFile(path.join(f.root,'outputs','tool.cmd'),'exit');f.plan.tasks[0].write_paths.push('outputs/tool.cmd');
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/tool.cmd'}));
  await f.service.act({teamId:'team-test',action:'reveal',relativePath:'outputs/tool.cmd'});assert.equal(f.opened.length,0);assert.equal(f.revealed.length,1);
});
test('workspace junctions cannot expose files outside the task directory',async t=>{
  const f=await setup(t),outside=await fs.mkdtemp(path.join(os.tmpdir(),'owo-outside-'));
  t.after(()=>fs.rm(outside,{recursive:true,force:true}));await fs.writeFile(path.join(outside,'secret.txt'),'secret');
  await fs.symlink(outside,path.join(f.root,'outputs','link'),process.platform==='win32'?'junction':'dir');
  f.plan.tasks[0].write_paths.push('outputs/link/secret.txt');
  const data=await f.service.describe('team-test');assert(!data.files.some(x=>x.relativePath.includes('secret')));
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'outputs/link/secret.txt'}));
});
test('file opening respects task write boundaries',async t=>{
  const f=await setup(t);await fs.writeFile(path.join(f.root,'private.txt'),'private');f.plan.tasks[0].write_paths.push('private.txt');
  await assert.rejects(f.service.act({teamId:'team-test',action:'open',relativePath:'private.txt'}));
});
