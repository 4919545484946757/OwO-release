import {test} from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {EventEmitter} from 'node:events';
const source=readFileSync(new URL('../src/main/pet-task-confirmation.js',import.meta.url),'utf8');
function fixture(){
 const handlers={},windows=[];let project='D:/demo';
 class Window extends EventEmitter{
  constructor(options){super();this.options=options;this.destroyed=false;this.webContents=new EventEmitter();this.webContents.mainFrame={url:''};this.webContents.setWindowOpenHandler=()=>{};windows.push(this);}
  isDestroyed(){return this.destroyed;}show(){this.visible=true;}focus(){}
  async loadFile(file){this.webContents.mainFrame.url=pathToFileURL(file).href;this.emit('ready-to-show');}
  destroy(){this.destroyed=true;this.emit('closed');}
 }
 const frame={url:'http://127.0.0.1:1234/pet/'},pet={isDestroyed:()=>false,isVisible:()=>true,webContents:{mainFrame:frame}};
 const module={exports:{}};
 vm.runInNewContext(source,{module,__dirname:'D:/app',URL,Error,Promise,require:name=>name==='electron'?{BrowserWindow:Window,ipcMain:{handle:(n,h)=>handlers[n]=h},screen:{getPrimaryDisplay:()=>({workArea:{x:0,y:0,width:1920,height:1080}})}}:name==='node:path'?path:name==='node:url'?{pathToFileURL}:name==='node:crypto'?{randomUUID:()=> 'request-one'}:null});
 module.exports.installTaskConfirmation({pet:()=>pet,pets:()=>[pet],workspace:()=>project,port:()=>1234});
 return{handlers,windows,event:{sender:pet.webContents,senderFrame:frame},dialogEvent:()=>({sender:windows.at(-1).webContents,senderFrame:windows.at(-1).webContents.mainFrame}),setWorkspace:p=>project=p};
}
const input={text:'original requirement',workspace:'D:/demo',roles:[{name:'指挥员',model:'model'}]};
test('task is editable and only an explicit answer completes confirmation',async()=>{const f=fixture();const result=f.handlers['pet:task:confirm'](f.event,input);const data=f.handlers['pet:task:input'](f.dialogEvent());assert.equal(data.text,input.text);assert.equal(data.workspace,input.workspace);assert.equal(f.windows[0].options.webPreferences.nodeIntegration,false);assert.equal(f.windows[0].options.modal,true);f.handlers['pet:task:answer'](f.dialogEvent(),{requestId:data.requestId,action:'confirm',text:'corrected task'});assert.equal((await result).text,'corrected task');assert(f.windows[0].destroyed);});
test('cancel and closing the window return no approved task',async()=>{for(const close of [false,true]){const f=fixture();const result=f.handlers['pet:task:confirm'](f.event,input);if(close)f.windows[0].destroy();else f.handlers['pet:task:answer'](f.dialogEvent(),{requestId:'request-one',action:'cancel'});assert.equal(await result,null);}});
test('foreign frames and stale answers cannot approve a task',async()=>{const f=fixture();await assert.rejects(f.handlers['pet:task:confirm']({...f.event,sender:{}},input),/untrusted/);const result=f.handlers['pet:task:confirm'](f.event,input);assert.throws(()=>f.handlers['pet:task:input'](f.event),/untrusted/);assert.throws(()=>f.handlers['pet:task:answer'](f.dialogEvent(),{requestId:'old',action:'confirm',text:'task'}),/stale/);f.windows[0].destroy();await result;});
test('empty and oversized edits remain unapproved',async()=>{const f=fixture();const result=f.handlers['pet:task:confirm'](f.event,input);for(const text of [' ','x'.repeat(8001)])assert.throws(()=>f.handlers['pet:task:answer'](f.dialogEvent(),{requestId:'request-one',action:'confirm',text}),/8000/);assert(!f.windows[0].destroyed);f.windows[0].destroy();await result;});
test('workspace cannot be spoofed or changed while awaiting confirmation',async()=>{const f=fixture();await assert.rejects(f.handlers['pet:task:confirm'](f.event,{...input,workspace:'other'}),/项目/);const result=f.handlers['pet:task:confirm'](f.event,input);f.setWorkspace('D:/other');assert.throws(()=>f.handlers['pet:task:answer'](f.dialogEvent(),{requestId:'request-one',action:'confirm',text:'task'}),/项目/);assert.equal(await result,null);});
test('only one confirmation window can be open',async()=>{const f=fixture();const first=f.handlers['pet:task:confirm'](f.event,input);await assert.rejects(f.handlers['pet:task:confirm'](f.event,input),/等待确认/);assert.equal(f.windows.length,1);f.windows[0].destroy();await first;});
