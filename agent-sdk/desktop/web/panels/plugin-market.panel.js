// 插件市场面板（Lane B）：目录/安装/更新/卸载/校验/扫描/seed/审计。
// 纯脚本 IIFE，无 ES module；经 OwoPanels 注册，主应用注入 helpers。
(function () {
  "use strict";

  window.OwoPanels = window.OwoPanels || {};

  var panelGeneration = 0;
  var catalogGeneration = 0;
  var auditGeneration = 0;
  var actionBusy = Object.create(null);
  var emptyIcon = '<svg class="owo-extension-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/></svg>';


  window.OwoPanels["plugin-market"] = {
    id: "plugin-market",
    title: "插件市场",

    tabs: ["catalog", "local", "remote", "audit", "dev"],
    fields: ["dir", "dir2", "id", "uid", "url", "rid", "rver", "seed", "search"],
    nav: function () {
      function field(id, label, placeholder) {
        return '<label for="market-' + id + '">' + label + '</label><input id="market-' + id + '" class="owo-market-' + id + '" placeholder="' + placeholder + '">';
      }
      var tabs = this.tabs.map(function (id, i) {
        return '<button type="button" role="tab" id="market-tab-' + id + '" data-market-tab="' + id + '" aria-controls="market-pane-' + id +
          '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' + ["目录", "本地安装", "远端市场", "审计", "开发工具"][i] + '</button>';
      }).join("");
      return '<section data-panel="plugin-market" data-layout="custom" class="owo-market-panel owo-insight">' +
        '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="插件市场">' + tabs +
        '</div><button type="button" class="owo-market-refresh">刷新</button></div>' +
        '<div class="owo-market-env sub" role="status">正在读取插件状态…</div>' +
        '<pre class="owo-market-result" role="status" aria-live="polite" hidden></pre>' +
        '<section id="market-pane-catalog" role="tabpanel" aria-labelledby="market-tab-catalog">' +
        field("search", "搜索插件", "按名称或 ID 搜索") +
        '<section class="owo-insight-section"><h3>已安装</h3><ul class="owo-market-list"></ul></section>' +
        '<section class="owo-insight-section"><h3>市场目录</h3><div class="owo-market-catalog"></div></section></section>' +
        '<section id="market-pane-local" role="tabpanel" aria-labelledby="market-tab-local" hidden><h3>扫描与校验</h3>' +
        field("dir", "插件目录", "相对工作区或绝对路径") +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-scan" data-core-action>扫描</button><button type="button" class="owo-market-verify" data-core-action>校验</button></div>' +
        '<section class="owo-insight-section"><h3>安装与更新</h3>' +
        field("dir2", "安装或更新来源", "插件目录路径") + field("id", "更新目标 ID", "仅更新时填写") +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-update" data-core-action>更新</button><button type="button" class="owo-market-install primary" data-core-action>安装</button></div></section>' +
        '<section class="owo-insight-section"><h3>卸载插件</h3>' + field("uid", "插件 ID", "已安装插件的 ID") +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-uninstall" data-core-action>卸载</button></div></section></section>' +
        '<section id="market-pane-remote" role="tabpanel" aria-labelledby="market-tab-remote" hidden><h3>远端市场</h3><p class="sub">拉取目录后，可在目录页选择插件。</p>' +
        field("url", "市场 URL", "留空使用配置的市场地址") +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-refreshremote" data-core-action>拉取目录</button></div>' +
        '<section class="owo-insight-section"><h3>按 ID 安装</h3>' + field("rid", "远端插件 ID", "插件 ID") + field("rver", "版本", "可选") +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-installremote primary" data-core-action>下载并安装</button></div></section></section>' +
        '<section id="market-pane-audit" role="tabpanel" aria-labelledby="market-tab-audit" hidden><h3>最近审计</h3><p class="sub">最近 20 条插件操作记录。</p><pre class="owo-market-audit sub" role="status">加载中…</pre></section>' +
        '<section id="market-pane-dev" role="tabpanel" aria-labelledby="market-tab-dev" hidden><h3>写入市场条目</h3><p class="sub">向本地目录写入 seed 条目，安装前仍需经过校验。</p>' +
        '<label for="market-seed">条目 JSON</label><textarea id="market-seed" class="owo-market-seed" rows="8" spellcheck="false" placeholder="粘贴包含 entries 的 JSON"></textarea>' +
        '<div class="owo-insight-actions"><button type="button" class="owo-market-seedbtn" data-core-action>写入 seed</button></div></section></section>';
    },

    mount: function (root, helpers) {
      this.dispose();
      var self = this;
      this.root = root;
      this.helpers = helpers || {};
      this.baseUrl =
        this.helpers.baseUrl ||
        (window.OwoPanels && window.OwoPanels.baseUrl) ||
        window.location.origin;
      this.get = this.helpers.get || this._get;
      this.post = this.helpers.post || this._post;
      this.esc = this.helpers.esc || this._esc;
      this.friendlyError = this.helpers.friendlyError || this._friendlyError;

      var scope = this.baseUrl + "#" + (this.helpers.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
      if (this.scope !== scope) { this.drafts = {}; this.activeTab = "catalog"; }
      this.scope = scope;
      this.catalogData = null;
      root.innerHTML = this.nav();
      var owner = panelGeneration;
      this.fields.forEach(function (id) {
        var el = root.querySelector(".owo-market-" + id);
        if (el && self.drafts && self.drafts[id] != null) el.value = self.drafts[id];
      });
      this.tabs.forEach(function (id, index) {
        var el = root.querySelector('[data-market-tab="' + id + '"]');
        el.addEventListener("click", function () { if (owner === panelGeneration) self.selectTab(id); });
        el.addEventListener("keydown", function (e) {
          if (owner !== panelGeneration) return;
          var next = e.key === "ArrowRight" ? (index + 1) % 5 : e.key === "ArrowLeft" ? (index + 4) % 5 : e.key === "Home" ? 0 : e.key === "End" ? 4 : -1;
          if (next < 0) return;
          e.preventDefault(); self.selectTab(self.tabs[next]);
          root.querySelector('[data-market-tab="' + self.tabs[next] + '"]').focus();
        });
      });
      this.selectTab(this.activeTab || "catalog");
      root.querySelector(".owo-market-search").addEventListener("input", function () {
        if (owner === panelGeneration && self.catalogData) self.renderCatalog(self.catalogData);
      });
      var $ = function (sel) {
        return root.querySelector(sel);
      };

      $(".owo-market-refresh").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.refresh(); self.refreshAudit();
      });
      $(".owo-market-scan").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doScan($(".owo-market-dir").value, $(".owo-market-scan"));
      });
      $(".owo-market-verify").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doVerify($(".owo-market-dir").value, $(".owo-market-verify"));
      });
      $(".owo-market-install").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doInstall($(".owo-market-dir2").value, $(".owo-market-install"));
      });
      $(".owo-market-update").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doUpdate($(".owo-market-id").value, $(".owo-market-dir2").value, $(".owo-market-update"));
      });
      $(".owo-market-uninstall").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doUninstall($(".owo-market-uid").value, $(".owo-market-uninstall"));
      });
      $(".owo-market-seedbtn").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doSeed($(".owo-market-seed").value, $(".owo-market-seedbtn"));
      });
      $(".owo-market-refreshremote").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doRefreshRemote($(".owo-market-url").value, $(".owo-market-refreshremote"));
      });
      $(".owo-market-installremote").addEventListener("click", function () {
        if (owner !== panelGeneration) return;
        self.doInstallRemote(
          $(".owo-market-rid").value,
          $(".owo-market-rver").value,
          $(".owo-market-url").value,
          $(".owo-market-installremote")
        );
      });

      this.refresh();
      this.refreshAudit();
    },

    refresh: function () {
      var self = this;
      var request = ++catalogGeneration;
      var owner = panelGeneration;
      return this.get("/plugins/market")
        .then(function (data) {
          if (request !== catalogGeneration || owner !== panelGeneration) return;
          self.renderCatalog(data || {});
        })
        .catch(function (error) {
          if (request !== catalogGeneration || owner !== panelGeneration) return;
          var root = self._root();
          var env = root && root.querySelector(".owo-market-env");
          if (env) env.textContent = "目录读取失败：" + self.friendlyError(error) + (self.catalogData ? "（保留上次结果）" : "，请刷新重试");

        });
    },

    renderCatalog: function (data) {
      if (!this.root) return;
      if (!data || !Array.isArray(data.plugins) || data.plugins.some(function (item) { return !item || typeof item !== "object"; })) throw new Error("插件目录响应不完整");
      this.catalogData = data;
      var list = this.root.querySelector(".owo-market-list");
      var catalog = this._root().querySelector(".owo-market-catalog");
      if (!list) return;
      var envEl = this._root().querySelector(".owo-market-env");
      if (envEl) {
        envEl.textContent =
          "App " + (data.app_version || "—") + " · 签名验证" + (data.require_signature === true ? "开启" : data.require_signature === false ? "关闭" : "状态未知");
      }
      var search = this.root.querySelector(".owo-market-search");
      var query = String(search && search.value || "").trim().toLowerCase();
      var plugins = data.plugins.filter(function (item) {
        return !query || [item.id, item.name, item.description].join(" ").toLowerCase().indexOf(query) >= 0;
      });
      var installed = plugins.filter(function (item) {
        return item.source !== "market";
      });
      var market = plugins.filter(function (item) {
        return item.source === "market";
      });
      list.innerHTML = "";
      for (var i = 0; i < installed.length; i++) {
        list.appendChild(this._installedItem(installed[i]));
      }
      if (!installed.length) {
        list.innerHTML = '<li class="owo-insight-empty">' + emptyIcon + '<span>' + (query ? "没有匹配的已安装插件" : "暂无已安装插件") + "</span></li>";
      }
      if (catalog) {
        catalog.innerHTML = "";
        if (!market.length) {
          catalog.innerHTML = '<div class="owo-insight-empty">' + emptyIcon + '<span>' + (query ? "没有匹配的市场条目" : "市场目录为空，可在远端市场中拉取目录。") + "</span></div>";
        }
        for (var j = 0; j < market.length; j++) {
          catalog.appendChild(this._marketItem(market[j]));
        }
      }
    },

    _installedItem: function (plugin) {
      var self = this, owner = panelGeneration;
      var li = document.createElement("li");
      var risks = Array.isArray(plugin.risks) ? plugin.risks.join("；") : "";
      var riskBadge = risks
        ? '<span class="sub owo-market-risk">⚠ ' + this.esc(risks) + "</span>"
        : "";
      var updateBadge = plugin.has_update
        ? '<span class="sub">⬆ 可更新</span>'
        : "";
      var initial = this.esc(String(plugin.name || plugin.id || "?").trim().slice(0, 1).toUpperCase());
      li.innerHTML =
        '<div class="owo-market-item-top">' +
        '<span class="owo-market-tile" aria-hidden="true">' +
        initial +
        "</span>" +
        '<div class="owo-market-item-meta">' +
        "<strong>" +
        this.esc(plugin.name || plugin.id) +
        "</strong>" +
        '<span class="sub">' +
        this.esc(plugin.id) +
        " v" +
        this.esc(plugin.version) +
        " ｜ " +
        this.esc(plugin.source) +
        updateBadge +
        "</span>" +
        "</div>" +
        '<span class="ps-check" title="已安装">✓</span>' +
        "</div>" +
        '<p class="owo-market-item-desc">' +
        this.esc(plugin.description || "暂无描述") +
        riskBadge +
        "</p>" +
        '<div class="owo-market-item-foot">' +
        '<span class="sub">' +
        this.esc(plugin.path || "") +
        "</span>" +
        "<button type=\"button\" data-core-action>卸载</button>" +
        "</div>";
      li.querySelector("button").addEventListener("click", function () {
        if (owner === panelGeneration) self.doUninstall(plugin.id, li.querySelector("button"));
      });
      return li;
    },

    _marketItem: function (entry) {
      var self = this, owner = panelGeneration;
      var row = document.createElement("div");
      row.className = "owo-market-cat-row";
      var initial = this.esc(String(entry.name || entry.id || "?").trim().slice(0, 1).toUpperCase());
      row.innerHTML =
        '<span class="owo-market-tile" aria-hidden="true">' +
        initial +
        "</span>" +
        '<div class="owo-market-cat-meta">' +
        "<strong>" +
        this.esc(entry.name || entry.id) +
        "</strong>" +
        '<span class="sub">v' +
        this.esc(entry.version || "?") +
        " ｜ 最低支持 App " +
        this.esc(entry.min_app_version || "—") +
        "</span>" +
        '<p class="owo-market-item-desc">' + this.esc(entry.description || "") + "</p></div>";
      var button = document.createElement("button");
      button.type = "button";
      button.className = "owo-market-cat-install";
      button.setAttribute("data-core-action", "true");
      button.textContent = "安装";
      button.addEventListener("click", function () {
        if (owner === panelGeneration) self.doInstallRemote(entry.id, entry.version || "", "", button);
      });
      row.appendChild(button);
      return row;
    },

    _action: function (key, button, request, success, failure) {
      if (!this.root) return Promise.resolve();
      var scope = this.scope;
      key = scope + ":" + key;
      if (actionBusy[key]) {
        this._result("同一插件操作仍在执行，请稍后再试。");
        return Promise.resolve();
      }
      var actionToken = {};
      actionBusy[key] = actionToken;
      var owner = panelGeneration;
      var self = this;
      var idleText = button && button.textContent;
      if (button) {
        if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, true);
        button.disabled = true;
        button.setAttribute("aria-busy", "true");
        button.textContent = "处理中…";
      }
      var pending;
      try { pending = request(); } catch (error) { pending = Promise.reject(error); }
      return Promise.resolve(pending)
        .then(function (data) {
          if (owner === panelGeneration) {
            success(data);
          } else if (self.root && scope === self.scope) {
            self._result("上一页面发起的插件操作已完成，请刷新当前目录确认状态。");
            self.refresh();
            self.refreshAudit();
          }
        })
        .catch(function (error) {
          if (owner === panelGeneration) {
            failure(error);
          } else if (self.root && scope === self.scope) {
            self._result("上一页面发起的插件操作失败：" + self.friendlyError(error));
          }
        })
        .finally(function () {
          if (actionBusy[key] !== actionToken) return;
          delete actionBusy[key];
          if (owner === panelGeneration && button) {
            if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, false);
            button.disabled = false;
            button.removeAttribute("aria-busy");
            button.textContent = idleText;
          }
        });
    },

    doScan: function (dir, button) {
      var self = this;
      if (!dir) return this._result("请填写插件目录");
      return this._action("scan:" + dir, button, function () {
        return self.get("/plugins/market/scan?dir=" + encodeURIComponent(dir));
      }, function (data) {
        self._result("扫描：" + data.dir + "\n通过：" + data.pass + "\n风险：" + JSON.stringify(data.risks));
      }, function (error) { self._result("扫描失败：" + self.friendlyError(error)); });
    },

    doVerify: function (dir, button) {
      var self = this;
      if (!dir) return this._result("请填写插件目录");
      return this._action("verify:" + dir, button, function () {
        return self.post("/plugins/market/verify", { dir: dir });
      }, function (data) {
        var report = data.report || {};
        self._result("校验通过：" + report.id + " v" + report.version + "\n" + (report.audit || []).join("\n"));
      }, function (error) { self._result("校验失败：" + self.friendlyError(error)); });
    },

    doInstall: function (dir, button) {
      var self = this;
      if (!dir) return this._result("请填写插件目录");
      return this._action("install:" + dir, button, function () {
        return self.post("/plugins/market/install", { dir: dir });
      }, function (data) {
        var report = data.report || {};
        self._result("安装完成：" + report.id + " v" + report.version + " 状态 " + report.state);
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("安装失败：" + self.friendlyError(error)); });
    },

    doUpdate: function (id, dir, button) {
      var self = this;
      if (!dir) return this._result("请填写插件目录");
      return this._action("update:" + (id || "") + ":" + dir, button, function () {
        return self.post("/plugins/market/update", { id: id, dir: dir });
      }, function (data) {
        var report = data.report || {};
        self._result("更新完成：" + report.id + " → v" + report.version);
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("更新失败：" + self.friendlyError(error)); });
    },

    doUninstall: function (id, button) {
      var self = this, owner = panelGeneration, post = this.post;
      if (!id) return this._result("请填写卸载 id");
      return this._action("uninstall:" + id, button, function () {
        var confirm = self.helpers && self.helpers.confirm
          ? self.helpers.confirm({
              title: "卸载插件",
              message: "确定卸载插件“" + id + "”？其文件将从工作区移除。",
              confirmText: "卸载插件",
              kind: "danger",
            })
          : Promise.resolve(window.confirm("确定卸载插件“" + id + "”？其文件将从工作区移除。"));
        return Promise.resolve(confirm).then(function (accepted) {
          if (!accepted || owner !== panelGeneration) return { cancelled: true };
          return post.call(self, "/plugins/market/uninstall", { id: id });
        });
      }, function (data) {
        if (data && data.cancelled) {
          self._result("已取消卸载：" + id);
          return;
        }
        self._result("已卸载：" + id + "，移除 " + ((data.removed || []).length) + " 个文件");
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("卸载失败：" + self.friendlyError(error)); });
    },

    doSeed: function (raw, button) {
      var self = this;
      var body;
      try { body = JSON.parse(raw || "{}"); }
      catch (error) { return this._result("seed JSON 解析失败：" + error.message); }
      return this._action("seed", button, function () {
        return self.post("/plugins/market/seed", body);
      }, function (data) {
        self._result("已写入 " + data.entries + " 条市场条目");
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("seed 失败：" + self.friendlyError(error)); });
    },

    doRefreshRemote: function (url, button) {
      var self = this;
      var body = url ? { url: url } : {};
      return this._action("refresh-remote", button, function () {
        return self.post("/plugins/market/refresh", body);
      }, function (data) {
        self._result("registry 已刷新：" + (data.entries || 0) + " 条（来源 " + (data.source || "?") + "）");
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("刷新失败：" + self.friendlyError(error)); });
    },

    doInstallRemote: function (id, version, url, button) {
      var self = this;
      if (!id) return this._result("请填写远端插件 id");
      var body = { id: id };
      if (version) body.version = version;
      if (url) body.url = url;
      return this._action("install-remote:" + id + ":" + (version || ""), button, function () {
        return self.post("/plugins/market/install-remote", body);
      }, function (data) {
        var report = data.report || {};
        self._result("远端安装完成：" + (report.id || id) + " v" + (report.version || "?") + " 状态 " + (report.state || "?"));
        self.refresh(); self.refreshAudit();
      }, function (error) { self._result("远端安装失败：" + self.friendlyError(error)); });
    },

    refreshAudit: function () {
      var self = this;
      var request = ++auditGeneration;
      var owner = panelGeneration;
      return this.get("/plugins/market/audit?n=20")
        .then(function (data) {
          if (request !== auditGeneration || owner !== panelGeneration) return;
          var root = self._root();
          var el = root && root.querySelector(".owo-market-audit");
          if (!data || !Array.isArray(data.entries)) throw new Error("审计响应不完整");
          if (el) el.textContent = data.entries.join("\n") || "暂无审计记录";
        })
        .catch(function (error) {
          if (request !== auditGeneration || owner !== panelGeneration) return;
          var root = self._root();
          var el = root && root.querySelector(".owo-market-audit");
          if (el) el.textContent = "审计读取失败：" + self.friendlyError(error);
        });
    },

    _result: function (text) {
      var root = this._root();
      var el = root && root.querySelector(".owo-market-result");
      if (el) { el.textContent = text; el.hidden = !text; }
    },

    _root: function () {
      return this.root || null;
    },

    selectTab: function (id) {
      if (!this.root || this.tabs.indexOf(id) < 0) return;
      this.activeTab = id;
      var self = this;
      this.tabs.forEach(function (tab) {
        var button = self.root.querySelector('[data-market-tab="' + tab + '"]');
        if (button && button.setAttribute) { button.setAttribute("aria-selected", String(tab === id)); button.setAttribute("tabindex", tab === id ? "0" : "-1"); }
        var pane = self.root.querySelector("#market-pane-" + tab);
        if (pane) pane.hidden = tab !== id;
      });
    },

    dispose: function () {
      if (this.root) {
        var self = this; this.drafts = {};
        this.fields.forEach(function (id) { var el = self.root.querySelector(".owo-market-" + id); if (el) self.drafts[id] = el.value; });
      }
      panelGeneration += 1;
      catalogGeneration += 1;
      auditGeneration += 1;
      this.root = null;
    },

    // ---- helpers 缺省实现（防御性降级） ----

    _get: function (path) {
      return window.OwoApi.stream(path).then(function (response) {
        if (!response.ok) {
          return response.text().then(function (body) {
            throw new Error(response.status + ": " + body);
          });
        }
        return response.status === 204 ? null : response.json();
      });
    },

    _post: function (path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (response) {
        if (!response.ok) {
          return response.text().then(function (text) {
            throw new Error(response.status + ": " + text);
          });
        }
        return response.json();
      });
    },

    _esc: function (text) {
      var div = document.createElement("div");
      div.textContent = text == null ? "" : String(text);
      return div.innerHTML;
    },

    _friendlyError: function (error) {
      var msg = String((error && error.message) || error || "");
      var match = msg.match(/^(\d{3}):/);
      var status = match ? Number(match[1]) : 0;
      if (status === 404 || status === 405 || status >= 500) {
        return "服务接口不可用（HTTP " + status + "）";
      }
      return msg || "未知错误";
    },
  };
})();
