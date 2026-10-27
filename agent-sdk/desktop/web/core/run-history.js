// Bounded persisted event inspector. The daemon remains the canonical history store.
(function (root) {
  "use strict";
  var LIMIT = 64;
  function esc(value) { return String(value == null ? "" : value).replace(/[&<>"']/g, function(c) { return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]; }); }
  function inputProof(event) {
    if (event.event_type !== "worker.input.prepared" && event.event_type !== "worker.input.source") return "";
    var data = event.data || {}, target = data.target || {}, source = data.source || {};
    var proof = '<p class="hint">这是调用 Worker 前保存的输入记录；尚未证明模型已经接收或使用。</p>' +
      '<p>接收步骤 ' + esc(target.step_id || "未记录") + ' · 成员 ' + esc(target.worker_id || "未记录") + '</p>';
    if (event.event_type === "worker.input.prepared") {
      var encodings = { structured_team_context: "结构化团队上下文，尚未编译为模型提示词", serialized_team_context: "序列化团队上下文", no_team_context_in_payload: "本次输入未附加团队上下文" };
      return proof + '<p class="hint">' + esc(encodings[data.context_encoding] || "上下文编码未记录") + ' · 上游记录 ' + esc(data.source_count == null ? "未记录" : data.source_count) + '</p>' +
        '<p style="overflow-wrap:anywhere">Worker 输入 SHA-256：' + esc(data.input_sha256 || "未记录") + '</p>';
    }
    return proof + '<p>上游步骤 ' + esc(source.producer_step_id || "未记录") + ' · 产物 ' + esc(source.artifact_id || "未记录") + ' · 版本 ' + esc(source.artifact_version == null ? "未记录" : source.artifact_version) + '</p>' +
      '<p class="hint">上游尝试 ' + esc(source.producer_attempt_id || "未记录") + ' · 输入预览 ' + esc(source.preview_bytes == null ? "未记录" : source.preview_bytes) + ' 字节 · 是否截断：' + esc(source.preview_truncated == null ? "未记录" : source.preview_truncated ? "是" : "否") + '</p>' +
      '<p style="overflow-wrap:anywhere">预览 SHA-256：' + esc(source.preview_sha256 || "未记录") + '</p>';
  }
  function count(value) { return Number.isSafeInteger(value) && value >= 0; }
  function digest(value) { return value && count(value.bytes) && /^[a-f0-9]{64}$/.test(value.sha256); }
  function modelRecord(event) {
    var kind=event.event_type, data=event.data || {};
    if (["model.input.prepared","model.request.completed","model.request.interrupted"].indexOf(kind)<0) return null;
    var valid=typeof data.call_id==="string" && data.call_id.length>0 && data.call_id.length<=256, title, body;
    if (kind==="model.input.prepared") {
      valid=valid && data.boundary==="before_provider_delegate" && data.model_delivery==="not_proven" &&
        data.representation==="gateway_chat_message_v1_including_input_metadata" && digest(data.request) &&
        count(data.message_count) && count(data.tool_count) && count(data.image_count) &&
        (data.worker_session_id==null || typeof data.worker_session_id==="string");
      title="Provider 请求输入已准备";
      body='<p>消息 '+esc(data.message_count)+' 条 · 工具 '+esc(data.tool_count)+' 个 · 图片 '+esc(data.image_count)+' 张</p>'+
        '<p class="hint">Worker 会话 '+esc(data.worker_session_id || "未记录")+'</p>';
      if (valid) body+='<p style="overflow-wrap:anywhere">请求表示 '+esc(data.request.bytes)+' 字节 · SHA-256 '+esc(data.request.sha256)+'</p>';
      var input=data.last_user_input;
      if (input!=null) {
        var parts=input.parts;
        valid=valid && digest(input.identity) && digest(input.prompt) && count(input.part_count) &&
          count(input.legacy_attachment_count) && Array.isArray(parts) && parts.length===Math.min(8,input.part_count) &&
          input.parts_truncated===(input.part_count>8);
        if (valid) {
          body+='<p>本次 Worker 最近用户输入 '+esc(input.part_count)+' 个内容部分 · 旧附件 '+esc(input.legacy_attachment_count)+' 个'+
            (input.parts_truncated?' · 仅展示前 8 项':'')+'</p>';
          parts.forEach(function(part) {
            if (!part || typeof part!=="object") { valid=false; return; }
            if (part.type==="text") {
              if (!digest(part.identity)) { valid=false; return; }
              body+='<p class="hint">文本部分 '+esc(part.identity.bytes)+' 字节 · SHA-256 '+esc(part.identity.sha256)+'</p>';
            } else {
              var names={image:"图片",audio:"音频",video:"视频",file:"文件"}, asset=part.asset;
              if (!names[part.type] || !asset || !(digest(asset.identity) ||
                (typeof asset.artifact_id==="string" && asset.artifact_id.length>0 && asset.artifact_id.length<=256 && /^[a-f0-9]{64}$/.test(asset.sha256)))) {
                valid=false;return;
              }
              body+='<p class="hint">'+names[part.type]+' · '+esc(asset.artifact_id || "引用仅保留摘要")+' · SHA-256 '+esc(asset.sha256 || asset.identity.sha256)+'</p>';
            }
          });
        }
      }
      body+='<p class="hint">摘要对应网关消息表示，包含输入元数据；不是最终网络请求。这里的用户输入属于 Worker 调用，不等同于最初建队原话。</p>';
    } else if (kind==="model.request.completed") {
      valid=valid && data.boundary==="provider_returned" && data.model_delivery==="not_proven" &&
        typeof data.provider_succeeded==="boolean" && (data.latency_ms==null || count(data.latency_ms)) &&
        (data.request_id==null || digest(data.request_id)) && (data.model==null || digest(data.model));
      title=data.provider_succeeded===true?"Provider 返回成功":data.provider_succeeded===false?"Provider 返回失败":"Provider 返回状态未知";
      var usage=data.usage;
      if (usage!=null) valid=valid && count(usage.prompt_tokens) && count(usage.completion_tokens) && count(usage.total_tokens);
      body='<p>耗时 '+esc(data.latency_ms==null?"未提供":data.latency_ms+" ms")+' · '+
        (usage!=null?'输入 '+esc(usage.prompt_tokens)+' / 输出 '+esc(usage.completion_tokens)+' / 总计 '+esc(usage.total_tokens)+' tokens':'用量未提供，不能按零计费')+'</p>'+
        '<p class="hint">此状态只描述 Provider 返回；不代表任务已完成、产物已验证或远端模型已理解输入。</p>';
    } else {
      valid=valid && data.boundary==="observer_dropped_before_result_receipt" &&
        data.model_delivery==="unknown" && data.provider_succeeded===null;
      title="Provider 观测中断，结果未知";
      body='<p class="hint">在结果回执保存前观测已中断。不能据此认定网络已取消或请求已完成；未闭合的请求不会自动补造成功。</p>';
    }
    return {valid:!!valid,title:valid?title:"Provider 回执字段无效",html:valid?
      '<p>调用 '+esc(data.call_id)+'</p>'+body+'<p class="hint">尚未证明最终模型接收或使用。</p>':
      '<p role="alert">Provider 回执字段无效，未据此判断调用成功或输入送达；可检查原始事件。</p>'};
  }
  function causeOf(run,event) {
    var prefix="run:"+run+":", id=String(event.causation_id || ""), suffix=id.indexOf(prefix)===0?id.slice(prefix.length):"", seq=Number(suffix);
    if (!/^[1-9][0-9]*$/.test(suffix) || !Number.isSafeInteger(seq+1) || seq>=event.seq) return null;
    var type, key, value, model=modelRecord(event);
    if (model && model.valid) {
      type=event.event_type==="model.input.prepared"?"worker.input.prepared":"model.input.prepared";
      if (type==="model.input.prepared") { key="call_id"; value=event.data.call_id; }
    } else if (event.event_type==="worker.input.source" && typeof (event.data || {}).invocation_id==="string") {
      type="worker.input.prepared";key="invocation_id";value=event.data.invocation_id;
    } else return null;
    return {id:id,seq:seq,type:type,key:key,value:value,node:event.node_id || null,task:event.task_id || null,attempt:event.attempt || null};
  }
  function matchesCause(event,expected) {
    if (event.event_id!==expected.id || event.event_type!==expected.type ||
        (event.node_id || null)!==expected.node || (event.task_id || null)!==expected.task || (event.attempt || null)!==expected.attempt) return false;
    var data=event.data || {};
    if (expected.key && data[expected.key]!==expected.value) return false;
    if (expected.type==="model.input.prepared") { var model=modelRecord(event);return !!model && model.valid; }
    var target=data.target || {};
    return data.boundary==="before_worker_run" && data.model_delivery==="not_proven" &&
      typeof data.invocation_id==="string" && data.invocation_id.length>0 && typeof target.step_id==="string" &&
      typeof target.worker_id==="string" && (target.task_id || null)===expected.task && (target.attempt_id || null)===expected.attempt;
  }
  function mergeWindow(first, second) {
    var byId = new Map();
    first.concat(second).forEach(function(event) { byId.set(event.event_id, event); });
    return Array.from(byId.values()).sort(function(a,b) { return a.seq-b.seq; }).slice(-LIMIT);
  }
  function create(o) {
    var run = o.runId, box = o.box, disposed = false, request = null, ticket = 0, timer = null, pendingLive = [];
    var state = { events: [], follow: true, busy: false, older: false, newer: false, error: "", history_complete: null, task_history_complete: null };
    var lastRead = ["before", 0, null], focusPainted=null;
    state.focus=null;
    var snapshots=root.OwoWorkerInputSnapshot;
    if(!snapshots&&typeof require==="function")snapshots=require("./worker-input-snapshot.js");
    var snapshot=snapshots?snapshots.create({runId:run,api:o.api,surface:"history",paint:paint}):null;
    function identity(event) {
      if (!event || event.run_id !== run || event.schema_version !== 1 || !Number.isSafeInteger(event.seq) || event.seq <= 0 ||
          event.event_id !== "run:" + run + ":" + event.seq || typeof event.event_type !== "string" || new TextEncoder().encode(JSON.stringify(event)).length > 65536) {
        throw new Error("运行历史事件身份或大小无效");
      }
    }
    function html() {
      var names = { "task.created":"任务创建", "task.running":"任务执行", "task.failed":"任务失败", "task.aborted":"任务已取消",
        "task.succeeded":"任务完成", "task.pending":"任务等待", "tool.started":"调用工具", "tool.finished":"工具返回",
        "message.delta":"消息片段", "message.final":"成员回复", "handoff.saved":"交接", "artifact.saved":"产物保存",
        "worker.input.prepared":"Worker 输入已准备", "worker.input.source":"Worker 上游输入记录" };
      return '<div class="owo-ws-inline"><button type="button" class="owo-ws-mini" data-history-action="older"' + (state.busy || !state.older ? " disabled":"") + '>更早一页</button>' +
        '<button type="button" class="owo-ws-mini" data-history-action="newer"' + (state.busy || !state.newer ? " disabled":"") + '>更新一页</button>' +
        '<button type="button" class="owo-ws-mini" data-history-action="latest"' + (state.busy ? " disabled":"") + '>最新并跟随</button>' +
        '<span class="hint" role="status">' + (state.busy ? "正在读取…" : state.follow ? "跟随最新记录" : "查看历史，实时执行继续") + ' · 本页 ' + state.events.length + '/64 条</span></div>' +
        (state.history_complete === false || state.task_history_complete === false ? '<p class="hint">此运行的早期历史不完整，缺失记录不会补造。</p>' : "") +
        (state.error ? '<p role="alert">' + esc(state.error) + ' <button type="button" data-history-action="retry">重试读取</button></p>' : "") +
        '<div role="log" aria-label="持久运行事件" style="max-height:560px;overflow:auto;overflow-wrap:anywhere">' +
        state.events.map(function(event) {
          var model=modelRecord(event), cause=causeOf(run,event);
          var task = event.data && event.data.task || {};
          var reason = task.waiting_reason ? " · 等待：" + task.waiting_reason : "";
          return '<details class="owo-ws-tool-detail" data-run-event="' + esc(event.event_id) + '"><summary>' +
            esc("#" + event.seq + " " + (model ? model.title : names[event.event_type] || event.event_type) + " · " + (event.node_id || event.task_id || "团队") + reason) +
            '</summary><div class="hint">事件 ' + esc(event.event_id) + " · 任务 " + esc(event.task_id || "未知") + " · 执行尝试 " + esc(event.attempt || "未知") +
            " · 前因 " + esc(event.causation_id || "未记录") + '</div>' +
            inputProof(event) + (model ? model.html : "") +
            (snapshot&&snapshots.canInspect(event,run)?'<button type="button" data-history-action="snapshot" data-history-event="'+esc(event.event_id)+'"'+(state.busy?' disabled':'')+'>查看输入快照</button>':'') +
            (model&&model.valid&&event.event_type==="model.input.prepared"&&snapshot&&!snapshots.canInspect(event,run)?'<p class="hint">此请求缺少可核对的 Worker 会话或输入摘要，无法读取精确快照。</p>':'') +
            (cause ? '<button type="button" data-history-action="cause" data-history-event="'+esc(event.event_id)+'"'+(state.busy?' disabled':'')+'>定位'+(cause.type==='model.input.prepared'?'本次请求输入':'本次 Worker 输入')+'</button>':'') +
            ((event.data || {}).payload_complete === false ? '<p class="hint">公开预览不完整，以下内容不能替代完整源记录。</p>' : "") +
            '<pre style="white-space:pre-wrap;overflow-wrap:anywhere">' + esc(JSON.stringify(event, null, 2)) + '</pre></details>';
        }).join("") + (!state.events.length && !state.busy ? '<p class="hint">本页没有已提交的运行记录。</p>' : "") + "</div>" +
        (snapshot&&snapshot.state.receipt&&!state.events.some(function(e){return e.event_id===snapshot.state.receipt.event_id;})?'<p class="hint">选中请求已移出当前事件窗口；以下快照仍属于 '+esc(snapshot.state.receipt.event_id)+'，不是最新输入。翻页将关闭此快照。</p>':"")+
        (snapshot?snapshot.html():"");
    }
    function paint() {
      if (disposed || !box) return;
      var opened = Array.from(box.querySelectorAll("details[data-run-event][open]")).map(function(node){return node.getAttribute("data-run-event");});
      var forceFocus=state.focus && focusPainted!==state.focus;
      var log = box.querySelector('[role="log"]'), top = log ? log.scrollTop : 0;
      var atEnd = !log || log.scrollHeight - log.clientHeight - top < 48;
      if(root.OwoMediaResult)root.OwoMediaResult.renderInto(box,html());else box.innerHTML = html();
      box.querySelectorAll("details[data-run-event]").forEach(function(node){ node.open = forceFocus && node.getAttribute("data-run-event")===state.focus || opened.indexOf(node.getAttribute("data-run-event")) >= 0; });
      log = box.querySelector('[role="log"]');
      if (log) log.scrollTop = state.follow && atEnd ? log.scrollHeight : top;
      if (state.focus && focusPainted!==state.focus) {
        box.querySelectorAll("details[data-run-event]").forEach(function(node) {
          if (node.getAttribute("data-run-event")===state.focus && node.scrollIntoView) node.scrollIntoView({block:"nearest"});
        });
        focusPainted=state.focus;
      }
    }
    function schedulePaint() {
      if (timer != null || disposed) return;
      timer = setTimeout(function(){timer=null;paint();},100);
    }
    function meta(page) { state.history_complete = page.history_complete; state.task_history_complete = page.task_history_complete; }
    async function read(direction, cursor, follow, expected) {
      if (disposed) return;
      if(snapshot)snapshot.close();
      if (request) request.abort();
      var ctrl = new AbortController(), version = ++ticket;
      request = ctrl; lastRead = [direction,cursor,expected || null];
      state.follow = !!follow; state.busy = true; state.error = ""; pendingLive = [];
      paint();
      try {
        var page = await o.api.get("/teams/" + encodeURIComponent(run) + "/timeline?format=json&limit=64&" + direction + "=" + cursor, {signal:ctrl.signal});
        if (disposed || version !== ticket) return;
        if (page.run_id !== run || !Array.isArray(page.events) || page.events.length > LIMIT || typeof page.has_more !== "boolean") throw new Error("运行历史分页无效");
        page.events.forEach(identity);
        if (!page.events.every(function(event,index){return index===0 || event.seq>page.events[index-1].seq;})) throw new Error("运行历史分页顺序无效");
        if (direction === "before" && !Object.prototype.hasOwnProperty.call(page,"next_before")) throw new Error("服务端尚不支持向前历史分页");
        if (expected && (page.next_before!==(page.events.length?page.events[0].seq:null) ||
            page.events.some(function(event){return event.seq>=cursor;}) ||
            !page.events.some(function(event){return matchesCause(event,expected);}))) {
          throw new Error("未找到具有相同调用、任务和尝试身份的精确前因；保留当前页面");
        }
        if (!page.events.length && cursor !== 0) { // preserve the current visible page at a boundary
          if (direction === "before") state.older = false; else state.newer = false;
        } else {
          state.events = follow ? mergeWindow(page.events,pendingLive) : page.events;
          state.focus=expected?expected.id:null;focusPainted=null;
          state.older = direction === "before" ? page.has_more || (follow && new Set(page.events.concat(pendingLive).map(function(event){return event.event_id;})).size > LIMIT) : cursor > 0;
          state.newer = follow ? false : direction === "after" ? page.has_more : state.events.length > 0 && page.high_watermark > state.events[state.events.length-1].seq;
        }
        meta(page);
      } catch(error) {
        if (!disposed && version === ticket && !ctrl.signal.aborted) state.error = String(error.message || error);
      } finally {
        if (!disposed && version === ticket) { request=null;pendingLive=[];state.busy=false;paint(); }
      }
    }
    function push(event) {
      if (disposed) return;
      identity(event);
      if (!state.follow) { state.newer=true;schedulePaint();return; }
      if (state.busy) pendingLive = mergeWindow(pendingLive,[event]);
      if (state.events.length === LIMIT && !state.events.some(function(old){return old.event_id===event.event_id;})) state.older=true;
      state.events = mergeWindow(state.events,[event]);
      schedulePaint();
    }
    function navigate(action,id) {
      if(disposed)return Promise.resolve();
      if(action==="snapshot-close"&&snapshot){snapshot.close();return Promise.resolve();}
      if(action==="snapshot-retry"&&snapshot)return snapshot.state.receipt?snapshot.open(snapshot.state.receipt):Promise.resolve();
      if(action==="snapshot"&&snapshot&&!state.busy){
        var selected=state.events.find(function(e){return e.event_id===id;});
        return selected&&snapshots.canInspect(selected,run)?snapshot.open(selected):Promise.resolve();
      }
      if (action === "latest") return read("before",0,true);
      if (action === "retry") return read(lastRead[0],lastRead[1],state.follow,lastRead[2]);
      if (action === "cause") {
        var event=state.events.find(function(record){return record.event_id===id;}), expected=event && causeOf(run,event);
        return expected ? read("before",expected.seq+1,false,expected) : Promise.resolve();
      }
      if (!state.events.length) return Promise.resolve();
      if (action === "older" && state.older) return read("before",state.events[0].seq,false);
      if (action === "newer" && state.newer) return read("after",state.events[state.events.length-1].seq,false);
      return Promise.resolve();
    }
    function locate(event) {
      if (disposed) return Promise.resolve();
      identity(event);
      var data=event.data || {}, expected={id:event.event_id,seq:event.seq,type:"worker.input.prepared",key:"invocation_id",value:data.invocation_id,node:event.node_id || null,task:event.task_id || null,attempt:event.attempt || null};
      if (event.event_type!=="worker.input.prepared" || !matchesCause(event,expected) || !Number.isSafeInteger(event.seq+1)) return Promise.resolve();
      return read("before",event.seq+1,false,expected);
    }
    function click(event) { var button=event.target.closest("[data-history-action]"); if(button && box.contains(button) && !button.disabled) navigate(button.getAttribute("data-history-action"),button.getAttribute("data-history-event")); }
    if (box) box.addEventListener("click",click);
    return { start: function(){return read("before",0,true);}, push:push, navigate:navigate, locate:locate, state:state, html:html, snapshot:snapshot,
      dispose:function(){disposed=true;if(snapshot)snapshot.dispose();if(box&&root.OwoMediaResult&&root.OwoMediaResult.disposeWithin)root.OwoMediaResult.disposeWithin(box);ticket++;if(request)request.abort();request=null;pendingLive=[];state.events=[];if(timer!=null)clearTimeout(timer);timer=null;if(box)box.removeEventListener("click",click);} };
  }
  var api = {create:create, mergeWindow:mergeWindow, limit:LIMIT, describeModel:modelRecord};
  root.OwoRunHistory=api;
  if(typeof module!=="undefined" && module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
