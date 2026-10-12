const {contextBridge,ipcRenderer}=require('electron');
contextBridge.exposeInMainWorld('taskConfirmation',{
  load:()=>ipcRenderer.invoke('pet:task:input'),
  answer:value=>ipcRenderer.invoke('pet:task:answer',value),
});
