import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const {createPetPresence}=createRequire(import.meta.url)('../src/main/pet-presence.js');
function fixture(){
  const calls=[];
  function win(name,position){let visible=false;return{name,position,isDestroyed:()=>false,isVisible:()=>visible,getPosition(){return this.position;},setPosition(x,y){this.position=[x,y];},hide(){visible=false;calls.push([name,'hide']);},show(){visible=true;calls.push([name,'show']);},showInactive(){this.show();}};}
  const primary=win('lead',[100,200]),writer=win('w1',[400,200]),reviewer=win('reviewer',[700,200]);
  const peers=new Map([['w1',writer],['reviewer',reviewer]]),windows=[primary,writer,reviewer];
  const presence=createPetPresence({primary:()=>primary,peers:()=>peers});
  const ref={id:'team-1',members:['lead','w1','reviewer'].map(role=>({role,runtime_binding:{kind:'agent'}}))};
  const shown=()=>windows.filter(w=>w.isVisible()).map(w=>w.name);
  const update=(activeRoles,status='running',id='team-1')=>presence.update({id,status,activeRoles});
  return{presence,ref,shown,update,primary,writer,reviewer,calls};
}
test('only the executing agent is shown, in one shared position',()=>{
  const f=fixture();f.presence.bind(f.ref);assert.deepEqual(f.shown(),['lead']);
  f.update(['lead']);f.update(['w1']);assert.deepEqual(f.shown(),['w1']);assert.deepEqual(f.writer.position,[100,200]);
  f.writer.setPosition(150,250);f.update(['reviewer']);assert.deepEqual(f.shown(),['reviewer']);assert.deepEqual(f.reviewer.position,[150,250]);
  f.update([],'succeeded');assert.deepEqual(f.shown(),['lead']);assert.deepEqual(f.primary.position,[150,250]);
});
test('the departing actor is hidden before the incoming one appears',()=>{
  const f=fixture();f.presence.bind(f.ref);f.calls.length=0;f.update(['w1']);
  assert(f.calls.findIndex(([n,a])=>n==='lead'&&a==='hide')<f.calls.findIndex(([n,a])=>n==='w1'&&a==='show'));
});
test('parallel executions keep one stable actor until it stops',()=>{
  const f=fixture();f.presence.bind(f.ref);f.update(['w1']);f.update(['reviewer','w1']);assert.deepEqual(f.shown(),['w1']);
  f.update(['reviewer']);assert.deepEqual(f.shown(),['reviewer']);
});
test('phase gaps, waiting approval and connection loss do not expose waiting members',()=>{
  const f=fixture();f.presence.bind(f.ref);f.update(['w1']);f.update([]);assert.deepEqual(f.shown(),['w1']);
  f.update([],'awaitinghuman');f.presence.apply();assert.deepEqual(f.shown(),['w1']);
});
test('global hide wins over polling and a subsequent show restores only the active actor',()=>{
  const f=fixture();f.presence.bind(f.ref);f.presence.setVisible(false);f.update(['w1']);f.presence.apply();assert.deepEqual(f.shown(),[]);
  f.presence.setVisible(true);assert.deepEqual(f.shown(),['w1']);assert(f.presence.isVisible());
});
test('stale or unknown actor reports cannot switch the active team',()=>{
  const f=fixture();f.presence.bind(f.ref);f.update(['w1']);assert.equal(f.update(['reviewer'],'running','old-team'),false);
  f.update(['foreign']);assert.deepEqual(f.shown(),['w1']);
});
test('binding updates retain active actors, while forget and new teams restore the primary',()=>{
  const f=fixture();f.presence.bind(f.ref);f.update(['w1']);f.presence.bind({...f.ref});assert.deepEqual(f.shown(),['w1']);
  f.presence.bind(null);assert.deepEqual(f.shown(),['lead']);f.presence.bind({...f.ref,id:'team-2'});assert.deepEqual(f.shown(),['lead']);
});
test('failure and cancellation leave one primary body ready for recovery or a new feed',()=>{
  for(const status of ['failed','cancelled']){const f=fixture();f.presence.bind(f.ref);f.update(['reviewer']);f.update([],status);assert.deepEqual(f.shown(),['lead']);}
});
