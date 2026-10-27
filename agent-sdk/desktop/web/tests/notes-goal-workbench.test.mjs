import {test} from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import vm from "node:vm";
const source=readFileSync(new URL("../panels/goal.panel.js",import.meta.url),"utf8");
const clone=value=>JSON.parse(JSON.stringify(value));
const escape=value=>String(value==null?"":value).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
const flush=()=>new Promise(resolve=>setImmediate(resolve));
function harness(options={}) {
  const nodes=new Map(), posts=[], notices=[];
  function node(id) {
    if(!nodes.has(id))nodes.set(id,{
      value:"",innerHTML:"",textContent:"",hidden:false,disabled:false,attrs:{},handlers:{},
      setAttribute(k,v){this.attrs[k]=v;},removeAttribute(k){delete this.attrs[k];},
      getAttribute(k){return this.attrs[k]??null;},
      addEventListener(k,fn){this.handlers[k]=fn;},querySelectorAll(){return[];},
      querySelector(selector){return node(selector.replace(/^#/,""));},
    });
    return nodes.get(id);
  }
  const root={innerHTML:"",querySelector(selector){return node(selector.replace(/^#/,""));}};
  const window={OwoPanels:{},location:{origin:"http://localhost"}};
  vm.runInNewContext(source,{window,document:{getElementById:node},Promise,JSON,String,Error,Date,setTimeout,clearTimeout});
  const helpers={
    baseUrl:"http://core-a",coreInstanceId:"a",esc:escape,friendlyError:error=>error.message,notify:text=>notices.push(text),
    get:options.get || (async path=>{
      if(path==="/goal")return{goals:[]};
      if(path.endsWith("/plan"))return{plan:options.plan || {steps:[]},waves:[["s"]]};
      if(path.endsWith("/status"))return{goal_status:"Pending",steps:[]};
      if(path.endsWith("/audit"))return{audit:[]};
      return{id:path.split("/")[2],objective:"目标",status:"Pending"};
    }),
    post:async(path,body)=>{posts.push([path,clone(body)]);return options.post?options.post(path,body):{ok:true,goal:{id:"created"}};}
  };
  const panel=window.OwoPanels.goal;panel.mount(root,helpers);
  return{panel,node,root,helpers,posts,notices};
}

test("plan GET to POST preserves exact equality, empty match, nonempty verification and parallel flags", async()=>{
  const steps=[
    {id:"s",worker:"echo",depends_on:[],verify:{OutputEquals:""},retries:2,input:{text:""},parallel:true},
    {id:"b",worker:"echo",depends_on:["s"],verify:"OutputNonEmpty",input:{text:"b"},parallel:false},
    {id:"c",worker:"echo",depends_on:["b"],verify:{OutputContains:""},input:null,parallel:true}
  ];
  const h=harness({plan:{steps}});
  await h.panel._test.loadGoal("g");
  const converted=clone(h.panel._test.serializableSteps({steps}));
  assert.deepEqual(converted.map(s=>s.verify),[{kind:"equals",value:""},{kind:"nonempty"},{kind:"contains",value:""}]);
  assert.deepEqual(converted.map(s=>s.parallel),[true,false,true]);
  h.node("owo-goal-steps").value=JSON.stringify(converted);
  await h.panel._test.savePlan("g");
  assert.deepEqual(h.posts[0],["/goal/g/plan",{steps:converted}]);
});

test("invalid plan members never throw into the UI or send a partial plan", async()=>{
  const h=harness();await h.panel._test.loadGoal("g");
  for(const value of ["[]","[null]",'[{"id":"s","worker":"echo","deps":"bad"}]','[{"id":"s","worker":"echo","max_retries":-1}]']) {
    h.node("owo-goal-steps").value=value;
    assert.equal(await h.panel._test.savePlan("g"),null);
  }
  assert.equal(h.posts.length,0);assert.equal(h.notices.length,4);
});

test("run configuration matches the top-level server DTO rather than an ignored config wrapper", async()=>{
  const h=harness();await flush();
  await h.panel._test.runGoal("g",h.node("run"),"运行");
  assert.deepEqual(h.posts[0],["/goal/g/run",{parallelism:2,allow_replan:true}]);
});

test("plan fetch failure disables editing while a real 404 means no saved plan", async()=>{
  let missing=false;
  const h=harness({get:async path=>{
    if(path==="/goal")return{goals:[]};
    if(path.endsWith("/plan"))throw new Error(missing?"HTTP 404":"HTTP 503");
    if(path.endsWith("/status"))throw new Error("HTTP 404");
    return{id:"g",objective:"目标",status:"Pending"};
  }});
  await h.panel._test.loadGoal("g");
  assert.match(h.node("owo-goal-detail").innerHTML,/计划读取失败/);
  h.node("owo-goal-steps").value='[{"id":"s","worker":"echo"}]';
  assert.equal(await h.panel._test.savePlan("g"),null);
  assert.equal(h.posts.length,0);
  missing=true;await h.panel._test.loadGoal("g");await flush();
  assert.doesNotMatch(h.node("owo-goal-detail").innerHTML,/计划读取失败/);
  assert.match(h.node("owo-goal-status").innerHTML,/尚未运行/);
});

test("status retains multiline escaped output, attempts, errors and truncation boundary", async()=>{
  const h=harness({get:async path=>{
    if(path==="/goal")return{goals:[]};
    if(path.endsWith("/plan"))return{plan:{steps:[]},waves:[]};
    if(path.endsWith("/status"))return{goal_status:"Failed",steps_taken:1,replan_count:0,steps:[
      {step_id:"s",status:"Failed",worker:"echo",model:"gpt-4.1-mini",attempts:3,output:"<tag>\\nline2",error:"failure detail",output_truncated:true}]};
    return{id:"g",objective:"目标",status:"Failed"};
  }});
  await h.panel._test.loadGoal("g");await flush();
  const html=h.node("owo-goal-status").innerHTML;
  assert.match(html,/&lt;tag&gt;\\nline2/);assert.match(html,/gpt-4.1-mini/);
  assert.match(html,/failure detail/);assert.match(html,/2000 字符/);assert.match(html,/<td>3<\/td>/);
});

test("an unmounted deferred goal mutation never reaches the API", async()=>{
  const h=harness();await flush();h.node("owo-goal-objective").value="目标";
  const pending=h.panel._test.createGoal();h.panel.dispose();await pending;
  assert.equal(h.posts.length,0);
});

test("a malformed list stays an error when local search changes", async()=>{
  const h=harness({get:async()=>({goals:null})});await flush();
  assert.match(h.node("owo-goal-list").innerHTML,/格式不完整/);
  h.node("owo-goal-search").handlers.input({target:{value:"filter"}});
  assert.match(h.node("owo-goal-list").innerHTML,/格式不完整/);
  assert.doesNotMatch(h.node("owo-goal-list").innerHTML,/暂无目标/);
});

test("audit renders the complete bounded response and identifies the core-wide scope", async()=>{
  const h=harness({get:async path=>{
    if(path==="/goal")return{goals:[]};
    if(path.endsWith("/plan"))return{plan:{steps:[]},waves:[]};
    if(path.endsWith("/status"))return{steps:[]};
    if(path.endsWith("/audit"))return{audit:Array.from({length:35},(_,i)=>({event:"event-"+i,detail:"detail-"+i}))};
    return{id:"g",objective:"目标",status:"Pending"};
  }});
  await h.panel._test.loadGoal("g");await h.panel._test.showAudit("g");
  const html=h.node("owo-goal-audit-box").innerHTML;
  assert.match(html,/包含其它目标/);assert.match(html,/event-34/);
});

test("initial Planning goals can run and a pending save remount is explicitly busy", async()=>{
  let release;
  const h=harness({get:async path=>{
    if(path==="/goal")return{goals:[]};
    if(path.endsWith("/plan"))return{plan:{steps:[{id:"s",worker:"echo"}]},waves:[["s"]]};
    if(path.endsWith("/status"))return{goal_status:"Planning",steps:[]};
    return{id:"g",objective:"目标",status:"Planning"};
  },post:()=>new Promise(resolve=>{release=resolve;})});
  await h.panel._test.loadGoal("g");
  assert.match(h.node("owo-goal-detail").innerHTML, /id="owo-goal-run-now" data-core-action>运行/);
  h.node("owo-goal-steps").value='[{"id":"s","worker":"echo"}]';
  const saving=h.panel._test.savePlan("g");await flush();
  await h.panel._test.loadGoal("g");
  assert.match(h.node("owo-goal-detail").innerHTML, /id="owo-goal-save-plan" data-core-action disabled data-core-busy="true" aria-busy="true"/);
  release({ok:true});await saving;
  assert.equal(h.node("owo-goal-save-plan").disabled,false);
});

test("runtime status supersedes the stored planning snapshot in list, heading and filters",async()=>{
  const goal={id:"g",objective:"已执行的目标",status:"Planning"};
  const h=harness({get:async path=>{
    if(path==="/goal")return{goals:[goal]};
    if(path.endsWith("/plan"))return{plan:{steps:[{id:"s",worker:"echo"}]},waves:[["s"]]};
    if(path.endsWith("/status"))return{goal_status:"Succeeded",steps:[]};
    return goal;
  }});
  await flush();await h.panel._test.loadGoal("g");await flush();
  assert.match(h.node("owo-goal-list").innerHTML,/已完成/);
  assert.equal(h.node("owo-goal-current-status").textContent,"已完成");
  h.node("owo-goal-filter").handlers.change({target:{value:"Succeeded"}});
  assert.match(h.node("owo-goal-list").innerHTML,/已执行的目标/);
  assert.equal(h.node("owo-goal-run-now").disabled,false);
  assert.equal(h.node("owo-goal-abort-now").disabled,true);
});
