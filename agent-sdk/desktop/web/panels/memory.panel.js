/* Lane 4 记忆面板：记忆图谱（第二大脑 · 子任务 1）。
 * 纯脚本 IIFE，注册 window.OwoPanels.memory。防御性降级。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.memory = (function () {
  "use strict";

  var id = "memory";
  var H = null;
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
    return (
      '<section data-panel="' + id + '">' +
      "<style>" +
      ".owo-memory-card{display:inline-block;margin:4px;padding:4px 10px;border:1px solid var(--border-strong);border-radius:12px;font-size:12px;background:var(--surface-2)}" +
      ".owo-memory-row{display:flex;gap:8px;align-items:center;padding:3px 0;border-bottom:1px solid var(--border);font-size:12px}" +
      ".owo-memory-rel{display:inline-block;margin:2px;padding:2px 8px;border:1px solid var(--accent);border-radius:8px;font-size:12px}" +
      ".owo-memory-hit{background:var(--yellow-soft);padding:2px 4px;border-radius:4px;font-size:12px}" +
      ".owo-memory-row select{width:auto;flex:0 0 auto}" +
      "</style>" +
      '<div class="stack">' +
      '<div class="sub">从情景记忆挖掘技能包（观察动作序列 → 泛化 → 沉淀；需先有观察样本）</div>' +
      '<div class="owo-memory-row"><input id="owo-memory-mine-name" placeholder="技能名（如 send-file）" style="flex:1">' +
      '<select id="owo-memory-mine-sensitivity"><option value="low">低敏感</option><option value="medium">中敏感</option><option value="high">高敏感</option></select>' +
      '<button class="primary" id="owo-memory-mine-btn" data-core-action>挖掘</button></div>' +
      '<div class="owo-memory-row"><input id="owo-memory-mine-apps" placeholder="目标应用，逗号分隔（如 qq）" style="flex:1">' +
      '<input id="owo-memory-mine-desc" placeholder="描述（可选）" style="flex:1"></div>' +
      '<div id="owo-memory-mine-result" class="sub"></div>' +
      '<div class="sub" style="margin-top:6px">记忆图谱（结构化检索 / 时间线 / 实体 / 关系 / recall）</div>' +
      '<div class="owo-memory-row"><input id="owo-memory-recall" placeholder="recall 查询（如：张子豪）" style="flex:1">' +
      '<button class="primary" id="owo-memory-recall-btn">检索</button></div>' +
      '<div id="owo-memory-recall-box"></div>' +
      '<div class="sub">时间线</div><div id="owo-memory-timeline" class="list"></div>' +
      '<div class="sub">实体（词元频次 + 共现）</div><div id="owo-memory-entities"></div>' +
      '<div class="sub">手动关系</div>' +
      '<div class="owo-memory-row"><input id="owo-memory-rel-a" placeholder="实体A" style="flex:1">' +
      '<input id="owo-memory-rel-b" placeholder="实体B" style="flex:1">' +
      '<input id="owo-memory-rel-r" placeholder="关系（如：约定）" style="flex:1">' +
      '<button id="owo-memory-rel-add" data-core-action>添加</button></div>' +
      '<div id="owo-memory-relations"></div>' +
      '<div class="sub">条目（app/时间过滤）</div>' +
      '<div class="owo-memory-row"><input id="owo-memory-app" placeholder="app（如 qq）" aria-label="应用过滤" style="flex:1">' +
      '<input id="owo-memory-from" type="date" aria-label="开始日期" title="开始日期（含当天）">' +
      '<input id="owo-memory-to" type="date" aria-label="结束日期" title="结束日期（含当天）">' +
      '<button id="owo-memory-refresh">刷新</button></div>' +
      '<div id="owo-memory-entries" class="list"></div>' +
      "</div>"
    );
  }

  function mount(root, helpers) {
    dispose();
    H = helpers || defaultHelpers();
    root.innerHTML = nav();
    root.querySelector("#owo-memory-recall-btn").addEventListener("click", doRecall);
    root.querySelector("#owo-memory-recall").addEventListener("keydown", function (e) {
      if (e.key === "Enter") doRecall();
    });
    root.querySelector("#owo-memory-rel-add").addEventListener("click", addRelation);
    root.querySelector("#owo-memory-refresh").addEventListener("click", refresh);
    root.querySelector("#owo-memory-mine-btn").addEventListener("click", mineSkill);
    root.querySelector("#owo-memory-mine-name").addEventListener("keydown", function (e) {
      if (e.key === "Enter") mineSkill();
    });
    refresh();
  }

  /// 从情景记忆挖掘技能包：观察动作序列 → 泛化 → 沉淀（服务端 /memory/mine-skill）。
  function mineSkill() {
    if (mining) return Promise.resolve();
    var nameEl = document.getElementById("owo-memory-mine-name");
    var name = (nameEl && nameEl.value.trim()) || "";
    var result = document.getElementById("owo-memory-mine-result");
    if (!name) {
      if (result) result.textContent = "请先填写技能名";
      return;
    }
    var appsEl = document.getElementById("owo-memory-mine-apps");
    var descEl = document.getElementById("owo-memory-mine-desc");
    var sensEl = document.getElementById("owo-memory-mine-sensitivity");
    var targetApps = ((appsEl && appsEl.value) || "")
      .split(",")
      .map(function (s) {
        return s.trim();
      })
      .filter(Boolean);
    mining = true;
    var request = ++mineGeneration;
    var owner = panelGeneration;
    var button = document.getElementById("owo-memory-mine-btn");
    if (button) button.disabled = true;
    if (result) result.textContent = "正在挖掘…";
    return H.post("/memory/mine-skill", {
      name: name,
      target_apps: targetApps,
      description: (descEl && descEl.value.trim()) || "",
      sensitivity: (sensEl && sensEl.value) || "low",
    })
      .then(function (data) {
        if (request !== mineGeneration || owner !== panelGeneration) return;
        var el = document.getElementById("owo-memory-mine-result"); // 面板已卸载则跳过
        if (!el) return;
        var variables = (data && data.variables) || [];
        el.innerHTML =
          "已生成技能包 <b>" + H.esc(data && data.name) + "</b>" +
          (variables.length ? "（变量 " + variables.length + " 个）" : "") +
          "，可在「操作学习」区块查看 / 导出 / 导入。";
      })
      .catch(function (e) {
        if (request !== mineGeneration || owner !== panelGeneration) return;
        var el = document.getElementById("owo-memory-mine-result"); // 面板已卸载则跳过
        if (el) el.textContent = H.friendlyError(e);
      })
      .finally(function () {
        if (request !== mineGeneration) return;
        mining = false;
        var btn = document.getElementById("owo-memory-mine-btn"); // 面板已卸载则跳过
        if (btn) btn.disabled = false;
      });
  }

  function refresh() {
    loadTimeline();
    loadEntities();
    loadRelations();
    loadEntries();
  }

  function doRecall() {
    var request = ++recallGeneration;
    var owner = panelGeneration;
    var input = document.getElementById("owo-memory-recall");
    var query = (input && input.value.trim()) || "";
    var box = document.getElementById("owo-memory-recall-box");
    if (box) box.textContent = "正在检索…";
    return H.get("/memory/graph/recall?q=" + encodeURIComponent(query) + "&top_k=5")
      .then(function (data) {
        if (request !== recallGeneration || owner !== panelGeneration) return;
        var currentBox = document.getElementById("owo-memory-recall-box");
        if (!currentBox) return;
        currentBox.innerHTML = "<div class='sub'>命中 " + ((data && data.count) || 0) + " 条</div>" +
          ((data && data.hits) || []).map(function (hit) {
            return '<div class="owo-memory-hit">[' + H.esc(hit.app_id) + "] " + H.esc(hit.ts) + " — " + H.esc(hit.summary) +
              (hit.matched_entities && hit.matched_entities.length ? " ｜ 实体命中：" + hit.matched_entities.map(H.esc).join("、") : "") + "</div>";
          }).join("");
      })
      .catch(function (error) {
        if (request !== recallGeneration || owner !== panelGeneration) return;
        var currentBox = document.getElementById("owo-memory-recall-box");
        if (currentBox) currentBox.innerHTML = '<div class="owo-memory-hit">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }
  /// 时间线直方图：每天一条横条，宽度按占比，尾部带计数（替代纯文本行）。
  function timelineBars(buckets) {
    var rows = (buckets || []).slice(0, 30);
    if (!rows.length) return '<div class="owo-memory-row">暂无观察记录</div>';
    var max = 1;
    rows.forEach(function (bucket) {
      max = Math.max(max, Number(bucket.count) || 0);
    });
    return rows
      .map(function (bucket) {
        var count = Number(bucket.count) || 0;
        var percent = max > 0 ? Math.round((count / max) * 100) : 0;
        var width = Math.max(percent, count > 0 ? 4 : 1);
        return (
          '<div class="owo-mem-tl-row">' +
          '<span class="owo-mem-tl-day">' + H.esc(bucket.day) + "</span>" +
          '<span class="owo-mem-tl-track" aria-hidden="true">' +
          '<i style="width:' + width + '%"></i></span>' +
          '<span class="owo-mem-tl-count">' + count + "</span></div>"
        );
      })
      .join("");
  }

  function loadTimeline() {
    var request = ++timelineGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/timeline")
      .then(function (data) {
        if (request !== timelineGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-timeline");
        if (!current) return;
        current.innerHTML = timelineBars((data && data.buckets) || []);
      })
      .catch(function (error) {
        if (request !== timelineGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-timeline");
        if (current) current.innerHTML = '<div class="owo-memory-row">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }

  /// 实体频次热度芯片：底色不透明度按频次占比，数字在芯片内。
  function entityChips(entities) {
    var rows = (entities || []).slice(0, 30);
    if (!rows.length) return '<div class="owo-memory-card">暂无实体</div>';
    var max = 1;
    rows.forEach(function (entity) {
      max = Math.max(max, Number(entity.count) || 0);
    });
    return rows
      .map(function (entity) {
        var count = Number(entity.count) || 0;
        var weight = max > 0 ? count / max : 0;
        var alpha = (0.15 + 0.55 * weight).toFixed(2);
        return (
          '<span class="owo-mem-ent" style="background:color-mix(in oklab, var(--accent) ' +
          Math.round(alpha * 100) + '%, var(--surface-2))" title="' + H.esc(entity.entity) + " ×" + count + '">' +
          H.esc(entity.entity) + "<i>" + count + "</i></span>"
        );
      })
      .join("");
  }

  function loadEntities() {
    var request = ++entitiesGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/entities?limit=30")
      .then(function (data) {
        if (request !== entitiesGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-entities");
        if (!current) return;
        current.innerHTML = entityChips((data && data.entities) || []);
      })
      .catch(function (error) {
        if (request !== entitiesGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-entities");
        if (current) current.innerHTML = '<div class="owo-memory-card">' + H.esc(H.friendlyError(error)) + "</div>";
      });
  }
  function loadRelations() {
    var request = ++relationsGeneration;
    var owner = panelGeneration;
    return H.get("/memory/graph/links")
      .then(function (data) {
        if (request !== relationsGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-relations");
        if (!current) return;
        current.innerHTML = ((data && data.links) || []).map(function (link) {
          return '<span class="owo-memory-rel">' + H.esc(link.a) + " —" + H.esc(link.relation) + "→ " + H.esc(link.b) + "</span>";
        }).join("");
      })
      .catch(function (error) {
        if (request !== relationsGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-relations");
        if (current) current.innerHTML = '<span class="owo-memory-rel">' + H.esc(H.friendlyError(error)) + "</span>";
      });
  }
  function addRelation() {
    if (addingRelation) return Promise.resolve();
    var a = document.getElementById("owo-memory-rel-a");
    var b = document.getElementById("owo-memory-rel-b");
    var relation = document.getElementById("owo-memory-rel-r");
    var values = { a: (a && a.value.trim()) || "", b: (b && b.value.trim()) || "", relation: (relation && relation.value.trim()) || "" };
    if (!values.a || !values.b || !values.relation) {
      notify("请填写实体 A、实体 B 和关系。", "error");
      return Promise.resolve();
    }
    addingRelation = true;
    var request = ++addRelationGeneration;
    var owner = panelGeneration;
    var button = document.getElementById("owo-memory-rel-add");
    if (button) button.disabled = true;
    return H.post("/memory/graph/link", values)
      .then(function () {
        if (request !== addRelationGeneration || owner !== panelGeneration) return;
        a.value = ""; b.value = ""; relation.value = "";
        return loadRelations();
      })
      .catch(function (error) {
        if (request === addRelationGeneration && owner === panelGeneration) notify(H.friendlyError(error), "error");
      })
      .finally(function () {
        if (request !== addRelationGeneration) return;
        addingRelation = false;
        var currentButton = document.getElementById("owo-memory-rel-add");
        if (currentButton) currentButton.disabled = false;
      });
  }
  function loadEntries() {
    var request = ++entriesGeneration;
    var owner = panelGeneration;
    var app = document.getElementById("owo-memory-app");
    var from = document.getElementById("owo-memory-from");
    var to = document.getElementById("owo-memory-to");
    var fromValue = (from && from.value.trim()) || "";
    var toValue = (to && to.value.trim()) || "";
    var list = document.getElementById("owo-memory-entries");
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
        var current = document.getElementById("owo-memory-entries");
        if (!current) return;
        current.innerHTML = ((data && data.entries) || []).map(function (entry) {
          return '<div class="owo-memory-row">[' + H.esc(entry.app_id) + "] " + H.esc(entry.ts) + " — " + H.esc(entry.summary) + "</div>";
        }).join("") || '<div class="sub">暂无记忆条目</div>';
      })
      .catch(function (error) {
        if (request !== entriesGeneration || owner !== panelGeneration) return;
        var current = document.getElementById("owo-memory-entries");
        if (current) current.innerHTML = H.esc(H.friendlyError(error));
      });
  }
  function dispose() {
    panelGeneration += 1;
    timelineGeneration += 1;
    entitiesGeneration += 1;
    relationsGeneration += 1;
    entriesGeneration += 1;
    recallGeneration += 1;
    mineGeneration += 1;
    addRelationGeneration += 1;
    mining = false;
    addingRelation = false;
  }

  return {
    id: id,
    title: "记忆图谱",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { doRecall: doRecall, loadEntries: loadEntries, addRelation: addRelation, mineSkill: mineSkill, timelineBars: timelineBars, entityChips: entityChips },
  };
})();
