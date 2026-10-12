"use strict";
const {app,ipcMain,clipboard,globalShortcut}=require('electron');
const fs=require('node:fs'),path=require('node:path');
function installPetFeed({pet,main,workspace,dataRoot,port,pets=()=>[pet()],ensureAgents=()=>{},moveActor,resetActor,setVisible,present=()=>{},queryVisible}) {
  const statePath=()=>path.join(dataRoot(),'pet-feed-run.json');
  function trusted(event,window) {
    if(!window||window.isDestroyed()||event.sender!==window.webContents||event.senderFrame!==window.webContents.mainFrame)throw Error('untrusted pet feed sender');
    const url=new URL(event.senderFrame.url);
    if(url.protocol!=='http:'||url.hostname!=='127.0.0.1'||Number(url.port)!==port()||(pets().includes(window)&&!/^\/pet\/?$/.test(url.pathname)))throw Error('untrusted origin');
  }
  function actor(event){const window=pets().find(w=>w&&!w.isDestroyed()&&w.webContents===event.sender);trusted(event,window);return window;}
  const id=value=>typeof value==='string'&&/^[a-zA-Z0-9_.-]{1,128}$/.test(value);
  const roleId=value=>typeof value==='string'&&!!value.trim()&&value.length<=128&&!['__proto__','constructor','prototype'].includes(value);
  const valid=ref=>ref && (id(ref.id) || ref.pending===true&&id(ref.goalId));
  const read=()=>{try{const ref=JSON.parse(fs.readFileSync(statePath(),'utf8'));return valid(ref)?ref:null;}catch{return null;}};
  function clean(ref){
    if(!valid(ref))throw Error('invalid run');
    if(ref.pending)return {pending:true,goalId:ref.goalId};
    return {id:ref.id,projectId:String(ref.projectId||''),model:String(ref.model||''),roleModels:Object.fromEntries(Object.entries(ref.roleModels||{}).filter(([role,model])=>roleId(role)&&typeof model==='string').map(([role,model])=>[role,model.slice(0,128)])),members:(ref.members||[]).slice(0,32).filter(m=>roleId(m.role)&&roleId(m.member_id)).map(m=>({role:m.role,member_id:m.member_id,runtime_binding:m.runtime_binding,capabilities:(m.capabilities||[]).filter(cap=>typeof cap==='string'&&cap.length<=64)}))};
  }
  function persist(ref){const file=statePath();fs.mkdirSync(dataRoot(),{recursive:true});fs.writeFileSync(file+'.tmp',JSON.stringify(ref),'utf8');fs.renameSync(file+'.tmp',file);}
  function broadcast(ref){for(const w of pets())if(w&&w!==pet()&&!w.isDestroyed())w.webContents.send('pet:feed:run',ref);}
  ipcMain.handle('pet:feed:read',event=>{actor(event);return clipboard.readText();});
  ipcMain.handle('pet:feed:save',(event,value)=>{trusted(event,pet());const ref=value===null?null:clean(value);persist(ref);ensureAgents(ref);broadcast(ref);});
  ipcMain.handle('pet:feed:load',event=>{const w=actor(event),ref=read();if(w===pet()&&ref?.id)ensureAgents(ref);return ref;});
  ipcMain.handle('pet:feed:activity',(event,value)=>{
    trusted(event,pet());
    if(!value||!id(value.id)||!Array.isArray(value.activeRoles)||value.activeRoles.length>32||!value.activeRoles.every(roleId)||typeof value.status!=='string'||value.status.length>32)throw Error('invalid activity');
    if(read()?.id!==value.id)return {ok:false};
    present({id:value.id,status:value.status,activeRoles:value.activeRoles});return {ok:true};
  });
  ipcMain.handle('pet:feed:forward' ,(event,text)=>{actor(event);if(typeof text!=='string'||!text.trim()||text.length>8000)throw Error('invalid feed');const w=pet();if(w&&!w.isDestroyed()){if(setVisible)Promise.resolve(setVisible(true)).catch(()=>{});w.webContents.send('pet:feed:text',text);}});
  ipcMain.handle('pet:actor:query',event=>{const w=actor(event);return {alive:true,visible:w===pet()&&queryVisible?queryVisible():w.isVisible()};});
  ipcMain.handle('pet:actor:visible',(event,visible)=>{actor(event);if(typeof visible!=='boolean')throw Error('invalid visibility');return setVisible?setVisible(visible):{ok:false};});
  ipcMain.handle('pet:actor:move',(event,dx,dy)=>{const w=actor(event);if(!Number.isFinite(dx)||!Number.isFinite(dy))throw Error('invalid movement');if(moveActor)return moveActor(w,dx,dy);const [x,y]=w.getPosition();w.setPosition(x+Math.max(-500,Math.min(500,dx)),y+Math.max(-500,Math.min(500,dy)));return {ok:true};});
  ipcMain.handle('pet:actor:reset',event=>{const w=actor(event);return resetActor?resetActor(w):{ok:false};});
  let wanted=null;
  function open(ref){wanted=ref;workspace();if(main()&&!main().isDestroyed())main().webContents.send('pet:feed:open-run',wanted);}
  ipcMain.handle('pet:feed:open',(event,ref)=>{actor(event);if(ref&&!valid(ref))throw Error('invalid run');open(ref);});
  ipcMain.handle('pet:feed:approvals',event=>{actor(event);open({panel:'permissions'});});
  ipcMain.handle('pet:feed:pending',event=>{trusted(event,main());return wanted;});
  app.whenReady().then(()=>{
    const ok=globalShortcut.register('Control+Alt+F',()=>{const w=pet();if(w&&!w.isDestroyed()){if(setVisible)Promise.resolve(setVisible(true)).catch(()=>{});w.webContents.send('pet:feed:text',clipboard.readText());}});
    if(!ok)console.warn('[pet-feed] Shortcut unavailable; use right-click feeding.');
  });
}
module.exports={installPetFeed};
