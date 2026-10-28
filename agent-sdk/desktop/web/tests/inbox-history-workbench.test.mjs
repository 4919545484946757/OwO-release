import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url);
const inbox=require("../panels/action-center.panel.js");
const history=require("../panels/project-history.panel.js");

test("History delegates search input to the persistent filter container before fields exist", async () => {
  const listeners={};
  const filters={innerHTML:"",addEventListener:(name,fn)=>{listeners[name]=fn;}};
  const list={innerHTML:"",addEventListener(){}};
  const root={innerHTML:"",querySelector:selector=>selector==="#ph-filters-box"?filters:selector==="#ph-list"?list:null};
  history.mount(root,{get:async path=>path==="/teams"?{teams:[{team_id:"matching",status:"failed"},{team_id:"hidden",status:"succeeded"}]}:{artifacts:[]}});
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(typeof listeners.input,"function");
  listeners.input({target:{id:"ph-f-q",value:"matching"}});
  assert.equal(history._test.state.filters.q,"matching");
  assert.match(list.innerHTML,/data-ph-row="matching"/);
  assert.doesNotMatch(list.innerHTML,/data-ph-row="hidden"/);
  history.dispose();
  history._test.state.filters={status:"",template:"",project:"",q:""};
});

test("Both pages navigate through the shared mounted team helper", () => {
  const opened=[];
  const root={innerHTML:"",querySelector:()=>null};
  inbox.mount(root,{get:async()=>({items:[]}),openTeam:id=>opened.push(["inbox",id])});
  inbox._test.gotoWorkswarm("team-inbox");
  inbox.dispose();
  history.mount(root,{get:async()=>({teams:[]}),openTeam:id=>opened.push(["history",id])});
  history._test.gotoWorkswarm("team-history");
  history.dispose();
  assert.deepEqual(opened,[["inbox","team-inbox"],["history","team-history"]]);
});

test("Inbox drafts survive same-host remounts and cannot leak to another core", () => {
  const root={innerHTML:"",querySelector:()=>null};
  inbox.mount(root,{baseUrl:"core-a",get:async()=>({items:[]})});
  inbox._test.state.drafts.item={result:"保留这份草稿"};
  inbox.dispose();
  inbox.mount(root,{baseUrl:"core-a",get:async()=>({items:[]})});
  assert.equal(inbox._test.state.drafts.item.result,"保留这份草稿");
  inbox.dispose();
  inbox.mount(root,{baseUrl:"core-b",get:async()=>({items:[]})});
  assert.equal(inbox._test.state.drafts.item,undefined);
  inbox.dispose();
});

test("Inbox holds item lock through post-action refresh and clears only successful drafts", async () => {
  const T=inbox._test;
  let finishRefresh, entered;
  const refreshStarted=new Promise(resolve=>{entered=resolve;});
  T.state.inbox=[{item_id:"human:t:s:1",kind:"human_result",status:"claimed",assignee:"",summary:"人工验收"}];
  T.state.inboxSource="inbox";
  T.state.drafts["human:t:s:1"]={result:"已完成"};
  T.setTransport({post:async()=>({}),get:()=>{entered();return new Promise(resolve=>{finishRefresh=resolve;});}});
  const action=T.startInboxAction("human:t:s:1","resolve",{result:"已完成"});
  await refreshStarted;
  assert.equal(T.itemBusy("human:t:s:1"),true);
  assert.equal(T.state.drafts["human:t:s:1"],undefined);
  finishRefresh({items:[]});
  await action;
  assert.equal(T.itemBusy("human:t:s:1"),false);
  assert.match(T.state.notice,/已处理/);
});


test("Returning during an in-flight action refreshes current inbox instead of leaving stale disabled controls", async () => {
  const T=inbox._test, root={innerHTML:"",querySelector:()=>null};
  const item={item_id:"returning",kind:"human_result",status:"claimed",assignee:"",summary:"处理中"};
  let done=false, finishPost, entered;
  const started=new Promise(resolve=>{entered=resolve;});
  const helpers={baseUrl:"return-core",get:async()=>({items:done?[]:[item]}),
    post:()=>{entered();return new Promise(resolve=>{finishPost=resolve;});}};
  inbox.mount(root,helpers);
  await new Promise(resolve=>setImmediate(resolve));
  const action=T.startInboxAction("returning","resolve",{result:"完成"});
  await started;
  inbox.dispose();
  inbox.mount(root,helpers);
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(T.itemBusy("returning"),true);
  done=true;finishPost({});
  await action;
  assert.equal(T.itemBusy("returning"),false);
  assert.equal(T.state.inbox.length,0);
  inbox.dispose();
});
