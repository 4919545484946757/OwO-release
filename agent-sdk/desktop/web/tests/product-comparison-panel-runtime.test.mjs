import { test } from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../panels/product-comparison.panel.js',import.meta.url),'utf8');
const flush=()=>new Promise(resolve=>setTimeout(resolve,0));
const deferred=()=>{let resolve; const promise=new Promise(r=>resolve=r);return {promise,resolve};};
function harness(){
 const timers=new Map(); let timer=0;
 const window={document:{hidden:false}, location:{origin:'http://core'},OwoPanels:{},setTimeout:fn=>{timers.set(++timer,fn);return timer;},clearTimeout:id=>timers.delete(id)};
 vm.runInNewContext(source,{window,AbortController,console});
 function root(){
   const elements=new Map();
   function element(){return {value:'',innerHTML:'',textContent:'',isConnected:true,handlers:{},attributes:{},setAttribute(k,v){this.attributes[k]=v;},addEventListener(k,fn){this.handlers[k]=fn;},querySelector(sel){if(sel==='details'||sel.endsWith(' details')||sel.includes('owo-pe-verdict')||sel.includes('owo-pe-samples'))return null;return get(sel);}};}
   function get(sel){if(!elements.has(sel))elements.set(sel,element());return elements.get(sel);}
   return {innerHTML:'',querySelector:get,get};
 }
 return {panel:window.OwoPanels['product-eval'],root,timers};
}

test('comparison creation lock and draft survive remount without stealing a new report selection',async()=>{
 const h=harness(),pending=deferred(); let posts=0;
 const helpers={baseUrl:'http://core',coreInstanceId:'one',get:async path=>path==='/product-eval/runs'?{runs:[{run_id:'chosen',status:'completed'}]}:{run_id:'chosen',status:'completed'},post:()=>{posts++;return pending.promise;}};
 const first=h.root();h.panel.mount(first,helpers);
 first.get('[name="only"]').value='document-structured-extract';first.get('form').handlers.input();
 const creating=first.get('form').handlers.submit({preventDefault(){}});
 assert.equal(posts,1);h.panel.dispose();
 const second=h.root();h.panel.mount(second,helpers);
 assert.equal(second.get('[name="only"]').value,'document-structured-extract');
 assert.equal(second.get('.owo-pe-start').disabled,true);
 await second.get('form').handlers.submit({preventDefault(){}});assert.equal(posts,1);
 second.get('.owo-pe-history').handlers.click({target:{closest:()=>({dataset:{runId:'chosen'}})}});await flush();
 pending.resolve({run_id:'created',status:'completed'});await creating;await flush();
 assert.equal(second.get('.owo-pe-start').disabled,false);
 assert.match(second.get('.owo-pe-report').innerHTML,/运行 ID：chosen/);
 assert.doesNotMatch(second.get('.owo-pe-report').innerHTML,/运行 ID：created/);
 h.panel.dispose();assert.equal(h.timers.size,0);
});

test('comparison mutation locks are scoped to the daemon instance and detached handlers cannot submit',async()=>{
 const h=harness(),pending=deferred();let postsA=0,postsB=0;
 const base={get:async()=>({runs:[]}),post:()=>{postsA++;return pending.promise;},baseUrl:'http://core',coreInstanceId:'a'};
 const a=h.root();h.panel.mount(a,base);const creating=a.get('form').handlers.submit({preventDefault(){}});
 const b=h.root();h.panel.mount(b,{...base,coreInstanceId:'b',post:async()=>{postsB++;return {run_id:'b',status:'completed'};},get:async path=>path==='/product-eval/runs'?{runs:[]}:{run_id:'b',status:'completed'}});
 assert.equal(b.get('.owo-pe-start').disabled,false);
 await a.get('form').handlers.submit({preventDefault(){}});assert.equal(postsA,1);
 await b.get('form').handlers.submit({preventDefault(){}});assert.equal(postsB,1);
 pending.resolve({run_id:'a',status:'completed'});await creating;assert.doesNotMatch(b.get('.owo-pe-notice').textContent,/已创建运行：a/);
 h.panel.dispose();
});


test('Ctrl Home keeps the selected report tab and remains available for scrolling',async()=>{
 const h=harness(),root=h.root();h.panel.mount(root,{get:async()=>({runs:[]}),post:async()=>({})});
 root.get('[data-pe-tab="report"]').handlers.click();
 let prevented=false;
 root.get('[data-pe-tab="report"]').handlers.keydown({key:'Home',ctrlKey:true,preventDefault(){prevented=true;}});
 assert.equal(prevented,false);assert.equal(root.get('#pe-pane-report').hidden,false);assert.equal(root.get('#pe-pane-create').hidden,true);
 h.panel.dispose();
});
