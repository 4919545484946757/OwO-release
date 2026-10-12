"use strict";
const $=id=>document.getElementById(id),task=$('task');let input=null,submitting=false;
function count(){const valid=!!task.value.trim()&&task.value.length<=8000;$('count').textContent=task.value.length+' / 8000';$('confirm').disabled=!valid||submitting;}
task.addEventListener('input',count);
async function answer(action){if(!input||submitting)return;submitting=true;count();$('cancel').disabled=true;
try{await window.taskConfirmation.answer({requestId:input.requestId,action,text:task.value});}
catch(e){$('error').hidden=false;$('error').textContent=e.message||'确认未提交，请重试';submitting=false;$('cancel').disabled=false;count();}}
$('confirm').addEventListener('click',()=>answer('confirm'));$('cancel').addEventListener('click',()=>answer('cancel'));
window.addEventListener('keydown',event=>{if(event.key==='Escape')answer('cancel');});
window.taskConfirmation.load().then(value=>{input=value;task.value=value.text;$('previous').hidden=!value.previousInterrupted;$('workspace').textContent=value.workspace;$('output').textContent=value.output;for(const role of value.roles){const e=document.createElement('span');e.className='role';e.textContent=role.name+' · '+role.model;$('roles').appendChild(e);}count();task.focus();}).catch(e=>{$('error').hidden=false;$('error').textContent=e.message;});
