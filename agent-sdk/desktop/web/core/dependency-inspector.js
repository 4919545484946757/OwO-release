// Read-only dependency evidence: handoff declarations, input receipts and Artifact metadata.
// This view never labels a declaration as a receipt from the downstream model.
(function(root) {
  "use strict";
  function esc(v) { return String(v == null ? "" : v).replace(/[&<>"']/g,function(c){return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c];}); }
  function bytes(value) { return new TextEncoder().encode(JSON.stringify(value)).length; }
  function related(record,run,source,target) {
    var prefix=run+":"+source.task_id+":",id=String(record.handoff_id||"");
    var suffix=id.indexOf(prefix)===0?id.slice(prefix.length):"";
    return /^(v[1-9][0-9]*|manual:[0-9]+)$/.test(suffix) &&
      record.from_member===source.worker && (record.to_member===target.worker || record.to_member==="*");
  }
  function create(options) {
    var api=options.api,box=options.box,run=String(options.runId||""),disposed=false,generation=0,request=null,mediaRequest=null,timer=null,pendingNotice=false;
    var inputClient=root.OwoDependencyInputs;
    if(!inputClient&&typeof require==="function")inputClient=require("./dependency-inputs.js");
    var inputs=inputClient?inputClient.create({api:api,runId:run,paint:paint,inspect:options.inspectInput}):null;
    var state={source:null,target:null,records:[],before:null,nextBefore:null,more:false,busy:false,error:"",artifact:null,artifactBusy:false,newRecords:false};
    function html() {
      if (!state.source) return '<p class="hint">选择任务图中的依赖连线，查看该边的交接声明与实际产物引用。</p>';
      var head='<h4>'+esc(state.source.role||state.source.task_id)+' → '+esc(state.target.role||state.target.task_id)+'</h4>'+
        '<p class="hint">上游 '+esc(state.source.task_id)+'（'+esc(state.source.status)+'） · 下游 '+esc(state.target.task_id)+'（'+esc(state.target.status)+'）</p>'+
        '<p class="hint">以下是已保存的交接声明，不表示下游模型已收到或消费；Worker 输入准备记录可单独查看。旧记录无法可靠定位到任务时不会按角色名猜测。</p>'+
        '<div class="owo-ws-inline"><button type="button" data-edge-action="older"'+(state.busy||!state.more||!state.nextBefore?' disabled':'')+'>更早交接页</button>'+
        '<button type="button" data-edge-action="latest"'+(state.busy?' disabled':'')+'>最新交接页</button>'+
        '<span role="status" class="hint">'+(state.busy?'正在读取…':state.before?'正在查看历史页':'最新交接页')+
        (state.newRecords?' · 有新记录，可返回最新':'')+'</span></div>'+
        (state.error?'<p role="alert">'+esc(state.error)+'</p>':'');
      var records=state.records.map(function(h) {
        var refs=Array.isArray(h.output_artifact_refs)?h.output_artifact_refs:[];
        return '<details class="owo-ws-tool-detail" data-edge-handoff="'+esc(h.handoff_id)+'"><summary>'+esc(h.handoff_id)+' · '+esc(h.from_member)+' → '+esc(h.to_member)+'</summary>'+
          '<p>'+esc(h.completed_summary)+'</p><p class="hint">保存时间 '+esc(h.created_at)+' · '+(h.to_member==="*"?'接收方未指定':'接收方为本下游成员')+'</p>'+
          '<div class="owo-ws-inline">'+refs.slice(0,8).map(function(id){return '<button type="button" data-edge-artifact="'+esc(id)+'"'+(state.artifactBusy?' disabled':'')+'>产物 '+esc(id)+'</button>';}).join("")+'</div>'+
          (refs.length>8?'<p class="hint">本记录还有 '+(refs.length-8)+' 个引用，完整 ID 见原始交接。</p>':'')+
          '<pre style="white-space:pre-wrap;overflow-wrap:anywhere">'+esc(JSON.stringify(h,null,2))+'</pre></details>';
      }).join("");
      var artifact=state.artifactBusy?'<p role="status">正在读取产物元数据…</p>':state.artifact?
        '<section aria-label="选中产物元数据"><h4>产物 '+esc(state.artifact.artifact_id)+'</h4>'+
        '<p class="hint">任务 '+esc(state.artifact.task_id||"未记录")+' · 尝试 '+esc(state.artifact.attempt_id||"未记录")+
        ' · 格式 '+esc(state.artifact.format||state.artifact.media_type||"未记录")+' · 版本 '+esc(state.artifact.version)+
        ' · 字节 '+esc(state.artifact.size_bytes)+' · SHA-256 '+esc(state.artifact.sha256||"未记录")+'</p>'+
        '<pre style="white-space:pre-wrap;overflow-wrap:anywhere">'+esc(JSON.stringify(state.artifact,null,2))+'</pre></section>':"";
      return head+(inputs?inputs.html():"")+'<div role="log" aria-label="依赖边交接声明" style="max-height:440px;overflow:auto">'+records+
        (!state.busy&&!state.records.length?'<p class="hint">本页未找到可定位到这条边的交接声明；可查看更早页。此结果不能证明没有传递输入。</p>':'')+'</div>'+artifact;
    }
    function paint() {
      if(disposed||!box)return;
      var details=new Map(),scroll=new Map();
      box.querySelectorAll("details[data-edge-handoff],details[data-edge-input-event],details[data-provider-call]").forEach(function(el){
        details.set(el.getAttribute("data-edge-handoff")||el.getAttribute("data-edge-input-event")||el.getAttribute("data-provider-call"),el.open);
      });
      box.querySelectorAll('[role="log"]').forEach(function(log){scroll.set(log.getAttribute("aria-label"),log.scrollTop);});
      if(root.OwoMediaResult)root.OwoMediaResult.renderInto(box,html());else box.innerHTML=html();
      box.querySelectorAll("details[data-edge-handoff],details[data-edge-input-event],details[data-provider-call]").forEach(function(el){
        var key=el.getAttribute("data-edge-handoff")||el.getAttribute("data-edge-input-event")||el.getAttribute("data-provider-call");
        if(details.has(key))el.open=details.get(key);
      });
      box.querySelectorAll('[role="log"]').forEach(function(log){if(scroll.has(log.getAttribute("aria-label")))log.scrollTop=scroll.get(log.getAttribute("aria-label"));});
    }
    async function read(before) {
      if(disposed||!state.source)return;
      if(timer!=null)clearTimeout(timer);timer=null;
      if(request)request.abort();if(mediaRequest)mediaRequest.abort();mediaRequest=null;
      var ctrl=new AbortController(),ticket=++generation;request=ctrl;
      pendingNotice=false;
      state.busy=true;state.artifactBusy=false;state.artifact=null;state.error="";paint();
      try {
        var cursors=options.cursors?options.cursors():{};
        var path="/teams/"+encodeURIComponent(run)+"/conversation?cursors="+encodeURIComponent(JSON.stringify(cursors));
        if(before!=null)path+="&handoff_before="+encodeURIComponent(before);
        var page=await api.get(path,{signal:ctrl.signal});
        if(disposed||ticket!==generation)return;
        if(!page||page.team_id!==run||!Array.isArray(page.handoffs)||page.handoffs.length>64||bytes(page.handoffs)>1024*1024||
          typeof page.handoffs_has_more!=="boolean"||page.next_handoff_before!==(page.handoffs.length?page.handoffs[0].handoff_id:null))
          throw new Error("交接页协议或大小不符合约定；未替换已读数据");
        if(page.handoffs.some(function(h){return !h||typeof h.handoff_id!=="string"||typeof h.from_member!=="string"||typeof h.to_member!=="string";}))
          throw new Error("交接页身份无效");
        if(!page.handoffs.length&&before!=null) {state.more=false;return;}
        state.records=page.handoffs.filter(function(h){return related(h,run,state.source,state.target);});
        state.before=before;state.nextBefore=page.next_handoff_before;state.more=page.handoffs_has_more;state.newRecords=pendingNotice;
      } catch(error) {if(!disposed&&ticket===generation&&!ctrl.signal.aborted)state.error=String(error.message||error);}
      finally {if(!disposed&&ticket===generation){
        request=null;state.busy=false;paint();
        if(pendingNotice&&state.before==null&&!state.error&&timer==null)timer=setTimeout(function(){timer=null;read(null);},250);
      }}
    }
    async function inspectArtifact(id) {
      if(disposed||!state.records.some(function(h){return (h.output_artifact_refs||[]).indexOf(id)>=0;}))return;
      if(mediaRequest)mediaRequest.abort();
      var ctrl=new AbortController(),ticket=generation;mediaRequest=ctrl;state.artifactBusy=true;state.error="";paint();
      try {
        var metadata=await api.get("/artifacts/"+encodeURIComponent(id)+"/metadata",{signal:ctrl.signal});
        if(disposed||ticket!==generation||mediaRequest!==ctrl)return;
        if(!metadata||metadata.artifact_id!==id||metadata.team_id!==run||
          (metadata.task_id&&metadata.task_id!==state.source.task_id)||bytes(metadata)>65536)
          throw new Error("产物身份、来源或大小不符；未展示为本边产物");
        state.artifact=metadata;
      } catch(error) {if(!disposed&&ticket===generation&&mediaRequest===ctrl&&!ctrl.signal.aborted){state.artifact=null;state.error=String(error.message||error);}}
      finally {if(!disposed&&ticket===generation&&mediaRequest===ctrl){mediaRequest=null;state.artifactBusy=false;paint();}}
    }
    function select(source,target) {
      if(!source||!target||!source.task_id||!source.worker||!target.worker||!(target.depends_on||[]).includes(source.task_id))return Promise.resolve();
      if(inputs)inputs.reset(source,target);
      state.source=source;state.target=target;state.records=[];state.before=null;state.nextBefore=null;state.more=false;
      return read(null);
    }
    function refreshTasks(tasks) {
      if(disposed||!state.source)return;
      var source=tasks.find(function(t){return t.task_id===state.source.task_id;});
      var target=tasks.find(function(t){return t.task_id===state.target.task_id;});
      if(!source||!target||source.worker!==state.source.worker||target.worker!==state.target.worker||!(target.depends_on||[]).includes(source.task_id)) {
        if(inputs)inputs.reset();
        generation++;if(request)request.abort();if(mediaRequest)mediaRequest.abort();if(timer!=null)clearTimeout(timer);
        request=null;mediaRequest=null;timer=null;state.source=null;state.target=null;state.records=[];state.artifact=null;state.busy=false;state.artifactBusy=false;
      } else {state.source=source;state.target=target;}
      paint();
    }
    function navigate(action) {if(action==="latest")return read(null);if(action==="older"&&state.more&&state.nextBefore)return read(state.nextBefore);return Promise.resolve();}
    function notice(event) {
      if(inputs)inputs.notice(event);
      if(disposed||!state.source||!/^(handoff|artifact)\./.test(String(event.event_type||"")))return;
      state.newRecords=true;if(state.busy)pendingNotice=true;
      if(state.before==null&&!state.busy&&timer==null)timer=setTimeout(function(){timer=null;read(null);},250);
      paint();
    }
    function click(event) {
      var button=event.target.closest("button");if(!button||!box.contains(button)||button.disabled)return;
      if(button.hasAttribute("data-edge-input-action")){if(inputs)inputs.navigate(button.getAttribute("data-edge-input-action"),button.getAttribute("data-edge-input-id"));}
      else if(button.hasAttribute("data-edge-artifact"))inspectArtifact(button.getAttribute("data-edge-artifact"));
      else if(button.hasAttribute("data-edge-action"))navigate(button.getAttribute("data-edge-action"));
    }
    if(box)box.addEventListener("click",click);
    paint();
    return {select:select,refreshTasks:refreshTasks,navigate:navigate,inspectArtifact:inspectArtifact,notice:notice,state:state,html:html,inputs:inputs,dispose:function(){
      if(inputs)inputs.dispose();
      disposed=true;generation++;if(request)request.abort();if(mediaRequest)mediaRequest.abort();if(timer!=null)clearTimeout(timer);
      request=null;mediaRequest=null;timer=null;state.records=[];state.artifact=null;if(box){box.removeEventListener("click",click);box.innerHTML='<p class="hint">观察已停止；重新选择依赖连线可继续检查。</p>';}
    }};
  }
  var api={create:create,related:related};root.OwoDependencyInspector=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
