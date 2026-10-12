"use strict";
const {BrowserWindow,ipcMain,screen}=require('electron');
const path=require('node:path');
const {pathToFileURL}=require('node:url');
const {randomUUID}=require('node:crypto');
function installTaskConfirmation({pet,pets,workspace,port}) {
  const page=path.join(__dirname,'pet-task-confirmation.html'),pageUrl=pathToFileURL(page).href;
  let current=null;
  function text(value){if(typeof value!=='string'||!value.trim()||value.length>8000)throw Error('需求应为 1–8000 字');return value.trim();}
  function trustedPet(event){
    const w=pet();if(!w||w.isDestroyed()||event.sender!==w.webContents||event.senderFrame!==w.webContents.mainFrame)throw Error('untrusted confirmation sender');
    const u=new URL(event.senderFrame.url);if(u.protocol!=='http:'||u.hostname!=='127.0.0.1'||Number(u.port)!==port()||!/^\/pet\/?$/.test(u.pathname))throw Error('untrusted confirmation origin');
  }
  function trustedDialog(event){
    const c=current;if(!c||c.window.isDestroyed()||event.sender!==c.window.webContents||event.senderFrame!==c.window.webContents.mainFrame||event.senderFrame.url!==pageUrl)throw Error('untrusted confirmation dialog');return c;
  }
  function finish(result){const c=current;if(!c)return;current=null;c.resolve(result);if(!c.window.isDestroyed())c.window.destroy();}
  ipcMain.handle('pet:task:confirm',async(event,input)=>{
    trustedPet(event);if(current)throw Error('已有需求等待确认');
    const task=text(input?.text),project=workspace();if(typeof project!=='string'||!project)throw Error('请先选择项目目录');
    if(input.workspace!==project)throw Error('项目已改变，请重新投喂并确认');
    if(!Array.isArray(input.roles))throw Error('队伍信息无效');
    const roles=input.roles.slice(0,8).map(r=>({name:String(r.name||'Agent').slice(0,96),model:String(r.model||'默认模型').slice(0,128)}));
    const area=screen.getPrimaryDisplay().workArea,width=Math.min(740,area.width-32),height=Math.min(720,area.height-32);
    const parent=pets().find(w=>w&&!w.isDestroyed()&&w.isVisible())||pet();
    const w=new BrowserWindow({parent,modal:true,width,height,x:Math.round(area.x+(area.width-width)/2),y:Math.round(area.y+(area.height-height)/2),show:false,resizable:true,minWidth:420,minHeight:480,title:'确认交给队伍的任务',backgroundColor:'#101824',webPreferences:{preload:path.join(__dirname,'pet-task-confirmation-preload.js'),contextIsolation:true,nodeIntegration:false,sandbox:true}});
    w.webContents.setWindowOpenHandler(()=>({action:'deny'}));w.webContents.on('will-navigate',e=>e.preventDefault());
    const result=new Promise(resolve=>{current={window:w,resolve,input:{requestId:randomUUID(),text:task,workspace:project,previousInterrupted:input.previousInterrupted===true,output:path.join(project,'outputs'),roles}};});
    w.once('closed',()=>{if(current?.window===w)finish(null);});
    w.once('ready-to-show',()=>{if(current?.window===w){w.show();w.focus();}});
    try{await w.loadFile(page);}catch(error){const closed=w.isDestroyed();finish(null);if(!closed)throw error;}
    return result;
  });
  ipcMain.handle('pet:task:input',event=>trustedDialog(event).input);
  ipcMain.handle('pet:task:answer',(event,value)=>{
    const c=trustedDialog(event);if(value?.requestId!==c.input.requestId)throw Error('stale confirmation');
    if(value.action==='cancel'){finish(null);return {ok:true};}
    if(value.action!=='confirm')throw Error('invalid confirmation action');
    const task=text(value.text);if(workspace()!==c.input.workspace){finish(null);throw Error('项目已改变，请重新确认');}
    finish({text:task,workspace:c.input.workspace});return {ok:true};
  });
}
module.exports={installTaskConfirmation};
