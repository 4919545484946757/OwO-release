import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),{create,html}=require('../panels/workswarm/delivery.js');
test('delivery renders actual names safely and separates missing files',()=>{
  const text=html({title:'<script>bad</script>',status:'succeeded',files:[{name:'poem.txt',relativePath:'outputs/poem.txt',size:320,available:true,canOpen:true},{name:'missing.txt',relativePath:'outputs/missing.txt',available:false}]});
  assert.match(text,/打开文件/);assert.match(text,/所在文件夹/);assert.match(text,/尚未生成或已移走/);assert.doesNotMatch(text,/<script>/);
  assert.equal((text.match(/data-team-file-action="open"/g)||[]).length,1);
});
test('late delivery response cannot overwrite the newly selected task',async()=>{
  const queue=[],box={innerHTML:'',addEventListener(){},removeEventListener(){}};
  const view=create({box,bridge:{list:input=>new Promise(resolve=>queue.push({input,resolve}))}});
  const old=view.load('team-old','running'),fresh=view.load('team-new','succeeded');
  queue[1].resolve({teamId:'team-new',title:'NEW RESULT',files:[]});await fresh;
  queue[0].resolve({teamId:'team-old',title:'OLD RESULT',files:[]});await old;
  assert.match(box.innerHTML,/NEW RESULT/);assert.doesNotMatch(box.innerHTML,/OLD RESULT/);
});
test('disposed pages ignore pending file listings',async()=>{
  let resolve;const box={innerHTML:'',addEventListener(){},removeEventListener(){}};
  const view=create({box,bridge:{list:()=>new Promise(r=>resolve=r)}});
  const pending=view.load('team-old','succeeded');view.dispose();resolve({title:'STALE',files:[]});await pending;
  assert.doesNotMatch(box.innerHTML,/STALE/);
});

test('a failed first listing can be retried for the same task',async()=>{
  let handler,calls=0;const box={innerHTML:'',addEventListener(_event,h){handler=h;},removeEventListener(){},contains(){return true;}};
  const view=create({box,bridge:{list:async input=>{assert.equal(input.teamId,'team-retry');if(++calls===1)throw Error('temporary error');return {teamId:input.teamId,title:'RETRIED',files:[]};}}});
  await view.load('team-retry','succeeded');assert.match(box.innerHTML,/重新加载/);
  await handler({target:{closest:()=>({dataset:{teamFileAction:'refresh'}})}});
  assert.equal(calls,2);assert.match(box.innerHTML,/RETRIED/);
});
