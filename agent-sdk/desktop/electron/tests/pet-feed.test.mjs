import {test} from 'node:test';import assert from 'node:assert/strict';import vm from 'node:vm';import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../src/main/pet-feed.js',import.meta.url),'utf8');
function setup(extra=false,queryValue=true){const handlers={},shortcuts={},restored=[],presented=[],menus=[],ignored=[],regions=[],drags=[];let stored=null, focusable=false, focused=false, blurred=false;const frame={url:'http://127.0.0.1:1234/pet/'};const sent=[];const wc={mainFrame:frame,send:(...x)=>sent.push(x)};const pet={isDestroyed:()=>false,isVisible:()=>queryValue,webContents:wc,show:()=>{},moveTop:()=>{},isFocusable:()=>focusable,setFocusable:value=>{focusable=value;},focus:()=>{focused=true;},isFocused:()=>focused,blur:()=>{blurred=true;},setIgnoreMouseEvents:(...args)=>ignored.push(args)};const secondary={isDestroyed:()=>false,isVisible:()=>true,getPosition:()=>[50,60],setPosition:(x,y)=>{secondary.position=[x,y];},show:()=>{},webContents:{mainFrame:{url:'http://127.0.0.1:1234/pet/?role=implementer'},send:(...x)=>sent.push(x)}};const electron={app:{whenReady:()=>Promise.resolve()},ipcMain:{handle:(n,h)=>handlers[n]=h},clipboard:{readText:()=> 'actual clipboard text'},globalShortcut:{register:(n,h)=>{shortcuts[n]=h;return true;}},Menu:{buildFromTemplate:template=>{const menu={template,once:(name,handler)=>{menu.close=handler;},popup:()=>{menu.open=true;}};menus.push(menu);return menu;}}};const module={exports:{}};vm.runInNewContext(source,{require:n=>n==='electron'?electron:n==='node:fs'?{readFileSync:()=>{if(stored===null)throw Error('absent');return stored;},mkdirSync:()=>{},writeFileSync:(_p,value)=>{stored=value;},renameSync:()=>{}}:n==='node:path'?{join:(...p)=>p.join('/')}:null,module,console,URL,setImmediate});module.exports.installPetFeed({pet:()=>pet,main:()=>pet,workspace:()=>{},dataRoot:()=>'/test',port:()=>1234,ensureAgents:ref=>restored.push(ref),present:value=>presented.push(value),queryVisible:()=>queryValue,pets:()=>extra?[pet,secondary]:[pet],setRegion:(w,rect)=>{regions.push([w,rect]);return{ok:true};},setDragging:(w,value)=>{drags.push([w,value]);return{ok:true};}});return{handlers,shortcuts,sent,restored,presented,menus,ignored,regions,drags,secondary,focusPet:()=>pet.focus(),focusState:()=>({focusable,focused,blurred}),secondaryEvent:{sender:secondary.webContents,senderFrame:secondary.webContents.mainFrame},event:{sender:wc,senderFrame:frame}};}
test('clipboard IPC only accepts bound top-level pet and expected core origin',()=>{const f=setup();assert.equal(f.handlers['pet:feed:read'](f.event),'actual clipboard text');assert.throws(()=>f.handlers['pet:feed:read']({...f.event,sender:{}}),/untrusted/);assert.throws(()=>f.handlers['pet:feed:read']({...f.event,senderFrame:{url:f.event.senderFrame.url}}),/untrusted/);f.event.senderFrame.url='http://127.0.0.1:9999/pet/';assert.throws(()=>f.handlers['pet:feed:read'](f.event),/untrusted/);});
test('feed shortcut routes clipboard to pet, without executing a second runtime',async()=>{const f=setup();await Promise.resolve();f.shortcuts['Control+Alt+F']();assert.deepEqual(f.sent[0],['pet:feed:text','actual clipboard text']);});
test('invalid task identities are rejected before persisted/opened',()=>{const f=setup();assert.throws(()=>f.handlers['pet:feed:save'](f.event,{id:'../escape'}),/invalid/);assert.throws(()=>f.handlers['pet:feed:open'](f.event,{id:'https://external'}),/invalid/);});

test('pet body forwards a feed to the coordinator instead of creating its own run',()=>{const f=setup();f.handlers['pet:feed:forward'](f.event,'a real requirement');assert.deepEqual(f.sent.at(-1),['pet:feed:text','a real requirement']);assert.throws(()=>f.handlers['pet:feed:forward'](f.event,' '),/invalid/);});

test('a complete secondary pet can be moved independently of the main pet',()=>{const f=setup(true);f.handlers['pet:actor:move'](f.secondaryEvent,12,-5);assert.deepEqual(f.secondary.position,[62,55]);assert.equal(f.handlers['pet:actor:query'](f.secondaryEvent).visible,true);});
test('actor visibility query does not borrow another window visibility',()=>{const f=setup(false,false);assert.equal(f.handlers['pet:actor:query'](f.event).visible,false);});
test('secondary body forwards requirements to primary and cannot save a new team',()=>{const f=setup(true);f.handlers['pet:feed:forward'](f.secondaryEvent,'new requirement');assert.deepEqual(f.sent.at(-1),['pet:feed:text','new requirement']);assert.throws(()=>f.handlers['pet:feed:save'](f.secondaryEvent,{id:'team-second'}),/untrusted/);});

test('startup load restores actual member windows and null releases persisted binding',()=>{const f=setup();f.handlers['pet:feed:save'](f.event,{id:'team-one',members:[{role:'w1',member_id:'m-w1',runtime_binding:{kind:'agent',agent_id:'unique'}}]});assert.equal(f.restored.length,1);const loaded=f.handlers['pet:feed:load'](f.event);assert.equal(loaded.members[0].role,'w1');assert.equal(f.restored.length,2);f.handlers['pet:feed:save'](f.event,null);assert.equal(f.handlers['pet:feed:load'](f.event),null);});
test('pending submission identity is persisted without saving original requirement text',()=>{const f=setup();f.handlers['pet:feed:save'](f.event,{pending:true,goalId:'feed-one',text:'private requirement'});const loaded=f.handlers['pet:feed:load'](f.event);assert.equal(loaded.goalId,'feed-one');assert.equal(loaded.text,undefined);assert.equal(f.restored.length,1);assert.equal(f.restored[0].pending,true);});

test('configured Unicode role identities survive persistence without losing members',()=>{const f=setup();f.handlers['pet:feed:save'](f.event,{id:'team-unicode',members:[{role:'执行者',member_id:'m-执行者',runtime_binding:{kind:'agent',agent_id:'a'},capabilities:['review']}]});const ref=f.handlers['pet:feed:load'](f.event);assert.equal(ref.members[0].role,'执行者');assert.equal(ref.members[0].capabilities[0],'review');});

test('only the bound primary can publish a current-team presence report',()=>{
  const f=setup(true);f.handlers['pet:feed:save'](f.event,{id:'team-one'});
  const value={id:'team-one',status:'running',activeRoles:['w1']};
  assert.equal(f.handlers['pet:feed:activity'](f.event,value).ok,true);assert.equal(f.presented.length,1);
  assert.throws(()=>f.handlers['pet:feed:activity'](f.secondaryEvent,value),/untrusted/);
  assert.equal(f.handlers['pet:feed:activity'](f.event,{...value,id:'old-team'}).ok,false);assert.equal(f.presented.length,1);
  assert.throws(()=>f.handlers['pet:feed:activity'](f.event,{...value,activeRoles:[42]}),/invalid/);
});

test('native pet menu returns the selected action and preserves checked submenus',async()=>{
  const f=setup();
  const selected=f.handlers['pet:actor:menu'](f.event,[{id:'open',label:'打开工作台'},{type:'separator'},{id:'skins',label:'更换形象',submenu:[{id:'squid',label:'3D 墨鱼',checked:true}]}]);
  assert.equal(f.menus.length,1);assert.equal(f.menus[0].open,true);
  assert.equal(f.menus[0].template[2].submenu[0].type,'checkbox');assert.equal(f.menus[0].template[2].submenu[0].checked,true);
  f.menus[0].template[0].click();assert.equal(await selected,'open');
});

test('命中区几何由页面上报，发送方与形状都要校验',()=>{
  const f=setup();
  f.handlers['pet:actor:region'](f.event,{x:76,y:116,width:120,height:120,hold:false});
  assert.equal(f.regions.length,1);
  assert.equal(f.regions[0][1].x,76);
  assert.equal(f.regions[0][1].hold,false);
  assert.throws(()=>f.handlers['pet:actor:region'](f.event,null),/invalid region/);
  assert.throws(()=>f.handlers['pet:actor:region']({...f.event,sender:{}},{x:0,y:0,width:8,height:8}),/untrusted/);
});

test('拖动期间握住输入权，松手再交还（穿透切换由主进程指针轮询负责）',()=>{
  const f=setup();
  f.handlers['pet:actor:dragging'](f.event,true);
  assert.equal(f.drags.length,1);assert.equal(f.drags[0][1],true);
  f.handlers['pet:actor:dragging'](f.event,false);
  assert.equal(f.drags.at(-1)[1],false);
  assert.throws(()=>f.handlers['pet:actor:dragging'](f.event,'yes'),/invalid dragging/);
  assert.throws(()=>f.handlers['pet:actor:dragging']({...f.event,sender:{}},true),/untrusted/);
});

test('interaction end returns the keyboard while keeping the pet clickable',()=>{
  const f=setup();
  f.handlers['pet:actor:blur'](f.event);
  assert.deepEqual(f.focusState(),{focusable:false,focused:false,blurred:false},'没聚焦时 releaseFocus 是空操作');
  f.focusPet();
  f.handlers['pet:actor:blur'](f.event);
  assert.deepEqual(f.focusState(),{focusable:false,focused:true,blurred:true},'互动结束只交还键盘，不撤掉命中区的可点击状态（由主进程轮询负责）');

  const g=setup();
  g.handlers['pet:actor:chat-focus'](g.event,true);
  g.handlers['pet:actor:blur'](g.event);
  assert.deepEqual(g.focusState(),{focusable:true,focused:true,blurred:false},'聊天期间不交还键盘');

  assert.throws(()=>f.handlers['pet:actor:blur']({...f.event,sender:{}}),/untrusted/);
});

test('chat focus is lent to the pet only while the panel is open',()=>{
  const f=setup();
  f.handlers['pet:actor:chat-focus'](f.event,true);
  assert.deepEqual(f.focusState(),{focusable:true,focused:true,blurred:false});
  f.handlers['pet:actor:chat-focus'](f.event,false);
  assert.deepEqual(f.focusState(),{focusable:false,focused:true,blurred:true});
  assert.throws(()=>f.handlers['pet:actor:chat-focus'](f.event,'yes'),/invalid chat focus/);
  assert.throws(()=>f.handlers['pet:actor:chat-focus']({...f.event,sender:{}},true),/untrusted/);
});
