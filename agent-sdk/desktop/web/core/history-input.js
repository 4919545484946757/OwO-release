// Stored input replay uses immutable refs; the host remains the custody authority.
(function(root){
 "use strict";
 const MAX_IMAGE=5*1024*1024,MAX_FILE=24*1024*1024;
 function error(text){throw new Error(text);}
 function textPart(part){if(!part||part.type!=="text"||typeof part.text!=="string"||new TextEncoder().encode(part.text).length>32768)error("历史附加文本格式或大小无效");return {type:"text",text:part.text};}
 function read(message){
  message=message||{};
  if(!message.input)return {prompt:String(message.content||""),content_parts:[],legacy_attachments:(message.images||[]).length||/\.owo-attachments|用户上传的附件/.test(message.content||"")?["legacy-history"]:[]};
  const input=message.input;
  if(typeof input.prompt!=="string"||input.content_parts!=null&&!Array.isArray(input.content_parts)||input.legacy_attachments!=null&&!Array.isArray(input.legacy_attachments))error("历史输入记录无效，未回退或重发");
  const parts=(input.content_parts||[]).map((part,index)=>{
   if(part&&part.type==="text")return textPart(part);
   if(!part||!part.asset)error("历史附件引用无效");
   return root.OwoAttachmentUpload.turnInput([{id:part.asset.artifact_id,content_part:part}]).content_parts[0];
  });
  if(parts.length>32||parts.filter(p=>p.type==="image").length>8)error("历史输入超过单次内容块或图片上限");
  return {prompt:input.prompt,content_parts:parts,legacy_attachments:(input.legacy_attachments||[]).slice()};
 }
 function displayText(message){try{return message.role==="user"?read(message).prompt:String(message.content||"");}catch(_){return String(message.content||"");}}
 function rowsFor(record,rows){
  if(!Array.isArray(rows)||rows.length>4096||new TextEncoder().encode(JSON.stringify(rows)).length>4*1024*1024)error("历史附件目录无效或过大，请整理后重试");
  const byId=new Map();for(const row of rows){if(!row||typeof row.id!=="string"||byId.has(row.id))error("历史附件目录包含无效或重复引用");byId.set(row.id,row);}
  return record.content_parts.map((part,index)=>{
   if(part.type==="text")return {id:"@text:"+index,name:"附加文本 "+(index+1),mime:"text/plain",content_part:textPart(part)}; // Local chip key only; never an AssetRef or legacy attachment ID.
   const row=byId.get(part.asset.artifact_id);
   if(!row||!row.content_part||row.content_part.type!==part.type||!row.content_part.asset||row.content_part.asset.sha256!==part.asset.sha256||row.content_part.asset.artifact_id!==part.asset.artifact_id)error("历史附件已不可用或版本不符，未回退或重发");
   if(!Number.isSafeInteger(row.size)||row.size<0||row.size>(part.type==="image"?MAX_IMAGE:MAX_FILE))error("历史附件大小无效");
   return {...row,content_part:{type:part.type,asset:{...part.asset}}};
  });
 }
 async function digest(blob){if(!root.crypto||!root.crypto.subtle)error("当前页面不支持附件完整性校验，未回退或重发");const hash=await root.crypto.subtle.digest("SHA-256",await blob.arrayBuffer());return Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,"0")).join("");}
 async function prepare(message,options){
  const record=read(message),current=options.isCurrent||(()=>true);function check(){if(!current())error("会话已切换，未回退或重发");}check();
  if(record.legacy_attachments.length)error("这条历史含旧附件，缺少不可变引用；请重新上传后发送，未回退对话");
  if(!record.content_parts.length)return {record,attachments:[]};
  const capabilities=options.capabilities||await options.get("/capabilities");check();if(capabilities?.constraints?.structured_input_parts!==true)error("当前核心不支持历史附件重发，未回退对话");
  const assets=record.content_parts.some(p=>p.type!=="text");
  const rows=assets?await options.get("/session/"+encodeURIComponent(options.sessionId)+"/attachments"):[];check();
  const attachments=rowsFor(record,rows);
  if(attachments.reduce((sum,r)=>sum+(r.content_part.type==="image"?r.size:0),0)>20*1024*1024)error("历史图片合计超过 20 MiB，未回退对话");
  for(const row of attachments){
   if(row.content_part.type==="text")continue;
   check();const blob=await options.download("/artifacts/"+encodeURIComponent(row.id)+"/content?raw=true",{maxResponseBytes:row.size});check();
   if(blob.size!==row.size||await (options.digest||digest)(blob)!==row.content_part.asset.sha256)error("历史附件内容校验失败，未回退对话");check();
  }
  root.OwoAttachmentUpload.turnInput(attachments);
  return {record,attachments};
 }
 function render(node,message,rows){
  if(!node||!node.ownerDocument)return;
  const doc=node.ownerDocument;function line(text){const p=doc.createElement("p");p.className="hint";p.textContent=text;node.appendChild(p);return p;}
  let record;try{record=read(message);}catch(e){line(e.message);return;}
  if(record.legacy_attachments.length){
   line("旧附件历史：编辑或重试前需重新上传附件。");let bytes=0,count=0;
   for(const image of message.images||[]){const url=String(image.url||""),match=/^data:image\/(png|jpeg|webp|gif);base64,([A-Za-z0-9+/]+={0,2})$/.exec(url);if(!match)continue;const size=match[2].length/4*3;bytes+=size;if(size>MAX_IMAGE||bytes>20*1024*1024||++count>8)break;const img=doc.createElement("img");img.alt="历史图片";img.className="owo-media-inline";img.style.maxWidth="100%";img.src=url;node.appendChild(img);}
   return;
  }
  for(const part of record.content_parts){
   if(part.type==="text"){line(part.text);continue;}
   let row;try{row=rowsFor({...record,content_parts:[part]},rows)[0];}catch(e){line(e.message);continue;}
   line(String(row.name||row.id)+" · 版本 "+part.asset.sha256.slice(0,12));
   if(part.type==="image"&&root.OwoMediaResult){const preview=doc.createElement("div");node.appendChild(preview);root.OwoMediaResult.mount(preview,{kind:"image",path:row.name||row.id,artifactId:row.id,mimeType:row.mime,bytes:row.size,contentPath:"/artifacts/"+encodeURIComponent(row.id)+"/content?raw=true"});}
  }
 }
 const api={read,displayText,prepare,render,rowsFor};root.OwoHistoryInput=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
