// 插件市场面板（Lane B）：目录/安装/更新/卸载/校验/扫描/seed/审计。
// 纯脚本 IIFE，无 ES module；经 OwoPanels 注册，主应用注入 helpers。
(function () {
  "use strict";

  window.OwoPanels = window.OwoPanels || {};

  var panelGeneration = 0;
  var catalogGeneration = 0;
  var auditGeneration = 0;
  var actionBusy = Object.create(null);

  window.OwoPanels["plugin-market"] = {
    id: "plugin-market",
    title: "插件市场",

    nav: function () {
      return (
        '<section data-panel="plugin-market" data-layout="custom" class="owo-market-panel">' +
        '<div class="owo-market-hero">' +
        "<div>" +
        "<h3>插件市场</h3>" +
        '<span class="owo-market-env sub"></span>' +
        "</div>" +
        '<button class="owo-market-refresh">刷新</button>' +
        "</div>" +
        '<div class="owo-market-block">' +
        "<h4>已安装</h4>" +
        '<ul class="owo-market-list list"></ul>' +
        "</div>" +
        '<div class="owo-market-block">' +
        "<h4>市场目录（可安装）</h4>" +
        '<div class="owo-market-catalog"></div>' +
        "</div>" +
        '<details class="owo-market-advanced">' +
        "<summary>高级工具（扫描 / 校验 / 目录安装 / 更新 / 卸载）</summary>" +
        '<div class="stack">' +
        '<div class="inline"><input class="owo-market-dir" placeholder="插件目录路径（相对 workspace 或绝对）"><button class="owo-market-scan" data-core-action>扫描</button><button class="owo-market-verify" data-core-action>校验</button></div>' +
        '<div class="inline"><input class="owo-market-dir2" placeholder="插件目录（安装/更新源）"><input class="owo-market-id" placeholder="更新目标 id（update 时）"><button class="owo-market-install" data-core-action>安装</button><button class="owo-market-update" data-core-action>更新</button></div>' +
        '<div class="inline"><input class="owo-market-uid" placeholder="卸载 id"><button class="owo-market-uninstall" data-core-action>卸载</button></div>' +
        '<div class="owo-market-result sub"></div>' +
        "</div>" +
        "</details>" +
        '<details class="owo-market-advanced">' +
        "<summary>远端市场（registry）</summary>" +
        '<div class="stack">' +
        '<div class="inline"><input class="owo-market-url" placeholder="市场 URL（OWO_MARKET_URL 缺省）"><button class="owo-market-refreshremote">拉取 registry</button></div>' +
        '<div class="inline"><input class="owo-market-rid" placeholder="远端插件 id"><input class="owo-market-rver" placeholder="版本（可选）"><button class="owo-market-installremote primary" data-core-action>下载并安装</button></div>' +
        "</div>" +
        "</details>" +
        '<details class="owo-market-advanced">' +
        "<summary>Seed 示例市场条目</summary>" +
        '<div class="stack">' +
        '<textarea class="owo-market-seed" rows="4" spellcheck="false" placeholder=\'{"entries":[{"id":"owo.plugin.demo","name":"Demo","version":"1.0.0","min_app_version":"0.5.0"}]}\'></textarea>' +
        '<button class="owo-market-seedbtn" data-core-action>写入 seed</button>' +
        "</div>" +
        "</details>" +
        '<details class="owo-market-advanced">' +
        "<summary>审计尾部</summary>" +
        '<pre class="owo-market-audit sub">—</pre>' +
        "</details>" +
        "<style>" +
        ".owo-market-panel { display: flex; flex-direction: column; gap: 12px; }" +
        ".owo-market-hero { display: flex; align-items: flex-end; justify-content: space-between; gap: 10px; }" +
        ".owo-market-hero h3 { margin: 0 0 2px; font-size: 14px; color: var(--text); }" +
        ".owo-market-block h4 { margin: 0 0 8px; font-size: 12px; color: var(--text-2); font-weight: 600; }" +
        ".owo-market-list { display: grid; grid-template-columns: repeat(auto-fill, minmax(215px, 1fr)); gap: 10px; margin: 0; padding: 0; list-style: none; }" +
        ".owo-market-list li { display: flex; flex-direction: column; gap: 6px; padding: 12px; border: 1px solid var(--border); border-radius: var(--r-md); background: var(--surface); cursor: default; }" +
        ".owo-market-list li:hover { background: var(--surface); border-color: var(--border-strong); }" +
        ".owo-market-item-top { display: flex; align-items: center; gap: 9px; }" +
        ".owo-market-tile { display: grid; place-items: center; flex: none; width: 34px; height: 34px; border-radius: 9px; background: var(--accent-soft); color: var(--accent); font-weight: 700; font-size: 15px; }" +
        ".owo-market-item-meta { flex: 1; min-width: 0; }" +
        ".owo-market-item-meta strong { display: block; font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }" +
        ".owo-market-item-meta .sub { margin-top: 1px; }" +
        ".owo-market-item-desc { margin: 0; min-height: 32px; font-size: 12px; color: var(--text-2); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }" +
        ".owo-market-item-foot { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-top: auto; }" +
        ".owo-market-item-foot button { height: 24px; padding: 0 9px; font-size: 11.5px; }" +
        ".owo-market-catalog { display: flex; flex-direction: column; gap: 6px; }" +
        ".owo-market-cat-row { display: flex; align-items: center; gap: 10px; padding: 9px 11px; border: 1px solid var(--border); border-radius: var(--r-md); background: var(--surface); }" +
        ".owo-market-cat-row:hover { border-color: var(--border-strong); }" +
        ".owo-market-cat-meta { flex: 1; min-width: 0; }" +
        ".owo-market-cat-meta strong { display: block; font-size: 12.5px; }" +
        ".owo-market-cat-meta .sub { margin-top: 1px; }" +
        ".owo-market-cat-install { flex: none; height: 26px; padding: 0 12px; font-size: 11.5px; font-weight: 600; color: var(--accent); background: var(--accent-soft); border-color: transparent; }" +
        ".owo-market-cat-install:hover { color: var(--accent-ink); background: var(--accent); border-color: var(--accent); }" +
        ".owo-market-result { white-space: pre-wrap; max-height: 200px; overflow: auto; }" +
        ".owo-market-audit { white-space: pre-wrap; max-height: 180px; overflow: auto; }" +
        ".owo-market-advanced > summary { font-size: 12px; }" +
        ".owo-market-panel input { flex: 1; min-width: 0; }" +
        ".owo-market-risk { color: var(--red); }" +
        "</style>" +
        "</section>"
      );
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

      root.innerHTML = this.nav();
      var $ = function (sel) {
        return root.querySelector(sel);
      };

      $(".owo-market-refresh").addEventListener("click", function () {
        self.refresh();
      });
      $(".owo-market-scan").addEventListener("click", function () {
        self.doScan($(".owo-market-dir").value, $(".owo-market-scan"));
      });
      $(".owo-market-verify").addEventListener("click", function () {
        self.doVerify($(".owo-market-dir").value, $(".owo-market-verify"));
      });
      $(".owo-market-install").addEventListener("click", function () {
        self.doInstall($(".owo-market-dir2").value, $(".owo-market-install"));
      });
      $(".owo-market-update").addEventListener("click", function () {
        self.doUpdate($(".owo-market-id").value, $(".owo-market-dir2").value, $(".owo-market-update"));
      });
      $(".owo-market-uninstall").addEventListener("click", function () {
        self.doUninstall($(".owo-market-uid").value, $(".owo-market-uninstall"));
      });
      $(".owo-market-seedbtn").addEventListener("click", function () {
        self.doSeed($(".owo-market-seed").value, $(".owo-market-seedbtn"));
      });
      $(".owo-market-refreshremote").addEventListener("click", function () {
        self.doRefreshRemote($(".owo-market-url").value, $(".owo-market-refreshremote"));
      });
      $(".owo-market-installremote").addEventListener("click", function () {
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
          var list = root && root.querySelector(".owo-market-list");
          if (list) {
            list.innerHTML =
              '<li class="sub">' +
              self.esc(self.friendlyError(error)) +
              "</li>";
          }
        });
    },

    renderCatalog: function (data) {
      var list = this._root().querySelector(".owo-market-list");
      var catalog = this._root().querySelector(".owo-market-catalog");
      if (!list) return;
      var envEl = this._root().querySelector(".owo-market-env");
      if (envEl) {
        envEl.textContent =
          "App " + data.app_version + " ｜ 签名" + (data.require_signature ? "开启" : "关闭");
      }
      var plugins = data.plugins || [];
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
        list.innerHTML = '<li class="sub">暂无已安装插件</li>';
      }
      if (catalog) {
        catalog.innerHTML = "";
        if (!market.length) {
          catalog.innerHTML = '<div class="sub">市场目录为空（可在「远端市场」中拉取 registry）</div>';
        }
        for (var j = 0; j < market.length; j++) {
          catalog.appendChild(this._marketItem(market[j]));
        }
      }
    },

    _installedItem: function (plugin) {
      var self = this;
      var li = document.createElement("li");
      var risks = plugin.risks && plugin.risks.length ? plugin.risks.join("；") : "";
      var riskBadge = risks
        ? '<span class="sub owo-market-risk">⚠ ' + this.esc(risks) + "</span>"
        : "";
      var updateBadge = plugin.has_update
        ? '<span class="sub">⬆ 可更新</span>'
        : "";
      var initial = this.esc((plugin.name || plugin.id || "?").trim().slice(0, 1).toUpperCase());
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
        self.doUninstall(plugin.id);
      });
      return li;
    },

    _marketItem: function (entry) {
      var self = this;
      var row = document.createElement("div");
      row.className = "owo-market-cat-row";
      var initial = this.esc((entry.name || entry.id || "?").trim().slice(0, 1).toUpperCase());
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
        "</div>";
      var button = document.createElement("button");
      button.type = "button";
      button.className = "owo-market-cat-install";
      button.setAttribute("data-core-action", "true");
      button.textContent = "安装";
      button.addEventListener("click", function () {
        self.doInstallRemote(entry.id, entry.version || "", "", button);
      });
      row.appendChild(button);
      return row;
    },

    _action: function (key, button, request, success, failure) {
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
          } else if (self.root) {
            self._result("上一页面发起的插件操作已完成，请刷新当前目录确认状态。");
            self.refresh();
            self.refreshAudit();
          }
        })
        .catch(function (error) {
          if (owner === panelGeneration) {
            failure(error);
          } else if (self.root) {
            self._result("上一页面发起的插件操作失败：" + self.friendlyError(error));
          }
        })
        .finally(function () {
          if (actionBusy[key] !== actionToken) return;
          delete actionBusy[key];
          if (owner === panelGeneration && button) {
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
      var self = this;
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
          if (!accepted) return { cancelled: true };
          return self.post("/plugins/market/uninstall", { id: id });
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
          if (el) el.textContent = (((data && data.entries) || []).join("\n")) || "（空）";
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
      if (el) el.textContent = text;
    },

    _root: function () {
      return (this.helpers && this.helpers.root) || this.root || null;
    },

    dispose: function () {
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
