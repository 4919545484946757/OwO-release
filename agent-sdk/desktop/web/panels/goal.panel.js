/* Lane D 编排面板：Goal/Plan 编排 API + 云端 SSE 进度。
 * 纯脚本 IIFE，注册 window.OwoPanels.goal。
 * 防御性降级：helpers 缺省时自建（fetch + esc + friendlyError）。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.goal = (function () {
  "use strict";

  var id = "goal";

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      });
    }
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) {
          return r.json().then(function (j) {
            throw new Error((j && j.error) || "HTTP " + r.status);
          });
        }
        return r.json();
      });
    }
    function esc(s) {
      return String(s == null ? "" : s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function friendlyError(e) {
      return "操作失败：" + (e && e.message ? e.message : String(e));
    }
    function renderMarkdown(text) {
      return esc(text);
    }
    return { baseUrl: baseUrl, get: get, post: post, esc: esc, friendlyError: friendlyError, renderMarkdown: renderMarkdown };
  }

  var H = defaultHelpers();
  function notify(message, kind) {
    if (H.notify) H.notify(message, kind || "error");
    else if (window.OwoToast) window.OwoToast(message);
    else window.alert(message);
  }
  var panelGeneration = 0;
  var listGeneration = 0;
  var detailGeneration = 0;
  var statusGeneration = 0;
  var auditGeneration = 0;
  var creatingGoal = false;
  var activeCreateButton = null;
  var savingPlans = Object.create(null);
  var goalActionPending = Object.create(null);
  var panelMounted = false;
  var state = {
    goals: [],
    current: null,
    status: null,
    pollTimer: null,
    cloudLog: [],
  };

  var hostScope = "";
  var activeTab = "goals";
  var detailTab = "plan";
  var goalQuery = "";
  var goalFilter = "";
  var planDrafts = Object.create(null);
  var renderedGoalId = null;
  var goalDraft = "";
  var editorOpen = false;
  var detailPlanError = false;
  var detailPlanPresent = false;
  var runtimeStatuses = Object.create(null);
  var listLoading = false;
  var listError = null;

  function el(id) { return document.getElementById(id); }
  function isMissing(error) { return error && (error.status === 404 || /\b404\b/.test(String(error.message || error))); }
  function localTime(value) {
    if (!value) return "时间未知";
    var date = new Date(value);
    return Number.isNaN(date.getTime()) ? "时间未知" : date.toLocaleString("zh-CN", { hour12: false });
  }
  function goalStatus(goal) { return runtimeStatuses[goal.id] || goal.status; }
  function label(status) {
    return ({Pending:"未运行",Planning:"计划中",Running:"运行中",Verifying:"校验中",Succeeded:"已完成",Failed:"失败",Aborted:"已中止",Skipped:"已跳过"})[status] || "未知状态";
  }
  function setBusy(button, busy, text) {
    if (!button) return;
    if (button.setAttribute) {
      if (busy) { button.setAttribute("aria-busy","true"); button.setAttribute("data-core-busy","true"); }
      else { button.removeAttribute("aria-busy"); button.removeAttribute("data-core-busy"); }
    }
    if (window.OwoCoreActionAvailability && window.OwoCoreActionAvailability.setBusy) window.OwoCoreActionAvailability.setBusy(button,busy);
    else button.disabled = !!busy;
    if (text != null) button.textContent = text;
  }
  function selectTab(tab) {
    activeTab = tab === "cloud" ? "cloud" : "goals";
    ["goals","cloud"].forEach(function (name) {
      var button = el("owo-goal-tab-" + name), pane = el("owo-goal-pane-" + name);
      if (button && button.setAttribute) { button.setAttribute("aria-selected",String(activeTab === name)); button.setAttribute("tabindex",activeTab === name ? "0" : "-1"); }
      if (pane) pane.hidden = activeTab !== name;
    });
  }
  function selectDetailTab(tab) {
    detailTab = ["plan","status","audit"].indexOf(tab) !== -1 ? tab : "plan";
    ["plan","status","audit"].forEach(function (name) {
      var button = el("owo-goal-detail-tab-" + name), pane = el("owo-goal-detail-pane-" + name);
      if (button && button.setAttribute) { button.setAttribute("aria-selected",String(detailTab === name)); button.setAttribute("tabindex",detailTab === name ? "0" : "-1"); }
      if (pane) pane.hidden = detailTab !== name;
    });
  }
  function bindTabs(names, prefix, callback) {
    names.forEach(function (name, index) {
      var button = el(prefix + name);
      if (!button || !button.addEventListener) return;
      button.addEventListener("click",function () {
        callback(name);
        if (prefix === "owo-goal-detail-tab-" && name === "audit" && state.current) showAudit(state.current);
      });
      button.addEventListener("keydown",function (event) {
        var next = event.key === "ArrowRight" ? (index+1)%names.length : event.key === "ArrowLeft" ? (index+names.length-1)%names.length : event.key === "Home" ? 0 : event.key === "End" ? names.length-1 : -1;
        if (next < 0) return;
        event.preventDefault(); callback(names[next]);
        if (prefix === "owo-goal-detail-tab-" && names[next] === "audit" && state.current) showAudit(state.current);
        var target=el(prefix+names[next]); if (target && target.focus) target.focus();
      });
    });
  }
  function capturePlanDraft() {
    var textarea = el("owo-goal-steps");
    if (renderedGoalId && textarea && panelMounted) planDrafts[renderedGoalId] = textarea.value;
  }
  function showEditor(open, restoring) {
    editorOpen = !!open;
    var box = el("owo-goal-editor"), button = el("owo-goal-new");
    if (box) box.hidden = !editorOpen;
    if (button && button.setAttribute) button.setAttribute("aria-expanded",String(editorOpen));
    if (open && !restoring) { selectTab("goals"); var input=el("owo-goal-objective"); if(input && input.focus) input.focus({preventScroll:true}); }
  }
  function serializableSteps(plan) {
    return (plan && Array.isArray(plan.steps) ? plan.steps : []).map(function (step) {
      var verify = step.verify;
      if (verify === "OutputNonEmpty" || (verify && typeof verify === "object" && Object.prototype.hasOwnProperty.call(verify,"OutputNonEmpty"))) verify = {kind:"nonempty"};
      else if (verify && typeof verify === "object" && Object.prototype.hasOwnProperty.call(verify,"OutputEquals")) verify = {kind:"equals",value:verify.OutputEquals};
      else if (verify && typeof verify === "object" && Object.prototype.hasOwnProperty.call(verify,"OutputContains")) verify = {kind:"contains",value:verify.OutputContains};
      return {id:step.id,worker:step.worker,deps:step.depends_on || step.deps || [],verify:verify == null ? null : verify,max_retries:step.retries == null ? (step.max_retries || 0) : step.retries,input:step.input === undefined ? {} : step.input,parallel:!!step.parallel};
    });
  }

  function nav() {
    return '<section data-panel="goal" data-layout="custom" class="owo-goal">' +
      '<div class="owo-goal-toolbar"><div role="tablist" aria-label="目标分区">' +
      '<button type="button" role="tab" id="owo-goal-tab-goals" aria-controls="owo-goal-pane-goals">目标</button>' +
      '<button type="button" role="tab" id="owo-goal-tab-cloud" aria-controls="owo-goal-pane-cloud">云端进度</button></div>' +
      '<div class="owo-goal-toolbar-actions"><button type="button" id="owo-goal-refresh">刷新</button>' +
      '<button type="button" class="primary" id="owo-goal-new" aria-expanded="false">新建目标</button></div></div>' +
      '<div role="tabpanel" id="owo-goal-pane-goals" aria-labelledby="owo-goal-tab-goals">' +
      '<div class="owo-goal-editor" id="owo-goal-editor" hidden><label for="owo-goal-objective">目标描述</label>' +
      '<textarea id="owo-goal-objective" rows="3" placeholder="描述要完成的目标"></textarea>' +
      '<div class="owo-goal-editor-actions"><button type="button" id="owo-goal-cancel">收起</button>' +
      '<button type="button" class="primary" id="owo-goal-create" data-core-action>创建目标</button></div></div>' +
      '<div class="owo-goal-filters"><input type="search" id="owo-goal-search" aria-label="搜索目标" placeholder="搜索目标">' +
      '<select id="owo-goal-filter" aria-label="目标状态"><option value="">全部状态</option>' +
      '<option value="Pending">未运行</option><option value="Planning">计划中</option><option value="Running">运行中</option>' +
      '<option value="Verifying">校验中</option><option value="Succeeded">已完成</option><option value="Failed">失败</option><option value="Aborted">已中止</option></select></div>' +
      '<div id="owo-goal-list" class="owo-goal-list" aria-live="polite">正在加载目标…</div>' +
      '<div id="owo-goal-detail" class="owo-goal-detail"></div></div>' +
      '<div role="tabpanel" id="owo-goal-pane-cloud" aria-labelledby="owo-goal-tab-cloud" hidden>' +
      '<label for="owo-goal-cloud-task">云端任务编号</label><div class="owo-goal-cloud-controls">' +
      '<input id="owo-goal-cloud-task" placeholder="如 cloud-0001"><button type="button" id="owo-goal-cloud-sub" data-core-action>订阅</button>' +
      '<button type="button" id="owo-goal-cloud-close">断开</button></div>' +
      '<pre class="owo-goal-cloudlog" id="owo-goal-cloudlog" role="log">输入云端任务编号以查看实时进度。</pre></div></section>';
  }

  function mount(root, helpers) {
    dispose();
    state.cloudLog = [];
    cloudReconnectNoticeShown = false;
    if (helpers) H = helpers;
    var scope = String(H.baseUrl || "") + "#" + String(H.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
    if (hostScope && hostScope !== scope) {
      state.current = null; state.goals = []; state.status = null; runtimeStatuses = Object.create(null);
      planDrafts = Object.create(null); renderedGoalId = null; goalDraft = ""; editorOpen = false;
      creatingGoal = false; savingPlans = Object.create(null); goalActionPending = Object.create(null);
    }
    hostScope = scope;
    root.innerHTML = nav();
    panelMounted = true;
    var createButton = root.querySelector("#owo-goal-create");
    if (creatingGoal) {
      activeCreateButton = createButton;
      setCreateButtonBusy(createButton, true);
    }
    createButton.addEventListener("click", createGoal);
    root.querySelector("#owo-goal-cloud-sub").addEventListener("click", subscribeCloud);
    root.querySelector("#owo-goal-cloud-close").addEventListener("click", closeCloud);
    function bind(id, event, callback) { var node=root.querySelector("#"+id); if(node && node.addEventListener) node.addEventListener(event,callback); }
    bind("owo-goal-new","click",function(){showEditor(true);});
    bind("owo-goal-cancel","click",function(){showEditor(false);});
    bind("owo-goal-refresh","click",refresh);
    bind("owo-goal-objective","input",function(event){goalDraft=event.target.value;});
    bind("owo-goal-search","input",function(event){goalQuery=event.target.value;renderList();});
    bind("owo-goal-filter","change",function(event){goalFilter=event.target.value;renderList();});
    var objective=root.querySelector("#owo-goal-objective"); if (objective && !objective.value && goalDraft) objective.value=goalDraft;
    var search=root.querySelector("#owo-goal-search"); if(search) search.value=goalQuery;
    var filter=root.querySelector("#owo-goal-filter"); if(filter) filter.value=goalFilter;
    bindTabs(["goals","cloud"],"owo-goal-tab-",selectTab);
    selectTab(activeTab);
    showEditor(editorOpen,true);
    refresh();
  }

  function refresh() {
    listLoading=true;listError=null;renderList();
    var request = ++listGeneration;
    var owner = panelGeneration;
    var selectedAtRequest = state.current;
    return H.get("/goal")
      .then(function (data) {
        if (request !== listGeneration || owner !== panelGeneration) return;
        listLoading=false;
        if (!data || !Array.isArray(data.goals)) throw new Error("目标列表响应格式不完整");
        state.goals = data.goals.slice().sort(function(a,b){return String(b.updated_at || b.created_at || "").localeCompare(String(a.updated_at || a.created_at || ""));});
        renderList();
        if (state.current && state.current === selectedAtRequest) return loadGoal(state.current);
      })
      .catch(function (e) {
        if (request !== listGeneration || owner !== panelGeneration) return;
        listLoading=false;listError=H.friendlyError(e);renderList();
      });
  }

  function renderList() {
    var list = el("owo-goal-list");
    if (!list) return;
    if (listLoading) { list.innerHTML='<p class="sub" role="status">正在加载目标…</p>';return; }
    if (listError) {
      list.innerHTML='<p class="owo-goal-error" role="alert">'+H.esc(listError)+'</p><button type="button" class="owo-goal-retry">重试</button>';
      var retry=list.querySelector && list.querySelector(".owo-goal-retry");if(retry)retry.addEventListener("click",refresh);
      return;
    }
    var query = goalQuery.trim().toLowerCase();
    var goals = state.goals.filter(function(g){return (!goalFilter || goalStatus(g)===goalFilter) && (!query || String(g.objective || "").toLowerCase().indexOf(query)!==-1 || String(g.id).toLowerCase().indexOf(query)!==-1);});
    if (!goals.length) { list.innerHTML='<p class="sub">'+(state.goals.length?"没有匹配的目标。":"暂无目标，先创建一个。")+'</p>';return; }
    list.innerHTML=goals.map(function(g){
      return '<div class="owo-goal-row'+(state.current===g.id?' is-selected':'')+'"><div><strong>'+H.esc(g.objective || g.id)+'</strong>' +
        '<span class="owo-goal-state">'+H.esc(label(goalStatus(g)))+'</span><p class="sub">'+H.esc(localTime(g.updated_at || g.created_at))+'（本地时间）</p></div>' +
        '<button type="button" data-id="'+H.esc(g.id)+'" class="owo-goal-open" aria-label="打开目标：'+H.esc(g.objective || g.id)+'">打开</button></div>';
    }).join("");
    if (list.querySelectorAll) Array.prototype.forEach.call(list.querySelectorAll(".owo-goal-open"),function(button){
      button.addEventListener("click",function(){loadGoal(button.getAttribute("data-id"));renderList();});
    });
  }

  function setCreateButtonBusy(button, busy) {
    if (!button) return;
    setBusy(button,busy,busy ? "正在创建…" : "创建目标");
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function createGoal() {
    if (creatingGoal) return Promise.resolve(null);
    var input = document.getElementById("owo-goal-objective");
    var objective = (input && input.value.trim()) || "";
    if (!objective) { notify("请填写目标描述","error"); return Promise.resolve(null); }
    var owner = panelGeneration;
    var scope = hostScope;
    var transport = H.post;
    creatingGoal = true;
    activeCreateButton = document.getElementById("owo-goal-create");
    setCreateButtonBusy(activeCreateButton, true);
    return Promise.resolve()
      .then(function () {
        if (owner !== panelGeneration) return null;
        return transport("/goal", { objective: objective });
      })
      .then(function (data) {
        if (owner !== panelGeneration) return null;
        if (!data || !data.goal || !data.goal.id) throw new Error("创建响应缺少目标编号");
        if (input && input.value.trim() === objective) { input.value=""; goalDraft=""; showEditor(false); }
        state.current = data.goal.id;
        return refresh().then(function(){return data;});
      })
      .catch(function (e) {
        if (owner === panelGeneration) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        if (scope !== hostScope) return;
        creatingGoal = false;
        var currentButton = document.getElementById("owo-goal-create");
        setCreateButtonBusy(activeCreateButton, false);
        if (currentButton !== activeCreateButton) setCreateButtonBusy(currentButton, false);
        activeCreateButton = null;
      });
  }

  function loadGoal(goalId) {
    capturePlanDraft();
    state.current = goalId;
    var request = ++detailGeneration;
    var owner = panelGeneration;
    function isCurrent() {
      return request === detailGeneration && owner === panelGeneration && state.current === goalId;
    }
    return H.get("/goal/" + encodeURIComponent(goalId))
      .then(function (goal) {
        if (!isCurrent()) return;
        return H.get("/goal/" + encodeURIComponent(goalId) + "/plan")
          .then(function (planData) {
            if (isCurrent()) renderDetail(goal, planData);
          })
          .catch(function (error) {
            if (isCurrent()) renderDetail(goal, null, isMissing(error) ? null : error);
          });
      })
      .catch(function (e) {
        if (!isCurrent()) return;
        var detail = document.getElementById("owo-goal-detail");
        if (detail) detail.innerHTML = '<div class="owo-goal-badge bad">' + H.esc(H.friendlyError(e)) + "</div>";
      });
  }

  function renderWaves(waves) {
    if (!Array.isArray(waves)) return "";
    return waves.map(function (wave, index) {
      var steps = Array.isArray(wave) ? wave : [];
      return "wave" + (index + 1) + ": " +
        steps.map(function (step) { return H.esc(step == null ? "" : step); }).join(", ");
    }).join("<br>");
  }

  function renderDetail(goal, planData, planError) {
    var detail=el("owo-goal-detail");
    if(!detail) return;
    var plan=planData && planData.plan;
    detailPlanError=!!planError;
    detailPlanPresent=!!plan;
    var raw=Object.prototype.hasOwnProperty.call(planDrafts,goal.id)?planDrafts[goal.id]:JSON.stringify(serializableSteps(plan),null,2);
    renderedGoalId=goal.id;
    var pending=goalActionPending[goal.id], running=["Running","Verifying"].indexOf(goalStatus(goal))!==-1;
    detail.innerHTML='<div class="owo-goal-detail-head"><h3>'+H.esc(goal.objective)+'</h3><span class="owo-goal-state" id="owo-goal-current-status">'+H.esc(label(goalStatus(goal)))+'</span></div>' +
      '<div role="tablist" aria-label="目标详情" class="owo-goal-detail-tabs">' +
      ["plan","status","audit"].map(function(name,index){return '<button type="button" role="tab" id="owo-goal-detail-tab-'+name+'" aria-controls="owo-goal-detail-pane-'+name+'">'+["执行计划","执行状态","审计记录"][index]+'</button>';}).join("")+'</div>' +
      '<div role="tabpanel" id="owo-goal-detail-pane-plan" aria-labelledby="owo-goal-detail-tab-plan">' +
      (planError?'<p class="owo-goal-error" role="alert">计划读取失败：'+H.esc(H.friendlyError(planError))+'。请刷新后再编辑。</p>':'') +
      '<label for="owo-goal-steps">步骤定义（JSON）</label><textarea class="owo-goal-steps" id="owo-goal-steps" spellcheck="false"'+(planError?' disabled':'')+'>'+H.esc(raw)+'</textarea>' +
      '<h4>执行批次</h4><div class="owo-goal-waves" id="owo-goal-waves">'+(plan?renderWaves(planData.waves):"暂无计划")+'</div>' +
      '<div class="owo-goal-editor-actions"><button type="button" class="primary" id="owo-goal-save-plan" data-core-action'+(planError?' disabled':savingPlans[goal.id]?' disabled data-core-busy="true" aria-busy="true"':'')+'>保存计划</button>' +
      '<button type="button" id="owo-goal-run-now" data-core-action'+(pending || !plan || planError || running?' disabled':'')+'>运行</button>' +
      '<button type="button" id="owo-goal-abort-now" data-core-action'+(pending || !running?' disabled':'')+'>中止</button></div></div>' +
      '<div role="tabpanel" id="owo-goal-detail-pane-status" aria-labelledby="owo-goal-detail-tab-status" hidden>' +
      '<div class="owo-goal-section-actions"><button type="button" id="owo-goal-poll">刷新状态</button></div>' +
      '<div class="owo-goal-table-wrap" tabindex="0" role="region" aria-label="步骤执行状态，可横向滚动"><table class="owo-goal-table" id="owo-goal-status"><tr><td>正在读取状态…</td></tr></table></div></div>' +
      '<div role="tabpanel" id="owo-goal-detail-pane-audit" aria-labelledby="owo-goal-detail-tab-audit" hidden>' +
      '<div class="owo-goal-section-actions"><button type="button" id="owo-goal-audit">刷新审计记录</button></div>' +
      '<div id="owo-goal-audit-box">正在读取审计记录…</div></div>';
    function bind(id,fn){var button=detail.querySelector("#"+id);if(button && button.addEventListener)button.addEventListener("click",fn);}
    bind("owo-goal-save-plan",function(){savePlan(goal.id);});
    bind("owo-goal-run-now",function(){runGoal(goal.id,el("owo-goal-run-now"),"运行");});
    bind("owo-goal-abort-now",function(){abortGoal(goal.id,el("owo-goal-abort-now"),"中止");});
    bind("owo-goal-poll",function(){pollStatus(goal.id);});
    bind("owo-goal-audit",function(){showAudit(goal.id);});
    var textarea=detail.querySelector("#owo-goal-steps");
    if(textarea && textarea.addEventListener)textarea.addEventListener("input",function(){planDrafts[goal.id]=textarea.value;});
    bindTabs(["plan","status","audit"],"owo-goal-detail-tab-",selectDetailTab);
    selectDetailTab(detailTab);
    pollStatus(goal.id);
    if (detailTab === "audit") showAudit(goal.id);
  }

  function setPlanSaveButtonBusy(button, busy) {
    if (!button) return;
    setBusy(button,busy,busy ? "正在保存…" : "保存计划");
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function savePlan(goalId) {
    if (savingPlans[goalId] || state.current !== goalId || detailPlanError) return Promise.resolve(null);
    var textarea = document.getElementById("owo-goal-steps");
    var saveButton = document.getElementById("owo-goal-save-plan");
    if (!textarea) return Promise.resolve(null);
    var steps;
    try {
      steps = JSON.parse(textarea.value);
    } catch (e) {
      notify("steps JSON 非法：" + e.message, "error");
      return Promise.resolve(null);
    }
    if (!Array.isArray(steps)) {
      notify("steps 必须是 JSON 数组", "error");
      return Promise.resolve(null);
    }
    if (!steps.length || steps.some(function(step){return !step || typeof step!=="object" || typeof step.id!=="string" || !step.id.trim() || typeof step.worker!=="string" || !step.worker.trim() || (step.deps!==undefined && (!Array.isArray(step.deps) || step.deps.some(function(dep){return typeof dep!=="string";}))) || (step.max_retries!==undefined && (!Number.isInteger(step.max_retries) || step.max_retries<0));})) {
      notify("步骤需包含 id、worker，依赖须为字符串数组，重试次数须为非负整数","error");
      return Promise.resolve(null);
    }
    var normalized = steps.map(function(step){
      return {id:step.id,worker:step.worker,deps:step.deps || [],verify:step.verify == null ? null : step.verify,max_retries:step.max_retries || 0,input:step.input === undefined ? {} : step.input,parallel:!!step.parallel};
    });
    var submitted=textarea.value;
    var scope=hostScope, transport=H.post, locks=savingPlans;
    var owner = panelGeneration;
    savingPlans[goalId] = true;
    setPlanSaveButtonBusy(saveButton, true);
    return Promise.resolve()
      .then(function () {
        if (owner !== panelGeneration) return null;
        return transport("/goal/" + encodeURIComponent(goalId) + "/plan", { steps: normalized });
      })
      .then(function (data) {
        if (owner !== panelGeneration || state.current !== goalId) return null;
        if (textarea.value === submitted) delete planDrafts[goalId];
        else planDrafts[goalId]=textarea.value;
        // Do not recapture the submitted textarea when applying the canonical saved plan.
        renderedGoalId=null;
        return loadGoal(goalId).then(function(){return data;});
      })
      .catch(function (e) {
        if (owner === panelGeneration && state.current === goalId) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        delete locks[goalId];
        if (scope !== hostScope) return;
        setPlanSaveButtonBusy(saveButton, false);
        if (owner === panelGeneration && state.current === goalId) {
          setPlanSaveButtonBusy(document.getElementById("owo-goal-save-plan"), false);
        }
      });
  }

  function setGoalActionButtonBusy(button, busy, idleText, busyText) {
    if (!button) return;
    setBusy(button,busy,busy ? busyText : idleText);
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function runGoal(goalId, button, idleText) {
    return startGoalAction(goalId, "run", button, idleText || "运行", "正在运行…");
  }

  function abortGoal(goalId, button, idleText) {
    return startGoalAction(goalId, "abort", button, idleText || "中止", "正在中止…");
  }

  function startGoalAction(goalId, action, button, idleText, busyText) {
    if (goalActionPending[goalId]) return Promise.resolve(null);
    var owner = panelGeneration, scope=hostScope, locks=goalActionPending, transport=H.post;
    goalActionPending[goalId] = action;
    setGoalActionButtonBusy(button, true, idleText, busyText);
    var path = "/goal/" + encodeURIComponent(goalId) + (action === "run" ? "/run" : "/abort");
    var body = action === "run" ? { parallelism: 2, allow_replan: true } : {};
    return Promise.resolve()
      .then(function () {
        if (owner !== panelGeneration) return null;
        return transport(path, body);
      })
      .then(function (result) {
        if (owner !== panelGeneration) return null;
        return result;
      })
      .catch(function (e) {
        if (owner === panelGeneration) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        delete locks[goalId];
        if (scope !== hostScope) return;
        setGoalActionButtonBusy(button, false, idleText, busyText);
        if (panelMounted) refresh();
      });
  }

  function pollStatus(goalId) {
    var request=++statusGeneration, owner=panelGeneration;
    return H.get("/goal/"+encodeURIComponent(goalId)+"/status").then(function(status){
      if(request!==statusGeneration || owner!==panelGeneration || state.current!==goalId)return;
      if(!status || typeof status!=="object")throw new Error("运行状态响应格式不完整");
      state.status=status;
      if (typeof status.goal_status === "string") {
        runtimeStatuses[goalId]=status.goal_status;
        renderList();
        var heading=el("owo-goal-current-status");if(heading)heading.textContent=label(status.goal_status);
        var running=["Running","Verifying"].indexOf(status.goal_status)!==-1;
        function disabled(id,value) {
          var button=el(id);if(!button)return;
          if(window.OwoCoreActionAvailability && window.OwoCoreActionAvailability.setDisabled) window.OwoCoreActionAvailability.setDisabled(button,value);
          else button.disabled=value;
        }
        disabled("owo-goal-run-now",!!goalActionPending[goalId] || !detailPlanPresent || detailPlanError || running);
        disabled("owo-goal-abort-now",!!goalActionPending[goalId] || !running);
      }
      var table=el("owo-goal-status");if(!table)return;
      var steps;
      if(status.steps!==undefined) {
        if(!Array.isArray(status.steps))throw new Error("步骤状态响应格式不完整");
        steps=status.steps;
      } else {
        var records=status.records || {};
        steps=Object.keys(records).map(function(id){var record=Object.assign({},records[id]);record.step_id=id;return record;});
      }
      function text(value){return typeof value==="string"?value:value==null?"":JSON.stringify(value,null,2);}
      var rows=steps.map(function(step){
        var output=text(step.output), error=text(step.error);
        return "<tr><td>"+H.esc(step.step_id)+"</td><td>"+H.esc(label(step.status))+"</td><td>"+H.esc(step.worker || "—")+
          (step.model?"<br>"+H.esc(step.model):"")+"</td><td>"+H.esc(step.attempts == null?"—":step.attempts)+"</td><td>"+
          (output?'<pre class="owo-goal-output">'+H.esc(output)+'</pre>':"—")+
          (step.output_truncated?'<p class="sub">核心仅返回输出前 2000 字符。</p>':"")+
          (error?'<pre class="owo-goal-error">'+H.esc(error)+'</pre>':"")+"</td></tr>";
      }).join("");
      table.innerHTML="<thead><tr><th>步骤</th><th>状态</th><th>工具 / 模型</th><th>尝试</th><th>输出</th></tr></thead><tbody>"+
        (rows || '<tr><td colspan="5">暂无步骤执行记录。</td></tr>')+
        '<tr><td colspan="5">'+H.esc(label(status.goal_status))+" · 执行步骤 "+H.esc(status.steps_taken == null?"—":status.steps_taken)+" · 重新规划 "+H.esc(status.replan_count == null?"—":status.replan_count)+"</td></tr></tbody>";
    }).catch(function(error){
      if(request!==statusGeneration || owner!==panelGeneration || state.current!==goalId)return;
      var table=el("owo-goal-status");if(!table)return;
      table.innerHTML='<tbody><tr><td colspan="5"'+(isMissing(error)?'':' class="owo-goal-error"')+'>'+
        (isMissing(error)?"尚未运行。":"状态读取失败："+H.esc(H.friendlyError(error))+"，可刷新状态重试。")+"</td></tr></tbody>";
    });
  }

  function showAudit(goalId) {
    var request=++auditGeneration, owner=panelGeneration;
    return H.get("/goal/"+encodeURIComponent(goalId)+"/audit").then(function(data){
      if(request!==auditGeneration || owner!==panelGeneration || state.current!==goalId)return;
      if(!data || !Array.isArray(data.audit))throw new Error("审计响应格式不完整");
      var box=el("owo-goal-audit-box");if(!box)return;
      box.innerHTML='<p class="sub">核心最近 50 条审计记录，包含其它目标的事件。</p>'+
        (data.audit.length?'<ol class="owo-goal-audit-list">'+data.audit.map(function(event){
          return '<li><strong>'+H.esc(event.event || "未知事件")+'</strong><span class="sub">'+H.esc(localTime(event.ts || event.timestamp || event.at))+'</span><p>'+H.esc(event.detail || "")+'</p></li>';
        }).join("")+'</ol>':'<p class="sub">暂无审计记录。</p>');
    }).catch(function(error){
      if(request!==auditGeneration || owner!==panelGeneration || state.current!==goalId)return;
      var box=el("owo-goal-audit-box");if(box)box.innerHTML='<p class="owo-goal-error" role="alert">'+H.esc(H.friendlyError(error))+'</p>';
    });
  }

  var cloudSource = null;
  var cloudGeneration = 0;
  var cloudReconnectNoticeShown = false;

  function subscribeCloud() {
    var input = document.getElementById("owo-goal-cloud-task");
    var taskId = (input && input.value.trim()) || "";
    if (!taskId) return;
    closeCloud();
    var base = H.baseUrl || window.location.origin;
    var log = document.getElementById("owo-goal-cloudlog");
    if (log) log.textContent = "订阅 " + taskId + " ...";
    cloudReconnectNoticeShown = false;
    var generation = cloudGeneration;
    cloudSource = new EventSource(base + "/cloud/tasks/" + encodeURIComponent(taskId) + "/events");
    cloudSource.onopen = function () {
      if (generation !== cloudGeneration) return;
      cloudReconnectNoticeShown = false;
    };
    cloudSource.onmessage = function (event) {
      if (generation === cloudGeneration) onCloudProgress(event);
    };
    // The server names cloud progress frames `progress`; EventSource.onmessage only
    // receives unnamed `message` events, so subscribe to both wire forms.
    cloudSource.addEventListener("progress", function (event) {
      if (generation === cloudGeneration) onCloudProgress(event);
    });
    cloudSource.onerror = function () {
      if (generation !== cloudGeneration) return;
      // EventSource tracks Last-Event-ID and retries transient disconnects itself.
      // Only explicit user action or a terminal/not-found frame closes this source.
      if (!cloudReconnectNoticeShown) {
        appendCloudLog("（连接暂时中断，正在自动续传进度）");
        cloudReconnectNoticeShown = true;
      }
    };
  }

  function onCloudProgress(ev) {
    var raw = ev && ev.data != null ? String(ev.data) : "";
    try {
      var payload = JSON.parse(raw);
      if (payload && payload.kind === "stream_gap") {
        appendCloudLog(payload.message || "实时进度出现丢帧，正在续传保留的历史。");
        return;
      }
      if (payload && ["task_not_found", "stream_complete"].includes(payload.kind)) {
        appendCloudLog(payload.message || "任务已结束或没有找到该任务。");
        closeCloudSilently();
        return;
      }
      if (payload && ["succeeded", "failed", "canceled"].includes(payload.kind || payload.event)) {
        appendCloudLog(raw);
        closeCloudSilently();
        return;
      }
    } catch (_) {
      // Non-JSON compatibility messages remain visible as received.
    }
    appendCloudLog(raw);
  }

  function closeCloudSilently() {
    cloudGeneration += 1;
    var source = cloudSource;
    cloudSource = null;
    if (source) source.close();
  }

  function dispose() {
    capturePlanDraft();
    var input=el("owo-goal-objective");
    if(panelMounted && input)goalDraft=input.value;
    panelMounted = false;
    panelGeneration += 1;
    listGeneration += 1;
    detailGeneration += 1;
    statusGeneration += 1;
    auditGeneration += 1;
    closeCloudSilently();
  }

  function closeCloud() {
    closeCloudSilently();
    appendCloudLog("（已断开）");
  }

  function appendCloudLog(line) {
    var log = document.getElementById("owo-goal-cloudlog");
    if (!log) return;
    state.cloudLog.push(String(line));
    if (state.cloudLog.length > 200) state.cloudLog.shift();
    log.textContent = state.cloudLog.join("\n");
    log.scrollTop = log.scrollHeight;
  }

  return {
    id: id,
    title: "编排（Goal/Plan + 云端进度）",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { serializableSteps: serializableSteps, selectTab: selectTab, selectDetailTab: selectDetailTab, loadGoal: loadGoal, pollStatus: pollStatus, showAudit: showAudit, createGoal: createGoal, savePlan: savePlan, runGoal: runGoal, abortGoal: abortGoal, renderWaves: renderWaves },
  };
})();
