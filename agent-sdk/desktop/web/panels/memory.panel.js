/* Lane 4 记忆面板：记忆图谱（第二大脑 · 子任务 1）。
 * 纯脚本 IIFE，注册 window.OwoPanels.memory。防御性降级。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.memory = (function () {
  "use strict";

  var id = "memory";
  var H = null;
  var rootEl = null;
  var mounted = false;
  var hostScope = "";
  var activeTab = "search";
  var draft = Object.create(null);
  var mineOwner = null;
  var relationOwner = null;
  var fields = ["recall","app","from","to","rel-a","rel-b","rel-r","mine-name","mine-apps","mine-desc","mine-sensitivity"];
  function el(id) { return rootEl && rootEl.querySelector("#" + id); }
  function localTime(value) {
    if (!value) return "时间未知";
    var date = new Date(value);
    return Number.isNaN(date.getTime()) ? "时间未知" : date.toLocaleString("zh-CN", {hour12:false}) + "（本地时间）";
  }
  function busy(button, value) {
    if (!button) return;
    if (button.setAttribute) {
      button.setAttribute("data-core-busy",value ? "true" : "false");
      button.setAttribute("aria-busy",value ? "true" : "false");
    }
    if (window.OwoCoreActionAvailability && window.OwoCoreActionAvailability.setBusy) window.OwoCoreActionAvailability.setBusy(button,value);
    else button.disabled=!!value;
  }
  function selectTab(value) {
    var names=["search","timeline","relations","mine"];
    activeTab=names.indexOf(value)<0 ? "search" : value;
    names.forEach(function(name) {
      var button=el("owo-memory-tab-"+name), pane=el("owo-memory-pane-"+name);
      if(button && button.setAttribute) {
        button.setAttribute("aria-selected",String(name===activeTab));
        button.setAttribute("tabindex",name===activeTab ? "0" : "-1");
      }
      if(pane) pane.hidden=name!==activeTab;
    });
  }
  function captureDraft() {
    if(!mounted)return;
    fields.forEach(function(name){var node=el("owo-memory-"+name);if(node)draft[name]=node.value;});
  }
  function arrayData(data,key) {
    if(data && data.error)throw new Error(typeof data.error==="string"?data.error:"记忆读取失败");
    if(!data || !Array.isArray(data[key]))throw new Error("响应格式不完整，请刷新重试");
    return data[key];
  }
  function searchIcon(className) { return '<svg class="'+className+'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4.5 4.5"/></svg>'; }
  function empty(message, search) { return '<p class="owo-insight-empty'+(search?' owo-memory-search-empty':'')+'" role="status">'+(search?searchIcon("owo-memory-empty-icon"):"")+'<span>'+H.esc(message)+'</span></p>'; }
  function entryHtml(entry,hit) {
    if(!entry || typeof entry!=="object")throw new Error("记忆条目格式不完整");
    return '<div class="owo-memory-entry"><span class="owo-memory-app">'+H.esc(entry.app_id || "未知应用")+'</span>'+
      '<time>'+H.esc(localTime(entry.ts))+'</time><div><p>'+H.esc(entry.summary || "无摘要")+'</p>'+
      (hit && Array.isArray(entry.matched_entities) && entry.matched_entities.length ? '<p class="sub">实体命中：'+entry.matched_entities.map(H.esc).join("、")+'</p>' : "")+'</div></div>';
  }
  var panelGeneration = 0;
  var timelineGeneration = 0;
  var entitiesGeneration = 0;
  var relationsGeneration = 0;
  var entriesGeneration = 0;
  var recallGeneration = 0;
  var mineGeneration = 0;
  var addRelationGeneration = 0;
  var mining = false;
  var addingRelation = false;
  function notify(message, kind) {
    if (H && H.notify) H.notify(message, kind || "error");
    else if (window.OwoToast) window.OwoToast(message);
    else window.alert(message);
  }

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) return r.json().catch(function () { return {}; }).then(function (body) {
          throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
        });
        return r.status === 204 ? null : r.json();
      });
    }
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) {
          return r.json().catch(function () { return {}; }).then(function (body) {
            throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
          });
        }
        return r.status === 204 ? null : r.json();
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

  function nav() {
    var names=["search","timeline","relations","mine"], labels=["检索","时间线","实体与关系","技能挖掘"];
    function input(name,label,placeholder,type) {
      return '<label>'+label+'<input id="owo-memory-'+name+'" aria-label="'+label+'" type="'+(type || "text")+'" placeholder="'+(placeholder || "")+'"></label>';
    }
    return '<section data-panel="memory" data-layout="custom" class="owo-memory owo-insight">'+
      '<div class="owo-insight-toolbar"><div role="tablist" aria-label="记忆分区">'+names.map(function(name,index){
        return '<button type="button" role="tab" id="owo-memory-tab-'+name+'" aria-controls="owo-memory-pane-'+name+'">'+labels[index]+'</button>';
      }).join("")+'</div><button type="button" id="owo-memory-refresh">刷新</button></div>'+
      '<div role="tabpanel" id="owo-memory-pane-search" aria-labelledby="owo-memory-tab-search">'+
      '<div class="owo-memory-search"><input type="search" id="owo-memory-recall" aria-label="检索记忆" placeholder="输入关键词查找记忆">'+searchIcon("owo-memory-input-icon")+
      '<button type="button" class="primary" id="owo-memory-recall-btn">检索</button></div><p class="sub">搜索结果</p>'+
      '<div id="owo-memory-recall-box" aria-live="polite">'+empty("输入关键词，检索相关记忆。",true)+'</div>'+
      '<section class="owo-insight-section"><h3>记忆条目</h3><div class="owo-memory-filters">'+
      input("app","应用过滤","所有应用")+input("from","开始日期","","date")+input("to","结束日期","","date")+
      '<button type="button" id="owo-memory-filter">筛选</button></div><div id="owo-memory-entries" aria-live="polite">正在加载条目…</div>'+
      '<p class="sub">最多显示 50 条。日期筛选按 UTC，条目显示本地时间。</p></section></div>'+
      '<div role="tabpanel" id="owo-memory-pane-timeline" aria-labelledby="owo-memory-tab-timeline" hidden><h3>时间线</h3>'+
      '<div id="owo-memory-timeline" aria-live="polite">正在加载时间线…</div><p class="sub">按 UTC 日期汇总记忆条目。</p></div>'+
      '<div role="tabpanel" id="owo-memory-pane-relations" aria-labelledby="owo-memory-tab-relations" hidden>'+
      '<h3>实体</h3><p class="sub">词元频次与共现，仅反映当前记忆中的关联。</p><div id="owo-memory-entities" aria-live="polite">正在加载实体…</div>'+
      '<section class="owo-insight-section"><h3>手动关系</h3><div class="owo-memory-relation-form">'+input("rel-a","实体 A","实体名称")+
      input("rel-r","关系","如：关联")+input("rel-b","实体 B","实体名称")+
      '<button type="button" id="owo-memory-rel-add" data-core-action>添加</button></div><div id="owo-memory-relations" aria-live="polite">正在加载关系…</div></section></div>'+
      '<div role="tabpanel" id="owo-memory-pane-mine" aria-labelledby="owo-memory-tab-mine" hidden><h3>从情景记忆挖掘技能包</h3>'+
      '<p class="sub">观察动作序列 → 泛化 → 沉淀；需先有观察样本。生成后可在共享技能中查看与导出。</p>'+
      '<div class="owo-memory-mine-form">'+input("mine-name","技能名","如 send-file")+input("mine-apps","目标应用","逗号分隔，如 qq")+
      input("mine-desc","描述","可选")+'<label>敏感度<select id="owo-memory-mine-sensitivity" aria-label="敏感度"><option value="low">低敏感</option><option value="medium">中敏感</option><option value="high">高敏感</option></select></label></div>'+
      '<div class="owo-insight-actions"><button type="button" class="primary" id="owo-memory-mine-btn" data-core-action>挖掘</button></div>'+
      '<div id="owo-memory-mine-result" role="status"></div></div></section>';
  }

  function mount(root, helpers) {
    dispose();
    H = helpers || defaultHelpers();
    var scope=String(H.baseUrl || "")+"#"+String(H.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
    if(hostScope && scope!==hostScope) {
      draft=Object.create(null);activeTab="search";mining=false;addingRelation=false;mineOwner=null;relationOwner=null;
    }
    hostScope=scope;rootEl=root;root.innerHTML=nav();mounted=true;
    var owner=panelGeneration;
    function bind(name,event,fn) {
      var node=el("owo-memory-"+name);
      if(node)node.addEventListener(event,function(e){if(mounted && owner===panelGeneration)return fn(e);});
    }
    ["search","timeline","relations","mine"].forEach(function(name,index,names){
      bind("tab-"+name,"click",function(){selectTab(name);});
      bind("tab-"+name,"keydown",function(event) {
        var next=event.key==="ArrowRight"?(index+1)%names.length:event.key==="ArrowLeft"?(index+names.length-1)%names.length:event.key==="Home"?0:event.key==="End"?names.length-1:-1;
        if(next<0)return;event.preventDefault();selectTab(names[next]);var target=el("owo-memory-tab-"+names[next]);if(target.focus)target.focus();
      });
    });
    bind("recall-btn","click",doRecall);
    bind("recall","keydown",function(event){if(event.key==="Enter")doRecall();});
    bind("rel-add","click",addRelation);
    bind("refresh","click",refresh);
    bind("filter","click",loadEntries);
    bind("mine-btn","click",mineSkill);
    bind("mine-name","keydown",function(event){if(event.key==="Enter")mineSkill();});
    fields.forEach(function(name){var node=el("owo-memory-"+name);if(node && Object.prototype.hasOwnProperty.call(draft,name))node.value=draft[name];});
    busy(el("owo-memory-mine-btn"),mining);busy(el("owo-memory-rel-add"),addingRelation);
    selectTab(activeTab);refresh();
  }

  /// 从情景记忆挖掘技能包：观察动作序列 → 泛化 → 沉淀（服务端 /memory/mine-skill）。
  function mineSkill() {
    if (!mounted || mining) return Promise.resolve();
    var nameEl = el("owo-memory-mine-name");
    var name = (nameEl && nameEl.value.trim()) || "";
    var result = el("owo-memory-mine-result");
    if (!name) {
      if (result) result.textContent = "请先填写技能名";
      return;
    }
    var appsEl = el("owo-memory-mine-apps");
    var descEl = el("owo-memory-mine-desc");
    var sensEl = el("owo-memory-mine-sensitivity");
    var targetApps = ((appsEl && appsEl.value) || "")
      .split(",")
      .map(function (s) {
        return s.trim();
      })
      .filter(Boolean);
    mining = true;
    var mutationOwner = mineOwner = {}; var scope=hostScope;
    var request = ++mineGeneration;
    var owner = panelGeneration;
    var button = el("owo-memory-mine-btn");
    busy(button,true);
    if (result) result.textContent = "正在挖掘…";
    return H.post("/memory/mine-skill", {
      name: name,
      target_apps: targetApps,
      description: (descEl && descEl.value.trim()) || "",
      sensitivity: (sensEl && sensEl.value) || "low",
    })
      .then(function (data) {
        if (request !== mineGeneration || owner !== panelGeneration) return;
        var resultEl = el("owo-memory-mine-result"); // 面板已卸载则跳过
        if (!resultEl) return;
        var variables = arrayData(data,"variables");
        if(typeof data.name!=="string" || !data.name.trim())throw new Error("技能响应缺少名称，请在共享技能中确认结果");
        resultEl.innerHTML =
          "已生成技能包 <b>" + H.esc(data && data.name) + "</b>" +
          (variables.length ? "（变量 " + variables.length + " 个）" : "") +
          "，可在「共享技能」查看 / 导出 / 导入。";
      })
      .catch(function (e) {
        if (request !== mineGeneration || owner !== panelGeneration) return;
        var resultEl = el("owo-memory-mine-result"); // 面板已卸载则跳过
        if (resultEl) resultEl.textContent = H.friendlyError(e);
      })
      .finally(function () {
        if (mineOwner!==mutationOwner || scope!==hostScope) return;
        mineOwner=null;mining=false;
        if(mounted)busy(el("owo-memory-mine-btn"),false);
      });
  }

  function refresh() {
    if(!mounted)return Promise.resolve();
    return Promise.all([loadTimeline(),loadEntities(),loadRelations(),loadEntries()]);
  }

  function doRecall() {
    var request = ++recallGeneration;
    var owner = panelGeneration;
    var input = el("owo-memory-recall");
    var query = (input && input.value.trim()) || "";
    var box = el("owo-memory-recall-box");
    if (!query) {
      if(box)box.innerHTML=empty("输入关键词，检索相关记忆。",true);
      return Promise.resolve();
    }
    if (box) box.textContent = "正在检索…";
    return H.get("/memory/graph/recall?q=" + encodeURIComponent(query) + "&top_k=5")
      .then(function (data) {
        if (request !== recallGeneration || owner !== panelGeneration) return;
        var currentBox = el("owo-memory-recall-box");
        if (!currentBox) return;
        var hits=arrayData(data,"hits");
        currentBox.innerHTML='<p class="sub">命中 '+H.esc(hits.length)+' 条（最多 5 条）</p>'+
          (hits.length?hits.map(function(hit){return entryHtml(hit,true);}).join(""):empty("未找到相关记忆，请尝试其它关键词。"));
      })
      .catch(function (error) {
        if (request !== recallGeneration || owner !== panelGeneration) return;
        var currentBox = el("owo-memory-recall-box");
        if (currentBox) currentBox.innerHTML = '<div class="owo-memory-hit">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }
  function loadTimeline() {
    var request = ++timelineGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/timeline")
      .then(function (data) {
        if (request !== timelineGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-timeline");
        if (!current) return;
        var buckets=arrayData(data,"buckets");
        current.innerHTML=buckets.map(function(bucket){
          return '<div class="owo-memory-timeline-row"><strong>'+H.esc(bucket.day)+'</strong><span>'+H.esc(bucket.count)+' 条</span></div>';
        }).join("") || empty("暂无时间线记录");
      })
      .catch(function (error) {
        if (request !== timelineGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-timeline");
        if (current) current.innerHTML = '<div class="owo-memory-row">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }
  function loadEntities() {
    var request = ++entitiesGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/entities?limit=30")
      .then(function (data) {
        if (request !== entitiesGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-entities");
        if (!current) return;
        current.innerHTML = arrayData(data,"entities").map(function (entity) {
          var related = (entity.related || []).map(function (relation) {
            return H.esc(relation.entity) + "×" + H.esc(relation.count);
          }).join(", ");
          return '<span class="owo-memory-card"><b>' + H.esc(entity.entity) + "</b>×" + H.esc(entity.count) +
            (related ? " <small>(" + related + ")</small>" : "") + "</span>";
        }).join("") || empty("暂无实体记录");
      })
      .catch(function (error) {
        if (request !== entitiesGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-entities");
        if (current) current.innerHTML = '<div class="owo-memory-card">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }
  function loadRelations() {
    var request = ++relationsGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/links")
      .then(function (data) {
        if (request !== relationsGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-relations");
        if (!current) return;
        current.innerHTML = arrayData(data,"links").map(function (link) {
          return '<span class="owo-memory-rel">' + H.esc(link.a) + " —" + H.esc(link.relation) + "→ " + H.esc(link.b) + "</span>";
        }).join("") || empty("暂无手动关系");
      })
      .catch(function (error) {
        if (request !== relationsGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-relations");
        if (current) current.innerHTML = '<span class="owo-memory-rel">' + H.esc(H.friendlyError(error)) + "</span>";
      });
  }
  function addRelation() {
    if (!mounted || addingRelation) return Promise.resolve();
    var a = el("owo-memory-rel-a");
    var b = el("owo-memory-rel-b");
    var relation = el("owo-memory-rel-r");
    var values = { a: (a && a.value.trim()) || "", b: (b && b.value.trim()) || "", relation: (relation && relation.value.trim()) || "" };
    if (!values.a || !values.b || !values.relation) {
      notify("请填写实体 A、实体 B 和关系。", "error");
      return Promise.resolve();
    }
    addingRelation = true;
    var mutationOwner=relationOwner={};var scope=hostScope;
    var request = ++addRelationGeneration;
    var owner = panelGeneration;
    var button = el("owo-memory-rel-add");
    busy(button,true);
    return H.post("/memory/graph/link", values)
      .then(function () {
        if (request !== addRelationGeneration || owner !== panelGeneration) return;
        if(a.value.trim()===values.a)a.value="";
        if(b.value.trim()===values.b)b.value="";
        if(relation.value.trim()===values.relation)relation.value="";
        return loadRelations();
      })
      .catch(function (error) {
        if (request === addRelationGeneration && owner === panelGeneration) notify(H.friendlyError(error), "error");
      })
      .finally(function () {
        if(relationOwner!==mutationOwner || scope!==hostScope)return;
        relationOwner=null;addingRelation=false;
        if(mounted)busy(el("owo-memory-rel-add"),false);
      });
  }
  function loadEntries() {
    var request = ++entriesGeneration;
    var owner = panelGeneration;
    var app = el("owo-memory-app");
    var from = el("owo-memory-from");
    var to = el("owo-memory-to");
    var fromValue = (from && from.value.trim()) || "";
    var toValue = (to && to.value.trim()) || "";
    var list = el("owo-memory-entries");
    if ((fromValue && !/^\d{4}-\d{2}-\d{2}$/.test(fromValue)) ||
        (toValue && !/^\d{4}-\d{2}-\d{2}$/.test(toValue))) {
      if (list) list.textContent = "日期格式无效，请重新选择日期";
      return Promise.resolve();
    }
    if (fromValue && toValue && fromValue > toValue) {
      if (list) list.textContent = "开始日期不能晚于结束日期";
      return Promise.resolve();
    }
    var params = [];
    if (app && app.value.trim()) params.push("app=" + encodeURIComponent(app.value.trim()));
    if (fromValue) params.push("from=" + encodeURIComponent(fromValue));
    if (toValue) params.push("to=" + encodeURIComponent(toValue));
    params.push("limit=50");
    if (list) list.textContent = "正在加载条目…";
    return H.get("/memory/graph/entries?" + params.join("&"))
      .then(function (data) {
        if (request !== entriesGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-entries");
        if (!current) return;
        var entries=arrayData(data,"entries");
        current.innerHTML=entries.map(function(entry){return entryHtml(entry,false);}).join("") || empty("暂无记忆条目");
      })
      .catch(function (error) {
        if (request !== entriesGeneration || owner !== panelGeneration) return;
        var current = el("owo-memory-entries");
        if (current) current.innerHTML = H.esc(H.friendlyError(error));
      });
  }
  function dispose() {
    captureDraft();
    mounted=false;
    panelGeneration += 1;
    timelineGeneration += 1;
    entitiesGeneration += 1;
    relationsGeneration += 1;
    entriesGeneration += 1;
    recallGeneration += 1;
    mineGeneration += 1;
    addRelationGeneration += 1;
  }

  return {
    id: id,
    title: "记忆图谱",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { doRecall: doRecall, loadEntries: loadEntries, addRelation: addRelation, mineSkill: mineSkill, selectTab:selectTab, localTime:localTime },
  };
})();
