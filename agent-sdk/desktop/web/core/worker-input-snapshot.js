// Read-only exact input recovery from the canonical Worker session checkpoint.
// A Provider receipt is a digest witness, never proof of remote model delivery.
(function(root){
 "use strict";
 var H=root.OwoRunHistory,I=root.OwoHistoryInput;
 if(typeof require==="function"){H=H||require("./run-history.js");I=I||require("./history-input.js");require("./attachment-upload.js");}
 var MAX_BYTES=4*1024*1024;
 function esc(v){return String(v==null?"":v).replace(/[&<>"']/g,function(c){return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c];});}
 function utf8(v){return new TextEncoder().encode(v);}
 // Match serde's StoredUserInput/ContentPart/AssetRef field order and omitted empty vectors.
 function encoded(message){
  var record=I.read(message),input={prompt:record.prompt};
  if(record.content_parts.length)input.content_parts=record.content_parts;
  if(record.legacy_attachments.length){
   if(record.legacy_attachments.some(function(v){return typeof v!=="string";}))throw new Error("旧附件记录无效");
   input.legacy_attachments=record.legacy_attachments;
  }
  return JSON.stringify(input);
 }
 async function digest(value){
  if(!root.crypto||!root.crypto.subtle)throw new Error("当前环境不能校验输入快照摘要");
  var buffer=await root.crypto.subtle.digest("SHA-256",value);
  return Array.from(new Uint8Array(buffer),function(v){return v.toString(16).padStart(2,"0");}).join("");
 }
 function canInspect(event,run){
  try{
   var d=event&&event.data||{},summary=H.describeModel(event||{}),sid=d.worker_session_id;
   if(!event||event.run_id!==run||event.schema_version!==1||!Number.isSafeInteger(event.seq)||event.seq<=0||
      event.event_id!=="run:"+run+":"+event.seq||event.event_type!=="model.input.prepared"||!summary||!summary.valid||!d.last_user_input||
      typeof sid!=="string"||!sid.startsWith(run+"-worker-")||sid.length>384||/[\u0000-\u001f\u007f]/.test(sid)||
      utf8(JSON.stringify(event)).length>65536)return false;
   return true;
  }catch(_){return false;}
 }
 function create(options){
  var generation=0,request=null,disposed=false;
  var actionAttr=options.surface==="history"?"data-history-action":"data-edge-input-action";
  var actionPrefix=options.surface==="history"?"snapshot-":"provider-snapshot-";
  var state={receipt:null,message:null,rows:[],busy:false,error:"",metadataError:""};
  function paint(){if(!disposed&&options.paint)options.paint();}
  function stop(){generation++;if(request)request.abort();request=null;}
  function close(){stop();Object.assign(state,{receipt:null,message:null,rows:[],busy:false,error:"",metadataError:""});paint();}
  async function json(path,ctrl){
   // Use the existing authenticated streaming size gate, before JSON parsing.
   if(!options.api||typeof options.api.download!=="function")throw new Error("受限历史读取服务不可用");
   var blob=await options.api.download(path,{signal:ctrl.signal,maxResponseBytes:MAX_BYTES});
   if(!blob||blob.size>MAX_BYTES)throw new Error("输入快照响应超过 4 MiB");
   if(!/^application\/json(?:;|$)/i.test(blob.type||""))throw new Error("输入快照响应格式无效");
   return JSON.parse(await blob.text());
  }
  async function open(receipt){
   if(disposed)return;
   stop();var ticket=generation,ctrl=new AbortController();request=ctrl;
   Object.assign(state,{receipt:null,message:null,rows:[],busy:false,error:"",metadataError:""});
   if(!canInspect(receipt,options.runId)){state.error="模型请求缺少可核对的 Worker 输入身份";request=null;paint();return;}
   state.receipt=JSON.parse(JSON.stringify(receipt));state.busy=true;paint();
   function current(){return !disposed&&ticket===generation&&!ctrl.signal.aborted;}
   try{
    var expected=state.receipt.data.last_user_input,sid=state.receipt.data.worker_session_id;
    var session=await json("/session/"+encodeURIComponent(sid),ctrl);if(!current())return;
    if(!session||session.id!==sid||!Array.isArray(session.messages)||session.messages.length>4096)throw new Error("Worker 会话身份或历史大小无效");
    var match=null;
    // A compacted/deleted checkpoint may no longer contain this exact historical input.
    // Never substitute the most recent user message, image cache, or parent input.
    for(var message of session.messages){
     if(message.role!=="user"||!message.input)continue;
     var text;try{text=encoded(message);}catch(_){continue;}
     var value=utf8(text);
     if(value.length!==expected.identity.bytes)continue;
     var hash=await (options.digest||digest)(value);if(!current())return;
     if(hash===expected.identity.sha256){match={role:"user",content:"",input:JSON.parse(text)};break;}
    }
    if(!match)throw new Error("会话检查点未找到摘要完全一致的输入；可能尚未持久化、已压缩或已删除，未用最新输入替代");
    // Verify the human-readable receipt fields as well as its complete identity.
    var record=I.read(match),prompt=utf8(JSON.stringify(record.prompt));
    if(record.content_parts.length!==expected.part_count||record.legacy_attachments.length!==expected.legacy_attachment_count||
       prompt.length!==expected.prompt.bytes||await (options.digest||digest)(prompt)!==expected.prompt.sha256)throw new Error("输入摘要与回执计数或原文摘要不符");
    if(!current())return;
    state.message=match;
    if(record.content_parts.some(function(p){return p.type!=="text";})){
     try{
      var rows=await json("/session/"+encodeURIComponent(sid)+"/attachments",ctrl);if(!current())return;
      var verified=I.rowsFor(record,rows);
      if(verified.some(function(row){return row.content_part.type==="image"&&(!["image/png","image/jpeg","image/webp","image/gif"].includes(row.mime)||row.size===0);}))throw new Error("图片元数据的格式或大小无效");
      state.rows=verified;
     }catch(e){if(current())state.metadataError=String(e.message||e);}
    }
   }catch(e){if(current())state.error=String(e.message||e);}
   finally{if(current()){request=null;state.busy=false;paint();}}
  }
  function html(){
   if(!state.receipt)return state.error?'<p role="alert">'+esc(state.error)+'</p>':"";
   var record=state.message&&I.read(state.message),d=state.receipt.data;
   var body=record?'<pre style="white-space:pre-wrap;overflow-wrap:anywhere">'+esc(record.prompt)+'</pre>':"";
   if(record){
    if(record.legacy_attachments.length)body+='<p class="hint">旧附件缺少不可变引用；不展示缓存图片或重发，需重新上传。</p>';
    record.content_parts.forEach(function(part,index){
     if(part.type==="text"){body+='<pre style="white-space:pre-wrap;overflow-wrap:anywhere">'+esc(part.text)+'</pre>';return;}
     var row=state.rows[index];
     body+='<p>'+esc(row&&row.name||part.asset.artifact_id)+' · SHA-256 '+esc(part.asset.sha256)+'</p>';
     if(!row){body+='<p class="hint">附件元数据未核对，未加载内容。</p>';return;}
     if(part.type==="image"){
      var payload={tool:"generate_image",ok:true,outputs:[{kind:"image",artifact_id:part.asset.artifact_id,path:row.name||row.id,mime_type:row.mime,bytes:row.size,sha256:part.asset.sha256}]};
      body+='<div data-media-artifact="'+esc(JSON.stringify(payload))+'"></div>';
     }else body+='<p class="hint">文件引用已核对；本视图不将文件内容冒充模型图片输入。</p>';
    });
   }
   return '<section aria-label="Worker 输入快照" style="overflow-wrap:anywhere"><h4>Worker 输入快照</h4>'+
    '<p class="hint">精确匹配 '+esc(state.receipt.event_id)+' 的保存输入；不等同于最初建队原话，不证明最终模型接收或理解。仅按需鉴权读取，不重发、不回退任务。</p>'+
    '<p class="hint">会话 '+esc(d.worker_session_id)+' · 输入 SHA-256 '+esc(d.last_user_input.identity.sha256)+'</p>'+
    '<div class="owo-ws-inline"><button type="button" '+actionAttr+'="'+actionPrefix+'retry"'+(state.busy?' disabled':'')+'>重新读取输入快照</button>'+
    '<button type="button" '+actionAttr+'="'+actionPrefix+'close">收起输入快照</button></div>'+
    (state.busy?'<p role="status">正在核对输入检查点…</p>':"")+
    (state.error?'<p role="alert">'+esc(state.error)+'</p>':"")+body+
    (state.metadataError?'<p role="alert">附件目录不可用：'+esc(state.metadataError)+'；输入原文仍可读取。</p>':"")+'</section>';
  }
  return {state:state,open:open,close:close,html:html,dispose:function(){disposed=true;close();}};
 }
 var api={create:create,encoded:encoded,canInspect:canInspect};root.OwoWorkerInputSnapshot=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
