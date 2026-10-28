// Bounded, read-only projection of one exact Worker invocation's Provider receipts.
// Notices trigger canonical reads; no second stream, scheduler, replay or model invocation.
(function(root){
  "use strict";
  var H=root.OwoRunHistory;
  if(!H&&typeof require==="function")H=require("./run-history.js");
  // One page can legitimately contain 64 events of up to 64 KiB each.
  var LIMIT=64,MAX_BYTES=LIMIT*65536+65536;
  function esc(v){return String(v==null?"":v).replace(/[&<>"']/g,function(c){return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c];});}
  function bytes(v){return new TextEncoder().encode(JSON.stringify(v)).length;}
  function count(v){return Number.isSafeInteger(v)&&v>=0;}
  function same(a,b){return (a.node_id||null)===(b.node_id||null)&&(a.task_id||null)===(b.task_id||null)&&(a.attempt||null)===(b.attempt||null);}
  function identity(e,run){
    if(!e||e.run_id!==run||e.schema_version!==1||!Number.isSafeInteger(e.seq)||e.seq<=0||
      e.event_id!=="run:"+run+":"+e.seq||typeof e.event_type!=="string"||bytes(e)>65536)throw new Error("调用记录事件身份或大小无效");
  }
  function headerValid(e,run){
    identity(e,run);var d=e.data||{},t=d.target||{};
    if(e.event_type!=="worker.input.prepared"||d.boundary!=="before_worker_run"||d.model_delivery!=="not_proven"||
      typeof d.invocation_id!=="string"||!d.invocation_id||typeof t.step_id!=="string"||!t.step_id||
      typeof t.worker_id!=="string"||!t.worker_id||(t.task_id||null)!==(e.task_id||null)||(t.attempt_id||null)!==(e.attempt||null)||
      typeof d.input_sha256!=="string"||!/^[a-f0-9]{64}$/.test(d.input_sha256)||!count(d.input_bytes))throw new Error("Worker 准备头无效，未读取模型调用");
  }
  function pageValid(p,run,after){
    if(!p||p.run_id!==run||!Array.isArray(p.events)||p.events.length>LIMIT||bytes(p)>MAX_BYTES||
      !count(p.high_watermark)||p.high_watermark<after||typeof p.has_more!=="boolean"||
      typeof p.history_complete!=="boolean"||(p.task_history_complete!=null&&typeof p.task_history_complete!=="boolean")||
      p.next_after!==(p.events.length?p.events[p.events.length-1].seq:after)||(!p.events.length&&p.has_more))throw new Error("调用历史分页协议或游标无效");
    var previous=after;
    p.events.forEach(function(e){identity(e,run);if(e.seq<=previous||e.seq>p.high_watermark)throw new Error("调用历史顺序或上界无效");previous=e.seq;});
  }
  function create(options){
    var run=String(options.runId||""),generation=0,request=null,timer=null,disposed=false,noticeHigh=0,staleReads=0;
    var setTimer=options.setTimeout||setTimeout,clearTimer=options.clearTimeout||clearTimeout;
    var state={header:null,calls:[],cursor:0,high:0,more:false,busy:false,error:"",evicted:0,unattributedResults:0,historyComplete:null,taskHistoryComplete:null,newRecords:false};
    var snapshots=root.OwoWorkerInputSnapshot;
    if(!snapshots&&typeof require==="function")snapshots=require("./worker-input-snapshot.js");
    var snapshot=snapshots?snapshots.create({runId:run,api:options.api,paint:paint}):null;
    function paint(){if(!disposed&&options.paint)options.paint();}
    function stop(){generation++;if(request)request.abort();request=null;if(timer!=null)clearTimer(timer);timer=null;}
    function close(){stop();if(snapshot)snapshot.close();noticeHigh=0;staleReads=0;Object.assign(state,{header:null,calls:[],cursor:0,high:0,more:false,busy:false,error:"",evicted:0,unattributedResults:0,historyComplete:null,taskHistoryComplete:null,newRecords:false});paint();}
    function apply(page){
      var next=state.calls.map(function(c){return {input:c.input,result:c.result};}),evicted=state.evicted,unattributed=state.unattributedResults;
      page.events.forEach(function(e){
        if(!/^model\.(input\.prepared|request\.(completed|interrupted))$/.test(e.event_type))return;
        var d=e.data||{},ownInput=next.find(function(c){return c.input.event_id===e.causation_id;}),ownId=next.find(function(c){return c.input.data.call_id===d.call_id;});
        if(e.event_type==="model.input.prepared"){
          if(e.causation_id!==state.header.event_id)return;
          var model=H.describeModel(e);
          if(!same(e,state.header)||!model||!model.valid||e.seq<=state.header.seq||ownId)throw new Error("请求准备记录的调用、任务或前因冲突");
          next.push({input:e,result:null});
          if(next.length>LIMIT){next.shift();evicted++;}
        }else if(ownInput||ownId){
          var result=H.describeModel(e);
          if(!ownInput||ownInput!==ownId||!same(e,ownInput.input)||!result||!result.valid||e.seq<=ownInput.input.seq)throw new Error("结果回执与本次请求身份不一致");
          if(ownInput.result)throw new Error("同一请求出现多个结果回执，未用后者覆盖");
          ownInput.result=e;
        }else if(same(e,state.header))unattributed++;
      });
      state.calls=next;state.evicted=evicted;state.unattributedResults=unattributed;state.cursor=page.next_after;state.high=page.high_watermark;
      state.more=page.has_more;state.historyComplete=page.history_complete;state.taskHistoryComplete=page.task_history_complete;
      state.newRecords=noticeHigh>state.cursor;
    }
    async function read(){
      if(disposed||!state.header||state.busy)return;
      if(timer!=null)clearTimer(timer);timer=null;
      var startedCursor=state.cursor;
      var ctrl=new AbortController(),ticket=generation;request=ctrl;state.busy=true;state.error="";paint();
      try{
        for(var n=0;n<4;n++){
          var after=state.cursor;
          var page=await options.api.get("/teams/"+encodeURIComponent(run)+"/timeline?format=json&limit=64&after="+after,{signal:ctrl.signal});
          if(disposed||ticket!==generation)return;
          pageValid(page,run,after);apply(page);paint();
          if(!state.more)break;
        }
      }catch(error){if(!disposed&&ticket===generation&&!ctrl.signal.aborted)state.error=String(error.message||error);}
      finally{if(!disposed&&ticket===generation){request=null;state.busy=false;staleReads=state.cursor===startedCursor?staleReads+1:0;paint();if(state.newRecords&&!state.more&&!state.error)schedule();}}
    }
    function open(header){
      if(disposed)return Promise.resolve();
      try{headerValid(header,run);}catch(error){state.error=String(error.message||error);paint();return Promise.resolve();}
      stop();if(snapshot)snapshot.close();noticeHigh=0;staleReads=0;
      Object.assign(state,{header:JSON.parse(JSON.stringify(header)),calls:[],cursor:header.seq,high:header.seq,more:false,busy:false,error:"",evicted:0,unattributedResults:0,historyComplete:null,taskHistoryComplete:null,newRecords:false});
      return read();
    }
    function schedule(){if(timer!=null||disposed||!state.header)return;timer=setTimer(function(){timer=null;read();},Math.min(10000,250*Math.pow(2,Math.min(6,staleReads))));}
    function notice(e){
      if(disposed||!state.header||!e||e.run_id!==run||!count(e.seq)||e.seq<=state.cursor||!/^model\./.test(e.event_type)||!same(e,state.header))return;
      noticeHigh=Math.max(noticeHigh,e.seq);state.newRecords=true;
      if(!state.busy&&!state.more&&!state.error)schedule();paint();
    }
    function navigate(action,id){
      if(action==="snapshot-close"&&snapshot){snapshot.close();return Promise.resolve();}
      if(action==="snapshot-retry"&&snapshot)return snapshot.state.receipt?snapshot.open(snapshot.state.receipt):Promise.resolve();
      if(action==="snapshot"&&snapshot){
        var call=state.calls.find(function(c){return c.input.event_id===id;});
        return call?snapshot.open(call.input):Promise.resolve();
      }
      if(action==="close"){close();return Promise.resolve();}
      if(!state.header||disposed)return Promise.resolve();
      if(action==="restart")return open(state.header);
      if(action==="continue"||action==="refresh"||action==="retry")return read();
      return Promise.resolve();
    }
    function html(){
      if(!state.header)return state.error?'<p role="alert">'+esc(state.error)+'</p>':"";
      var d=state.header.data,t=d.target,latest=!state.more&&!state.newRecords&&state.cursor>=state.high;
      var warning=state.more||state.newRecords?"尚未读取至最新，缺少结果不能判定请求未完成":"截至已读游标尚无结果回执的请求保持未知";
      var rows=state.calls.map(function(c){
        var result=c.result,description=H.describeModel(result||c.input);
        var title=result?description.title:"尚无结果回执";
        return '<details class="owo-ws-tool-detail" data-provider-call="'+esc(c.input.event_id)+'"><summary>'+esc(c.input.data.call_id)+' · '+esc(title)+'</summary>'+
          '<p class="hint">请求 '+esc(c.input.event_id)+' → '+esc(result?result.event_id:"截至 #"+state.cursor+" 未找到结果")+'</p>'+
          H.describeModel(c.input).html+(snapshot&&c.input.data.last_user_input?'<button type="button" data-edge-input-action="provider-snapshot" data-edge-input-id="'+esc(c.input.event_id)+'">查看输入快照</button>':'')+(result?description.html:'<p class="hint">'+esc(warning)+'；不自动重试或补造成功，也不能推断网络已经取消。</p>')+'</details>';
      }).join("");
      return '<section aria-label="本次 Worker 模型调用" style="overflow-wrap:anywhere"><h4>本次 Worker 模型调用</h4>'+
        '<p>步骤 '+esc(t.step_id)+' · 成员 '+esc(t.worker_id)+' · 尝试 '+esc(t.attempt_id||"未记录")+' · 调用 '+esc(d.invocation_id)+'</p>'+
        '<p class="hint">只配对精确准备头 '+esc(state.header.event_id)+' 的 Provider 请求。未证明远端接收或理解，不代表任务完成。</p>'+
        '<div class="owo-ws-inline"><button type="button" data-edge-input-action="provider-continue"'+(state.busy||!state.more?' disabled':'')+'>继续读取后续记录</button>'+
        '<button type="button" data-edge-input-action="provider-refresh"'+(state.busy?' disabled':'')+'>刷新调用回执</button>'+
        '<button type="button" data-edge-input-action="provider-restart"'+(state.busy?' disabled':'')+'>从准备头重新读取</button>'+
        '<button type="button" data-edge-input-action="provider-close">收起模型调用</button></div>'+
        '<p class="hint" role="status">'+(state.busy?"正在读取…":latest?"已读取至当前可见末尾":"后续记录尚未读完")+' · 已读 #'+state.cursor+' / 服务端 #'+state.high+' · 保留 '+state.calls.length+'/64 个请求</p>'+
        (state.evicted?'<p class="hint">较早 '+state.evicted+' 个请求已移出此有界视图；不能据此判断全部调用已完成，可从准备头重新读取。</p>':"")+
        (state.unattributedResults?'<p class="hint">同任务/尝试有 '+state.unattributedResults+' 个结果无法关联本次准备头，可能属于其他调用或缺失历史，不计作本次结果。</p>':"")+
        (state.historyComplete===false||state.taskHistoryComplete===false?'<p class="hint">早期历史不完整，缺失回执不会补造。</p>':"")+
        (state.error?'<p role="alert">'+esc(state.error)+' <button type="button" data-edge-input-action="provider-retry">重试调用读取</button></p>':"")+
        '<div role="log" aria-label="已配对模型调用" style="max-height:440px;overflow:auto;overflow-wrap:anywhere">'+rows+
        (!state.calls.length&&!state.busy?'<p class="hint">已读范围未找到属于本次准备头的请求；可继续读取。这不证明没有模型调用。</p>':"")+'</div>'+(snapshot?snapshot.html():"")+'</section>';
    }
    return {state:state,html:html,open:open,close:close,navigate:navigate,notice:notice,snapshot:snapshot,dispose:function(){disposed=true;if(snapshot)snapshot.dispose();close();}};
  }
  var api={create:create};root.OwoProviderCalls=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
