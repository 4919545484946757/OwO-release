// 工作流：项目定义、校验、运行、认证事件与审批、审计。
(function () {
  'use strict';
  window.OwoPanels = window.OwoPanels || {};
  var rootEl = null, H = {}, S = null, scopes = Object.create(null), generation = 0;
  var catalogRequest = 0, flowRequest = 0, runsRequest = 0, auditRequest = 0, validationRequest = 0;
  var monitor = 0, pollTimer = null, streamController = null, streamCloseTimer = null;
  var tabs = ['flows','validate','runs','events','audit'];
  var labels = ['流程','校验','运行记录','事件与审批','审计'];
  var states = {running:'运行中',waiting_approval:'等待审批',succeeded:'已完成',failed:'失败',aborted:'已中止',error:'错误'};
  function $(id) { return rootEl && rootEl.querySelector('#' + id); }
  function alive(owner, state) { return !!rootEl && rootEl.isConnected !== false && owner === generation && state === S; }
  function esc(value) { return String(value == null ? '' : value).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;'); }
  function errorText(e) { return H.friendlyError ? H.friendlyError(e) : String(e && (e.message || e.error) || e || '请求失败'); }
  function get(path) { return H.get ? H.get(path) : window.OwoApi.stream(path).then(read); }
  function post(path, body) { return H.post ? H.post(path,body) : window.OwoApi.stream(path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)}).then(read); }
  function read(response) { return response.json().then(function (data) { if (!response.ok) throw data; return data; }); }
  function text(id,value,bad) { var el=$(id); if(el) { el.textContent=value; el.className=bad?'owo-insight-error':'sub'; } }
  function busy(id,value,label) { var el=$(id); if(!el) return; el.disabled=!!value; el.setAttribute('aria-busy',String(!!value)); if(label) el.textContent=label; if(window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(el,!!value); }
  function bind(id, event, fn) { var el=$(id), owner=generation, state=S; if(el) el.addEventListener(event,function(e){ if(alive(owner,state)) return fn(e); }); }
  function nav() {
    return '<section data-panel="workflow" data-layout="custom" class="owo-workflow-root owo-insight"><div class="owo-insight-toolbar"><div role="tablist" aria-label="工作流分区">'+tabs.map(function(tab,i){return '<button type="button" role="tab" id="workflow-tab-'+tab+'" aria-controls="workflow-pane-'+tab+'">'+labels[i]+'</button>';}).join('')+'</div><button type="button" id="owo-workflow-refresh"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1"/></svg>刷新流程</button></div>'+
      '<section role="tabpanel" id="workflow-pane-flows" aria-labelledby="workflow-tab-flows"><h3>项目流程</h3><div id="owo-workflow-list" role="status">正在读取流程…</div><section class="owo-insight-section"><h3>流程详情</h3><div id="owo-workflow-runner"><p class="sub">选择流程后查看定义与运行方式。</p></div></section></section>'+
      '<section role="tabpanel" id="workflow-pane-validate" aria-labelledby="workflow-tab-validate" hidden><h3>校验流程</h3><p class="sub">JSON 语法在本地检查；流程结构由已连接的核心校验。</p><label for="owo-workflow-validate-dsl">流程定义（JSON）</label><textarea id="owo-workflow-validate-dsl" rows="9" placeholder="粘贴 .owflow JSON 定义…"></textarea><div class="owo-insight-actions"><button type="button" id="owo-workflow-validate-btn" class="primary" data-core-action>校验</button></div><section class="owo-insight-section"><div id="owo-workflow-validate-result" role="status" aria-live="polite">粘贴流程后检查语法。</div></section></section>'+
      '<section role="tabpanel" id="workflow-pane-runs" aria-labelledby="workflow-tab-runs" hidden><h3>运行记录</h3><p class="sub" id="owo-workflow-runs-context">选择流程后读取运行记录。</p><div id="owo-workflow-runs"></div><section class="owo-insight-section"><h3>运行详情</h3><div id="owo-workflow-snapshot"><p class="sub">选择一次运行后查看状态。</p></div><p id="owo-workflow-poll-status" role="status"></p></section></section>'+
      '<section role="tabpanel" id="workflow-pane-events" aria-labelledby="workflow-tab-events" hidden><h3>实时事件</h3><p id="owo-workflow-stream-status" class="sub" role="status">选择一次运行后连接事件流，最多保留最近 200 条。</p><button id="owo-workflow-stream-retry" type="button" hidden>重新连接事件流</button><div id="owo-workflow-events" class="owo-wf-events"></div><section class="owo-insight-section"><h3>人工审批</h3><div id="owo-workflow-approval"><p class="sub">当前没有待处理审批。</p></div><p id="owo-workflow-action-status" role="status"></p></section></section>'+
      '<section role="tabpanel" id="workflow-pane-audit" aria-labelledby="workflow-tab-audit" hidden><h3>运行审计</h3><p class="sub" id="owo-workflow-audit-context">从运行详情打开对应审计。</p><div id="owo-workflow-audit"></div></section></section>';
  }
  function selectTab(tab) {
    if(!rootEl || tabs.indexOf(tab)<0) return; S.tab=tab;
    tabs.forEach(function(id){var el=$('workflow-tab-'+id); el.setAttribute('aria-selected',String(id===tab)); el.setAttribute('tabindex',id===tab?'0':'-1'); $('workflow-pane-'+id).hidden=id!==tab;});
  }
  function stopMonitor() { monitor++; if(streamCloseTimer) clearTimeout(streamCloseTimer); streamCloseTimer=null; if(pollTimer) clearTimeout(pollTimer); pollTimer=null; if(streamController) streamController.abort(); streamController=null; }
  function dispose() {
    if(rootEl && S) { var input=$('owo-workflow-validate-dsl'); if(input) S.dsl=input.value; saveParameters(); }
    stopMonitor(); generation++; catalogRequest++; flowRequest++; runsRequest++; auditRequest++; validationRequest++; rootEl=null;
  }
  function saveParameters() { if(!S || !S.flow) return; var input=$('owo-workflow-ctx'), backend=$('owo-workflow-backend'); if(input && backend) S.parameters[S.flow]={ctx:input.value,backend:backend.value}; }
  function mount(root,helpers) {
    dispose(); H=helpers||{};
    var key=(H.baseUrl||window.location.origin)+'#'+(H.coreInstanceId||(window.OwoApi&&window.OwoApi.coreInstanceId)||'');
    S=scopes[key]||(scopes[key]={tab:'flows',dsl:'',flow:'',parameters:{},starts:{},actions:{},snapshots:{},runId:'',validation:null});
    rootEl=root; root.innerHTML=nav(); $('owo-workflow-validate-dsl').value=S.dsl;
    tabs.forEach(function(tab,i){bind('workflow-tab-'+tab,'click',function(){selectTab(tab);});bind('workflow-tab-'+tab,'keydown',function(e){var next=e.key==='ArrowRight'?(i+1)%5:e.key==='ArrowLeft'?(i+4)%5:e.key==='Home'?0:e.key==='End'?4:-1;if(next<0)return;e.preventDefault();selectTab(tabs[next]);$('workflow-tab-'+tabs[next]).focus();});});
    bind('owo-workflow-refresh','click',refresh);
    bind('owo-workflow-validate-dsl','input',function(){S.dsl=$('owo-workflow-validate-dsl').value;validationRequest++;localSyntax();});
    bind('owo-workflow-validate-btn','click',validate);
    bind('owo-workflow-stream-retry','click',function(){if(S.runId) connectEvents(S.runId,monitor);});
    busy('owo-workflow-validate-btn',!!S.validation,S.validation?'校验中…':'校验');
    selectTab(S.tab); refresh();
    if(S.flow) loadFlow(S.flow);
    if(S.runId) viewRun(S.runId,false);
  }
  function emptyFlows() { return '<div class="owo-workflow-empty"><svg viewBox="0 0 32 32" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="12" y="2" width="8" height="8" rx="2"/><rect x="2" y="22" width="8" height="8" rx="2"/><rect x="22" y="22" width="8" height="8" rx="2"/><path d="M16 10v7M6 22v-5h20v5"/></svg><p>当前项目没有工作流。</p><span class="sub">将 .owflow 文件放入项目目录后刷新；运行前会校验流程与权限。</span></div>'; }
  function refresh() {
    var owner=generation,state=S,request=++catalogRequest;if(!rootEl)return Promise.resolve();
    return get('/workflow').then(function(data){
      if(!alive(owner,state)||request!==catalogRequest)return;
      if(!data||!Array.isArray(data.flows))throw new Error('流程列表响应不完整');
      if(S.flow&&!data.flows.some(function(flow){return flow.name===S.flow;})){saveParameters();S.flow='';flowRequest++;text('owo-workflow-runner','所选流程已不在当前项目中，请重新选择。');text('owo-workflow-runs-context','选择流程后读取运行记录。');$('owo-workflow-runs').innerHTML='';}
      var el=$('owo-workflow-list');el.innerHTML=data.flows.length?data.flows.map(function(f){return '<div class="owo-workflow-row"><svg class="owo-workflow-file-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6M8 13h8M8 17h8"/></svg><div><strong>'+esc(f.name)+'</strong><span class="sub">'+esc(f.path)+'</span></div><button type="button" class="primary owo-workflow-load" data-name="'+esc(f.name)+'">查看与运行</button></div>';}).join(''):emptyFlows();
      el.querySelectorAll('.owo-workflow-load').forEach(function(button){button.addEventListener('click',function(){if(alive(owner,state))return loadFlow(button.getAttribute('data-name'));});});
    }).catch(function(e){if(alive(owner,state)&&request===catalogRequest) text('owo-workflow-list','流程读取失败：'+errorText(e)+'。可刷新重试。',true);});
  }
  function stepSummary(step) {
    var names={sense:'感知',locate:'定位',act:'执行动作',assert:'条件检查',invoke_skill:'调用技能',invoke_mcp:'调用 MCP',human_approve:'人工审批',notify:'通知',subflow:'子流程',loop:'循环',cond:'条件分支',rollback_point:'回滚点'};
    var kind=step.kind||'未知步骤', spec=step.spec||{};
    return (names[kind]||kind)+' · '+(step.expr||step.prompt||step.message||step.skill||step.flow||(step.server?step.server+'.'+step.tool:'')||spec.action||spec.query||step.id||'—');
  }
  function loadFlow(name) {
    if(!rootEl)return Promise.resolve();saveParameters();S.flow=name;
    var owner=generation,state=S,request=++flowRequest; text('owo-workflow-runner','正在读取流程…');
    return get('/workflow/'+encodeURIComponent(name)).then(function(data){
      if(!alive(owner,state)||request!==flowRequest)return;
      if(!data||!data.definition||!Array.isArray(data.definition.steps)||typeof data.valid!=='boolean')throw new Error('流程定义响应不完整');
      var def=data.definition, params=S.parameters[name]||{ctx:'',backend:'mock'};
      $('owo-workflow-runner').innerHTML='<label>名称</label><p class="owo-workflow-definition-name">'+esc(def.name||name)+'</p><div class="owo-wf-chips"><span class="owo-wf-chip '+(data.valid?'owo-wf-valid':'owo-wf-invalid')+'">'+(data.valid?'已校验':'校验未通过')+'</span><span class="owo-wf-chip">'+def.steps.length+' 步</span><span class="owo-wf-chip">版本 '+esc(def.version==null?'—':def.version)+'</span></div>'+
        (data.issues||[]).map(function(issue){return '<p class="owo-insight-error">'+esc(issue)+'</p>';}).join('')+
        '<label>流程步骤</label><div class="owo-workflow-steps">'+def.steps.map(function(step,i){return '<details class="owo-wf-step"><summary><span class="owo-wf-step-idx">'+(i+1)+'</span>'+esc(stepSummary(step))+'</summary><pre>'+esc(JSON.stringify(step,null,2))+'</pre></details>';}).join('')+'</div><details class="owo-workflow-definition"><summary>完整定义与权限</summary><pre>'+esc(JSON.stringify(def,null,2))+'</pre></details>'+
        '<label for="owo-workflow-ctx">运行参数（JSON 对象，可选）</label><input id="owo-workflow-ctx" placeholder="{&quot;key&quot;: &quot;value&quot;}" /><label for="owo-workflow-backend">执行后端</label><select id="owo-workflow-backend"><option value="mock">沙箱（默认，不执行真实桌面动作）</option><option value="real">真实后端（动作仍受权限约束）</option></select><p class="sub" id="owo-workflow-backend-hint"></p><button type="button" id="owo-workflow-run-this" class="primary" data-core-action>运行</button><p id="owo-workflow-run-status" role="status"></p>';
      $('owo-workflow-ctx').value=params.ctx; $('owo-workflow-backend').value=params.backend==='real'?'real':'mock';
      function backendHint(){text('owo-workflow-backend-hint',$('owo-workflow-backend').value==='real'?'真实后端会执行获准的动作，人工审批由本页处理。':'沙箱自动通过人审节点；结果不代表真实桌面执行已验收。');saveParameters();}
      bind('owo-workflow-backend','change',backendHint);bind('owo-workflow-ctx','input',saveParameters);backendHint();
      bind('owo-workflow-run-this','click',function(){if(S.flow===name)return runFlow(name,$('owo-workflow-ctx').value);});
      busy('owo-workflow-run-this',!!S.starts[name],S.starts[name]?'正在启动…':'运行');
      return refreshRuns(name);
    }).catch(function(e){if(alive(owner,state)&&request===flowRequest)text('owo-workflow-runner','流程读取失败：'+errorText(e),true);});
  }
  function runFlow(name,ctxText) {
    if(!rootEl||S.flow!==name||S.starts[name])return Promise.resolve(null);
    var ctx;try{ctx=JSON.parse(ctxText&&ctxText.trim()?ctxText:'{}');if(!ctx||typeof ctx!=='object'||Array.isArray(ctx))throw new Error('参数必须是 JSON 对象');}catch(e){text('owo-workflow-run-status',errorText(e),true);return Promise.resolve(null);}
    var owner=generation,state=S,token={},backend=$('owo-workflow-backend').value==='real'?'real':'mock';
    S.starts[name]=token;saveParameters();busy('owo-workflow-run-this',true,'正在启动…');text('owo-workflow-run-status','正在启动流程…');
    var pending;try{pending=post('/workflow/'+encodeURIComponent(name)+'/run',{ctx:ctx,backend:backend});}catch(e){pending=Promise.reject(e);}
    return Promise.resolve(pending).then(function(data){
      if(!data||!data.run_id)throw new Error('服务未返回运行 ID');
      state.lastStarted={id:data.run_id,name:name,backend:backend};
      if(!alive(owner,state))return data;
      text('owo-workflow-run-status','已启动：'+data.run_id+' · '+(backend==='real'?'真实后端':'沙箱'));
      if(S.flow===name){viewRun(data.run_id,true);refreshRuns(name);}return data;
    }).catch(function(e){if(alive(owner,state))text('owo-workflow-run-status','启动失败：'+errorText(e),true);return null;}).finally(function(){if(state.starts[name]===token)delete state.starts[name];if(state===S&&rootEl&&S.flow===name)busy('owo-workflow-run-this',false,'运行');});
  }
  function viewRun(runId,navigate) {stopMonitor();S.runId=runId;S.snapshots[runId]=null; if(navigate)selectTab('runs');text('owo-workflow-snapshot','正在读取运行状态…');$('owo-workflow-approval').innerHTML='<p class="sub">正在同步审批状态…</p>';text('owo-workflow-action-status','');connectEvents(runId,monitor);return pollRun(runId,0,generation,0,monitor);}
  function decodeFrame(frame) {
    var raw=typeof frame==='string'?frame:String(frame.data||''), kind=frame.event||'';
    // 当前服务把内层 event/data 帧放入 SSE data；同时兼容直接 JSON 帧。
    if(/^event:/.test(raw)){var event=/^event:\s*(.+)/m.exec(raw);if(event)kind=event[1].trim();raw=raw.split(/\r?\n/).filter(function(line){return /^data:/.test(line);}).map(function(line){return line.replace(/^data:\s?/,'');}).join('\n');}
    var data;try{data=JSON.parse(raw);}catch(e){data={detail:raw};}return {event:kind||data.event||data.kind||'事件',data:data};
  }
  function connectEvents(runId,watch) {
    if(streamController)streamController.abort();if(streamCloseTimer)clearTimeout(streamCloseTimer);streamCloseTimer=null;
    var controller=new AbortController(),owner=generation,state=S,open=H.openEventStream||(window.OwoApi&&window.OwoApi.openEventStream&&window.OwoApi.openEventStream.bind(window.OwoApi));streamController=controller;
    var el=$('owo-workflow-events');if(!el)return Promise.resolve();el.innerHTML='';$('owo-workflow-stream-retry').hidden=true;text('owo-workflow-stream-status','正在连接 '+runId+' 的认证事件流…');
    if(!open){text('owo-workflow-stream-status','当前客户端不支持认证事件流；运行状态仍通过轮询同步。',true);return Promise.resolve();}
    function current(){return alive(owner,state)&&watch===monitor&&S.runId===runId&&!controller.signal.aborted;}
    var pending;try{pending=open('/workflow/run/'+encodeURIComponent(runId)+'/events',{signal:controller.signal,onOpen:function(){if(current())text('owo-workflow-stream-status','已连接 · '+runId+' · 最近 200 条事件');},onEvent:function(frame){
      if(!current())return;var decoded=decodeFrame(frame),d=decoded.data,row=document.createElement('div');row.className='owo-wf-event';
      row.innerHTML='<time>'+esc(d.ts||d.at||d.time||'')+'</time><strong>'+esc(decoded.event)+'</strong><span>'+esc(d.detail||d.message||d.state||d.step_id||d.run_id||JSON.stringify(d))+'</span>';el.appendChild(row);while(el.children.length>200)el.removeChild(el.firstElementChild);
      if(decoded.event==='state_change'&&d.state&&d.state!=='running'&&d.state!=='waiting_approval'){text('owo-workflow-stream-status','运行已结束，已保留事件记录 · '+runId);streamCloseTimer=setTimeout(function(){streamCloseTimer=null;if(current())controller.abort();},200);}
    }});}catch(e){pending=Promise.reject(e);}
    return Promise.resolve(pending).then(function(){if(current()){text('owo-workflow-stream-status','事件流已结束；可重新连接。');$('owo-workflow-stream-retry').hidden=false;}}).catch(function(e){if(current()){text('owo-workflow-stream-status','事件流中断：'+errorText(e)+'。运行状态仍通过轮询同步。',true);$('owo-workflow-stream-retry').hidden=false;}});
  }
  function pollRun(runId,attempt,owner,failures,watch) {
    if(owner==null)owner=generation;if(watch==null)watch=monitor;if(!alive(owner,S)||watch!==monitor||S.runId!==runId)return Promise.resolve();
    var state=S;return get('/workflow/run/'+encodeURIComponent(runId)).then(function(snap){
      if(!alive(owner,state)||watch!==monitor||S.runId!==runId)return;
      if(!snap||snap.run_id!==runId||typeof snap.state!=='string')throw new Error('运行快照响应不完整');
      renderSnapshot(snap);text('owo-workflow-poll-status','');
      if(snap.state==='running'||snap.state==='waiting_approval')schedule(1000,0);else if(S.flow)refreshRuns(S.flow);
    }).catch(function(e){if(!alive(owner,state)||watch!==monitor)return;var delay=Math.min(1000*Math.pow(2,Math.min(failures||0,4)),15000);text('owo-workflow-poll-status','状态同步失败，'+Math.ceil(delay/1000)+' 秒后重试：'+errorText(e),true);schedule(delay,(failures||0)+1);});
    function schedule(delay,count){if(pollTimer)clearTimeout(pollTimer);pollTimer=setTimeout(function(){pollTimer=null;pollRun(runId,0,owner,count,watch);},delay);}
  }
  function renderSnapshot(snap) {
    if(!rootEl||!snap||!snap.run_id||snap.run_id!==S.runId)return;S.snapshots[snap.run_id]=snap;
    var active=snap.state==='running'||snap.state==='waiting_approval',started=S.lastStarted&&S.lastStarted.id===snap.run_id?S.lastStarted:null;
    $('owo-workflow-snapshot').innerHTML='<div class="owo-wf-chips"><strong>'+esc(states[snap.state]||snap.state)+'</strong><span class="sub">'+esc(snap.run_id)+'</span>'+(started?'<span class="owo-wf-chip">'+(started.backend==='real'?'真实后端':'沙箱')+'</span>':'<span class="owo-wf-chip">后端信息未提供</span>')+'</div>'+(snap.error?'<p class="owo-insight-error">'+esc(snap.error)+'</p>':'')+(snap.rollback_to?'<p>已回滚到：'+esc(snap.rollback_to)+'</p>':'')+
      (Array.isArray(snap.steps)&&snap.steps.length?snap.steps.map(function(step){return '<div class="owo-workflow-step"><strong>'+esc(step.ok===true?'通过':step.ok===false?'失败':'未确认')+' · '+esc(step.kind||step.id)+'</strong><span class="sub">'+esc(step.detail||step.id||'')+'</span></div>';}).join(''):'<p class="sub">尚无步骤结果。</p>')+
      '<div class="owo-workflow-run-actions">'+(active?'<button type="button" id="owo-workflow-abort" data-core-action>中止运行</button>':'')+'<button type="button" id="owo-workflow-audit-btn">查看审计</button><button type="button" id="owo-workflow-events-btn">事件与审批</button></div>';
    if(active)bind('owo-workflow-abort','click',function(){return startRunAction(snap.run_id,'abort',{});});
    bind('owo-workflow-audit-btn','click',function(){selectTab('audit');return loadAudit(snap.run_id);});bind('owo-workflow-events-btn','click',function(){selectTab('events');});
    var pending=snap.pending_approval;
    $('owo-workflow-approval').innerHTML=snap.state==='waiting_approval'&&pending&&pending.id?'<div class="owo-workflow-approval"><p>'+esc(pending.prompt||'等待人工决定')+'</p><p class="sub">'+esc(pending.created_at||'')+' · '+esc(snap.run_id)+'</p><button type="button" id="owo-workflow-approve" class="primary" data-core-action>批准</button><button type="button" id="owo-workflow-reject" data-core-action>拒绝</button></div>':'<p class="sub">当前没有待处理审批。</p>';
    bind('owo-workflow-approve','click',function(){return decideApproval(snap.run_id,'approve');});bind('owo-workflow-reject','click',function(){return decideApproval(snap.run_id,'reject');});setRunActionUi(snap.run_id);
  }
  function setRunActionUi(runId) {var pending=S.actions[runId];[['abort','中止运行'],['approve','批准'],['reject','拒绝']].forEach(function(item){busy('owo-workflow-'+item[0],!!pending,pending?'操作处理中…':item[1]);});}
  function startRunAction(runId,action,body) {
    if(!rootEl||S.runId!==runId||S.actions[runId])return Promise.resolve(null);var snap=S.snapshots[runId];
    if(!snap||['running','waiting_approval'].indexOf(snap.state)<0||['abort','approve','reject'].indexOf(action)<0)return Promise.resolve(null);
    if(action!=='abort'&&!(snap.state==='waiting_approval'&&snap.pending_approval&&snap.pending_approval.id))return Promise.resolve(null);
    var owner=generation,state=S,token={},watch=monitor;S.actions[runId]=token;setRunActionUi(runId);text('owo-workflow-action-status','正在提交…');
    var path='/workflow/run/'+encodeURIComponent(runId)+(action==='abort'?'/abort':'/approval'),pending;
    try{pending=post(path,body||{});}catch(e){pending=Promise.reject(e);}
    return Promise.resolve(pending).then(function(result){if(alive(owner,state)&&S.runId===runId&&watch===monitor){text('owo-workflow-action-status','操作已提交，正在同步状态。');return pollRun(runId,0,owner,0,watch).then(function(){if(alive(owner,state)&&S.runId===runId){var current=S.snapshots[runId];text('owo-workflow-action-status','操作已提交'+(current?' · 当前状态：'+(states[current.state]||current.state):'')+'。');}return result;});}}).catch(function(e){if(alive(owner,state)&&S.runId===runId)text('owo-workflow-action-status','操作失败：'+errorText(e),true);}).finally(function(){if(state.actions[runId]===token)delete state.actions[runId];if(state===S&&rootEl&&S.runId===runId)setRunActionUi(runId);});
  }
  function decideApproval(id,decision){return startRunAction(id,decision,{decision:decision});}
  function loadAudit(runId) {
    var owner=generation,state=S,request=++auditRequest;text('owo-workflow-audit-context','运行 '+runId);text('owo-workflow-audit','正在读取审计…');
    return get('/workflow/run/'+encodeURIComponent(runId)+'/audit').then(function(data){if(!alive(owner,state)||request!==auditRequest)return;if(!data||!Array.isArray(data.audit))throw new Error('审计响应不完整');$('owo-workflow-audit').innerHTML=data.audit.map(function(row){return '<div class="owo-wf-event"><time>'+esc(row.ts||'')+'</time><strong>'+esc(row.event||'')+'</strong><span>'+esc(row.detail||'')+'</span></div>';}).join('')||'<p class="sub">暂无审计记录。</p>';}).catch(function(e){if(alive(owner,state)&&request===auditRequest)text('owo-workflow-audit','审计读取失败：'+errorText(e),true);});
  }
  function refreshRuns(name) {
    var owner=generation,state=S,request=++runsRequest;text('owo-workflow-runs-context','流程 '+name);
    return get('/workflow/'+encodeURIComponent(name)+'/runs').then(function(data){if(!alive(owner,state)||request!==runsRequest||S.flow!==name)return;if(!data||!Array.isArray(data.runs))throw new Error('运行记录响应不完整');var el=$('owo-workflow-runs');el.innerHTML=data.runs.length?data.runs.map(function(row){return '<div class="owo-workflow-row"><div><strong>'+esc(states[row.state]||row.state||'状态未知')+'</strong><span class="sub">'+esc(row.run_id)+' · '+esc(row.created_at||'')+'</span></div><button type="button" class="owo-workflow-snap" data-run="'+esc(row.run_id)+'">查看运行</button></div>';}).join(''):'<p class="sub">该流程暂无运行记录。</p>';el.querySelectorAll('.owo-workflow-snap').forEach(function(button){button.addEventListener('click',function(){if(alive(owner,state))return viewRun(button.getAttribute('data-run'),true);});});}).catch(function(e){if(alive(owner,state)&&request===runsRequest)text('owo-workflow-runs','记录读取失败：'+errorText(e),true);});
  }
  function localSyntax() {
    var input=$('owo-workflow-validate-dsl').value.trim();if(!input){text('owo-workflow-validate-result','粘贴流程后检查语法。');return null;}
    try{var parsed=JSON.parse(input);text('owo-workflow-validate-result','JSON 语法有效；流程结构尚未校验。');return {value:parsed};}catch(e){text('owo-workflow-validate-result','JSON 解析失败：'+e.message,true);return null;}
  }
  function validate() {
    if(!rootEl||S.validation)return Promise.resolve();var parsed=localSyntax();if(!parsed)return Promise.resolve();
    var owner=generation,state=S,request=++validationRequest,token={};S.validation=token;busy('owo-workflow-validate-btn',true,'校验中…');
    var pending;try{pending=post('/workflow/validate',parsed.value);}catch(e){pending=Promise.reject(e);}
    return Promise.resolve(pending).then(function(data){if(!alive(owner,state)||request!==validationRequest)return;if(!data||typeof data.valid!=='boolean'||!Array.isArray(data.issues))throw new Error('校验响应不完整');$('owo-workflow-validate-result').innerHTML='<p>'+ (data.valid?'流程结构校验通过。':'流程结构校验未通过。')+'</p>'+data.issues.map(function(issue){return '<p class="owo-insight-error">'+esc(issue)+'</p>';}).join('');}).catch(function(e){if(alive(owner,state)&&request===validationRequest)text('owo-workflow-validate-result','校验失败：'+errorText(e),true);}).finally(function(){if(state.validation===token)state.validation=null;if(rootEl&&state===S)busy('owo-workflow-validate-btn',false,'校验');});
  }
  window.OwoPanels.workflow={id:'workflow',title:'工作流',nav:nav,mount:mount,dispose:dispose,refresh:refresh,_test:{loadFlow:loadFlow,runFlow:runFlow,viewRun:viewRun,pollRun:pollRun,loadAudit:loadAudit,renderSnapshot:renderSnapshot,startRunAction:startRunAction,decideApproval:decideApproval,decodeFrame:decodeFrame,validate:validate}};
})();
