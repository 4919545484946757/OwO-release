// Read-only projection of host-prepared input receipts from the canonical run journal.
// One bounded page at a time; this is not evidence of final model delivery.
(function(root) {
  "use strict";
  var LIMIT=64, MAX_BYTES=1024*1024;
  function esc(value){return String(value==null?"":value).replace(/[&<>"']/g,function(c){return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c];});}
  function bytes(value){return new TextEncoder().encode(JSON.stringify(value)).length;}
  function count(value){return Number.isSafeInteger(value)&&value>=0;}
  function digest(value){return typeof value==="string"&&/^[a-f0-9]{64}$/.test(value);}
  function causeSeq(run,event){
    var prefix="run:"+run+":",id=String(event.causation_id||""),suffix=id.indexOf(prefix)===0?id.slice(prefix.length):"";
    var seq=Number(suffix);
    return /^[1-9][0-9]*$/.test(suffix)&&Number.isSafeInteger(seq)&&seq<event.seq?seq:null;
  }
  function related(event,source,target){
    var data=event.data||{},receiver=data.target||{};
    if(receiver.step_id!==target.task_id||receiver.worker_id!==target.worker)return false;
    if(event.event_type==="worker.input.prepared")return true;
    var from=data.source||{};
    return event.event_type==="worker.input.source"&&from.producer_step_id===source.task_id&&from.producer_worker_id===source.worker;
  }
  function validateReceipt(run,event){
    var data=event.data||{},target=data.target||{},source=data.source||{};
    if(typeof data.invocation_id!=="string"||!data.invocation_id||data.boundary!=="before_worker_run"||data.model_delivery!=="not_proven"||
      (event.task_id||null)!==(target.task_id||null)||(event.attempt||null)!==(target.attempt_id||null))
      throw new Error("输入记录的宿主边界或调用身份无效");
    if(event.event_type==="worker.input.prepared"){
      if(!digest(data.input_sha256)||!count(data.input_bytes)||!count(data.source_count)||
        data.input_representation!=="serde_json_worker_value"||
        ["structured_team_context","serialized_team_context","no_team_context_in_payload"].indexOf(data.context_encoding)<0)
        throw new Error("输入准备头的摘要或编码无效");
    }else if(typeof source.artifact_id!=="string"||!source.artifact_id||!Array.isArray(event.artifact_refs)||
      event.artifact_refs.length!==1||event.artifact_refs[0]!==source.artifact_id||!count(source.preview_bytes)||
      !digest(source.preview_sha256)||causeSeq(run,event)==null||
      (source.preview_truncated!=null&&typeof source.preview_truncated!=="boolean")){
      throw new Error("逐源输入记录的产物引用、摘要或前因无效");
    }
  }
  function validatePage(page,run,before){
    if(!page||page.run_id!==run||!Array.isArray(page.events)||page.events.length>LIMIT||bytes(page)>MAX_BYTES||
      typeof page.has_more!=="boolean"||typeof page.history_complete!=="boolean"||!count(page.high_watermark)||
      (page.task_history_complete!=null&&typeof page.task_history_complete!=="boolean")||
      page.next_before!==(page.events.length?page.events[0].seq:null)||(!page.events.length&&page.has_more))
      throw new Error("输入记录历史分页协议或大小无效");
    var previous=0;
    page.events.forEach(function(event){
      if(!event||event.run_id!==run||event.schema_version!==1||!Number.isSafeInteger(event.seq)||event.seq<=previous||
        event.seq>page.high_watermark||(before!==0&&event.seq>=before)||event.event_id!=="run:"+run+":"+event.seq||
        typeof event.event_type!=="string"||bytes(event)>65536)throw new Error("输入记录事件身份、顺序或边界无效");
      previous=event.seq;
    });
  }
  function create(options){
    var run=String(options.runId||""),source=null,target=null,disposed=false,request=null,generation=0,timer=null,pending=false;
    var state={open:false,events:[],busy:false,error:"",before:0,nextBefore:null,more:false,newRecords:false,historyComplete:null,taskHistoryComplete:null,focus:null};
    var callClient=root.OwoProviderCalls;
    if(!callClient&&typeof require==="function")callClient=require("./provider-call-inspector.js");
    var calls=callClient?callClient.create({runId:run,api:options.api,paint:paint}):null;
    function paint(){if(!disposed&&options.paint)options.paint();}
    function stopRead(){generation++;if(request)request.abort();request=null;if(timer!=null)clearTimeout(timer);timer=null;pending=false;}
    function reset(from,to){
      stopRead();if(calls)calls.close();source=from||null;target=to||null;
      Object.assign(state,{open:false,events:[],busy:false,error:"",before:0,nextBefore:null,more:false,newRecords:false,historyComplete:null,taskHistoryComplete:null,focus:null});
    }
    function html(){
      if(!source||!target)return "";
      if(!state.open)return '<button type="button" data-edge-input-action="open">查看本边输入准备记录</button>';
      var warning='<p class="hint">以下记录保存于调用 Worker 前，证明已准备的输入和来源引用；尚未证明最终模型接收或使用。准备头属于该下游调用，只有逐源记录才能定位本依赖边。</p>';
      var status=state.busy?'正在读取输入记录…':state.before?'历史输入页':'最新输入页';
      var head='<section aria-label="依赖边输入准备记录"><h4>输入准备记录</h4>'+warning+
        '<div class="owo-ws-inline"><button type="button" data-edge-input-action="older"'+(state.busy||!state.more||!state.nextBefore?' disabled':'')+'>更早输入页</button>'+
        '<button type="button" data-edge-input-action="latest"'+(state.busy?' disabled':'')+'>最新输入页</button>'+
        '<button type="button" data-edge-input-action="close">收起输入记录</button></div>'+
        '<p class="hint" role="status">'+status+' · 本页匹配 '+state.events.length+'/64 条'+(state.newRecords?' · 有新输入记录，可返回最新':'')+'</p>'+
        (state.error?'<p role="alert">'+esc(state.error)+'</p>':'')+
        (state.historyComplete===false||state.taskHistoryComplete===false?'<p class="hint">此运行的早期历史不完整；缺失输入不会补造。</p>':'');
      var rows=state.events.map(function(event){
        var data=event.data||{},receiver=data.target||{},from=data.source||{},prepared=event.event_type==="worker.input.prepared";
        var summary=prepared?'下游调用已准备':'本边上游输入 · '+from.artifact_id+' · 版本 '+(from.artifact_version==null?'未记录':from.artifact_version);
        var body='<p>接收步骤 '+esc(receiver.step_id)+' · 成员 '+esc(receiver.worker_id)+' · 任务 '+esc(receiver.task_id||'未记录')+' · 尝试 '+esc(receiver.attempt_id||'未记录')+'</p>'+
          '<p class="hint">调用 '+esc(data.invocation_id)+' · 事件 '+esc(event.event_id)+' · 时间 '+esc(event.timestamp||'未记录')+'</p>';
        if(prepared){
          var encodings={structured_team_context:'结构化团队上下文，尚未编译为模型提示词',serialized_team_context:'序列化团队上下文',no_team_context_in_payload:'本次未附加团队上下文'};
          body+='<p>'+esc(encodings[data.context_encoding])+' · 共 '+esc(data.source_count)+' 条上游来源（不代表都属于本边）</p>'+
            '<p>Worker Value '+esc(data.input_bytes)+' 字节 · SHA-256 '+esc(data.input_sha256)+'</p>'+
            (calls?'<button type="button" data-edge-input-action="provider-open" data-edge-input-id="'+esc(event.event_id)+'"'+(state.busy?' disabled':'')+'>查看本次模型调用</button>':'')+
            (typeof options.inspect==='function'?'<button type="button" data-edge-input-action="timeline" data-edge-input-id="'+esc(event.event_id)+'"'+(state.busy?' disabled':'')+'>定位到运行记录</button>':'');
        }else{
          body+='<p>生产步骤 '+esc(from.producer_step_id)+' · 成员 '+esc(from.producer_worker_id)+' · 任务 '+esc(from.producer_task_id||'未记录')+' · 尝试 '+esc(from.producer_attempt_id||'未记录')+'</p>'+
            '<p>产物 '+esc(from.artifact_id)+' · 输入时版本 '+esc(from.artifact_version==null?'未记录':from.artifact_version)+' · 完整内容 SHA-256 '+esc(from.artifact_sha256||'未记录')+'</p>'+
            '<p>实际输入预览 '+esc(from.preview_bytes)+' 字节 · 截断 '+esc(from.preview_truncated==null?'未记录':from.preview_truncated?'是':'否')+' · SHA-256 '+esc(from.preview_sha256)+'</p>'+
            '<p class="hint">前因 '+esc(event.causation_id)+'；不会把产物当前版本当作本次输入版本。</p>'+
            '<button type="button" data-edge-input-action="header" data-edge-input-id="'+esc(event.event_id)+'"'+(state.busy?' disabled':'')+'>定位本次准备头</button>';
        }
        return '<details class="owo-ws-tool-detail" data-edge-input-event="'+esc(event.event_id)+'"'+(state.focus===event.event_id?' open':'')+'><summary>'+esc('#'+event.seq+' '+summary)+'</summary>'+body+
          '<pre style="white-space:pre-wrap;overflow-wrap:anywhere">'+esc(JSON.stringify(event,null,2))+'</pre></details>';
      }).join('');
      return head+'<div role="log" aria-label="本边输入事件" style="max-height:440px;overflow:auto;overflow-wrap:anywhere">'+rows+
        (!state.busy&&!state.events.length?'<p class="hint">本页未找到可精确定位的输入记录；可查看更早页。不能据此判断未传递输入，也不按角色名猜测旧来源。</p>':'')+'</div></section>'+(calls?calls.html():'');
    }
    async function read(before,expected){
      if(disposed||!source||!target||!state.open)return;
      stopRead();var ctrl=new AbortController(),ticket=generation;request=ctrl;
      state.busy=true;state.error="";paint();
      try{
        var page=await options.api.get('/teams/'+encodeURIComponent(run)+'/timeline?format=json&limit=64&before='+before,{signal:ctrl.signal});
        if(disposed||ticket!==generation)return;
        validatePage(page,run,before);
        var matched=page.events.filter(function(event){return related(event,source,target);});
        matched.forEach(function(event){validateReceipt(run,event);});
        if(expected&&!matched.some(function(event){return event.event_id===expected.id&&event.event_type==='worker.input.prepared'&&event.data.invocation_id===expected.invocation;}))
          throw new Error('未找到该调用的精确准备头；没有替换为其他调用');
        if(!page.events.length&&before!==0){state.more=false;return;}
        state.events=matched;state.before=before;state.nextBefore=page.next_before;state.more=page.has_more;state.newRecords=pending;
        state.historyComplete=page.history_complete;state.taskHistoryComplete=page.task_history_complete;state.focus=expected?expected.id:null;
      }catch(error){if(!disposed&&ticket===generation&&!ctrl.signal.aborted)state.error=String(error.message||error);}
      finally{if(!disposed&&ticket===generation){request=null;state.busy=false;paint();if(pending&&state.before===0&&!state.error)schedule();}}
    }
    function schedule(){if(timer!=null||disposed||!state.open)return;timer=setTimeout(function(){timer=null;read(0);},250);}
    function navigate(action,id){
      if(disposed||!source||!target)return Promise.resolve();
      if(action==='close'){stopRead();if(calls)calls.close();state.open=false;state.events=[];state.busy=false;paint();return Promise.resolve();}
      if(action==='open'){state.open=true;return read(0);}
      if(!state.open)return Promise.resolve();
      if(action==='latest')return read(0);
      if(action==='older'&&state.more&&state.nextBefore)return read(state.nextBefore);
      if(action==='provider-open'&&calls){
        var callHeader=state.events.find(function(row){return row.event_id===id&&row.event_type==='worker.input.prepared';});
        return callHeader?calls.open(callHeader):Promise.resolve();
      }
      if(action.indexOf('provider-')===0&&calls)return calls.navigate(action.slice(9),id);
      if(action==='timeline' && typeof options.inspect==='function'){
        var prepared=state.events.find(function(row){return row.event_id===id && row.event_type==='worker.input.prepared';});
        return prepared?Promise.resolve(options.inspect(prepared)):Promise.resolve();
      }
      if(action==='header'){
        var event=state.events.find(function(row){return row.event_id===id&&row.event_type==='worker.input.source';});
        var seq=event&&causeSeq(run,event);
        if(seq!=null&&Number.isSafeInteger(seq+1))return read(seq+1,{id:event.causation_id,invocation:event.data.invocation_id});
      }
      return Promise.resolve();
    }
    function notice(event){
      if(calls)calls.notice(event);
      if(disposed||!state.open||!/^worker\.input\.(prepared|source)$/.test(String(event.event_type||''))||event.run_id!==run)return;
      state.newRecords=true;if(state.busy)pending=true;
      if(state.before===0&&!state.busy)schedule();paint();
    }
    return {state:state,html:html,reset:reset,navigate:navigate,notice:notice,calls:calls,dispose:function(){stopRead();if(calls)calls.dispose();disposed=true;state.events=[];source=null;target=null;}};
  }
  var api={create:create,related:related};root.OwoDependencyInputs=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
