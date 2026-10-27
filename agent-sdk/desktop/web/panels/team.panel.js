// 共享技能：同一 Daemon 内的导出、评审、导入、版本和审计。
(function () {
  "use strict";
  window.OwoPanels = window.OwoPanels || {};
  window.OwoPanels.team = {
    id: "team",
    title: "共享技能",
    tabs: ["import", "export", "versions", "audit"],
    fields: [".owo-team-export-id", ".owo-team-import-b64", ".owo-team-versions-id"],
    nav: function () {
      var tabs = this.tabs.map(function (id, i) {
        return '<button type="button" role="tab" id="team-tab-' + id + '" aria-controls="team-pane-' + id +
          '" data-team-tab="' + id + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' +
          ["导入与评审", "导出", "版本历史", "审计"][i] + '</button>';
      }).join("");
      return '<section data-panel="team" data-layout="custom" class="owo-team-panel owo-insight">' +
        '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="共享技能">' + tabs +
        '</div><button type="button" class="owo-team-audit-refresh">刷新审计</button></div>' +
        '<section id="team-pane-import" role="tabpanel" aria-labelledby="team-tab-import">' +
        '<h3>导入技能包</h3><p class="sub">粘贴技能包编码，或选择技能包文件（.owskill / JSON，最大 20 MiB）。</p>' +
        '<div class="owo-extension-file"><button type="button" class="owo-team-choose">选择文件</button>' +
        '<input type="file" class="owo-team-file" accept=".owskill,.json" hidden>' +
        '<span class="owo-team-file-status sub" role="status">未选择文件</span></div>' +
        '<label for="team-package">技能包编码</label><textarea id="team-package" class="owo-team-import-b64" rows="8" spellcheck="false" placeholder="粘贴 package_b64"></textarea>' +
        '<div class="owo-insight-actions"><button type="button" class="owo-team-reviewbtn" data-core-action>只评审</button>' +
        '<button type="button" class="owo-team-importbtn primary" data-core-action>导入（评审通过才落盘）</button></div>' +
        '<section class="owo-insight-section"><h3>评审结果</h3><div class="owo-team-findings sub" role="status" aria-live="polite">尚未评审</div></section></section>' +
        '<section id="team-pane-export" role="tabpanel" aria-labelledby="team-tab-export" hidden><h3>导出技能包</h3>' +
        '<p class="sub">从本地技能库导出 .owskill 文件。</p><label for="team-export-id">技能包 ID 或名称</label>' +
        '<div class="owo-extension-input-row"><input id="team-export-id" class="owo-team-export-id" placeholder="本地技能包 ID 或名称">' +
        '<button type="button" class="owo-team-exportbtn" data-core-action>导出</button></div>' +
        '<div class="owo-team-export-result sub" role="status">尚未导出</div>' +
        '<div class="owo-team-export-actions owo-insight-actions" hidden><button type="button" class="owo-team-copy">复制编码</button>' +
        '<button type="button" class="owo-team-download">下载技能包</button><button type="button" class="owo-team-use primary">用于评审</button></div>' +
        '<textarea class="owo-team-export-code" aria-label="导出的技能包编码" readonly hidden></textarea></section>' +
        '<section id="team-pane-versions" role="tabpanel" aria-labelledby="team-tab-versions" hidden><h3>版本历史</h3>' +
        '<p class="sub">查询已导入技能包的版本与校验值。</p><label for="team-versions-id">技能包 ID</label>' +
        '<div class="owo-extension-input-row"><input id="team-versions-id" class="owo-team-versions-id" placeholder="技能包 ID">' +
        '<button type="button" class="owo-team-versionsbtn">查询</button></div><div class="owo-team-versions sub" role="status">输入 ID 后查询</div></section>' +
        '<section id="team-pane-audit" role="tabpanel" aria-labelledby="team-tab-audit" hidden><h3>最近审计</h3>' +
        '<p class="sub">最近 50 条技能包操作记录。</p><div class="owo-team-audit sub" role="status">加载中…</div></section></section>';
    },
    mount: function (root, helpers) {
      this.dispose();
      this.helpers = helpers || {};
      this.baseUrl = this.helpers.baseUrl || window.OwoPanels.baseUrl || window.location.origin;
      var scope = this.baseUrl + "#" + (this.helpers.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
      if (scope !== this.scope) { this.drafts = {}; this.activeTab = "import"; this.exported = null; }
      this.scope = scope;
      this.importLocks = this.importLocks || Object.create(null);
      this.importBusy = !!this.importLocks[scope];
      this.get = this.helpers.get || this._get;
      this.post = this.helpers.post || this._post;
      this.esc = this.helpers.esc || this._esc;
      this.friendlyError = this.helpers.friendlyError || this._friendlyError;
      this.root = root;
      root.innerHTML = this.nav();
      var self = this, lifecycle = this.lifecycleGeneration;
      var $ = function (selector) { return root.querySelector(selector); };
      var bind = function (selector, event, action) {
        var el = $(selector);
        if (el) el.addEventListener(event, function (e) { if (self._isCurrent(lifecycle, root)) action(e); });
      };
      this.fields.forEach(function (selector) {
        var el = $(selector); if (el && self.drafts && self.drafts[selector] != null) el.value = self.drafts[selector];
      });
      this.tabs.forEach(function (id, index) {
        bind('[data-team-tab="' + id + '"]', "click", function () { self.selectTab(id); });
        bind('[data-team-tab="' + id + '"]', "keydown", function (e) {
          var next = e.key === "ArrowRight" ? (index + 1) % 4 : e.key === "ArrowLeft" ? (index + 3) % 4 : e.key === "Home" ? 0 : e.key === "End" ? 3 : -1;
          if (next < 0) return;
          e.preventDefault(); self.selectTab(self.tabs[next]);
          $('[data-team-tab="' + self.tabs[next] + '"]').focus();
        });
      });
      bind(".owo-team-exportbtn", "click", function () { self.doExport($(".owo-team-export-id").value); });
      bind(".owo-team-reviewbtn", "click", function () { self.doReview($(".owo-team-import-b64").value); });
      bind(".owo-team-importbtn", "click", function () { self.doImport($(".owo-team-import-b64").value); });
      bind(".owo-team-versionsbtn", "click", function () { self.doVersions($(".owo-team-versions-id").value); });
      bind(".owo-team-audit-refresh", "click", function () { self.refreshAudit(); });
      bind(".owo-team-choose", "click", function () { $(".owo-team-file").click(); });
      bind(".owo-team-file", "change", function () { self.readFile($(".owo-team-file").files[0]); });
      bind(".owo-team-import-b64", "input", function () { self.inputChanged(); });
      bind(".owo-team-copy", "click", function () { self.copyExport(); });
      bind(".owo-team-download", "click", function () { self.downloadExport(); });
      bind(".owo-team-use", "click", function () {
        if (!self.exported || self.importBusy) return;
        self.inputChanged();
        $(".owo-team-import-b64").value = self.exported.package_b64;
        $(".owo-team-file-status").textContent = "来自本页导出的技能包";
        self.selectTab("import");
      });
      this.selectTab(this.activeTab || "import");
      this._importButtons(this.importBusy);
      if (this.exported) this.renderExport();
      this.refreshAudit();
    },
    selectTab: function (id) {
      if (!this.root || this.tabs.indexOf(id) < 0) return;
      this.activeTab = id;
      var self = this;
      this.tabs.forEach(function (tab) {
        var button = self.root.querySelector('[data-team-tab="' + tab + '"]');
        if (button && button.setAttribute) { button.setAttribute("aria-selected", String(tab === id)); button.setAttribute("tabindex", tab === id ? "0" : "-1"); }
        var pane = self.root.querySelector("#team-pane-" + tab); if (pane) pane.hidden = tab !== id;
      });
    },
    dispose: function () {
      if (this.root) {
        var self = this; this.drafts = {};
        this.fields.forEach(function (selector) { var el = self.root.querySelector(selector); if (el) self.drafts[selector] = el.value; });
      }
      this.lifecycleGeneration = (this.lifecycleGeneration || 0) + 1;
      this.fileGeneration = (this.fileGeneration || 0) + 1;
      this.root = null;
    },
    refresh: function () { return this.refreshAudit(); },
    _busy: function (button, busy, text) {
      if (!button) return;
      if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, busy);
      button.disabled = busy;
      if (button.setAttribute) button.setAttribute("aria-busy", String(busy));
      if (text) button.textContent = text;
    },
    _importButtons: function (busy) {
      if (!this.root) return;
      this._busy(this.root.querySelector(".owo-team-importbtn"), busy, busy ? "正在导入…" : "导入（评审通过才落盘）");
      this._busy(this.root.querySelector(".owo-team-reviewbtn"), busy, "只评审");
      this.root.querySelector(".owo-team-import-b64").readOnly = busy;
      this.root.querySelector(".owo-team-choose").disabled = busy;
    },
    _text: function (selector, text) { var el = this.root && this.root.querySelector(selector); if (el) el.textContent = text; },
    inputChanged: function () {
      this.fileGeneration = (this.fileGeneration || 0) + 1;
      this.reviewGeneration = (this.reviewGeneration || 0) + 1;
      if (!this.importBusy && this.root) this._busy(this.root.querySelector(".owo-team-reviewbtn"), false);
      this._findings("编码已更改，尚未评审");
    },
    readFile: function (file) {
      if (!file || !this.root || this.importBusy) return Promise.resolve();
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration;
      var request = this.fileGeneration = (this.fileGeneration || 0) + 1;
      var input = root.querySelector(".owo-team-import-b64"), before = input.value;
      if (file.size > 20 * 1024 * 1024) { this._text(".owo-team-file-status", "文件超过 20 MiB，请选择较小的技能包。"); return Promise.resolve(); }
      this._text(".owo-team-file-status", "正在读取 " + file.name + "…");
      var pending = Promise.resolve().then(function () {
        if (/\.json$/i.test(file.name)) return file.text().then(function (raw) {
          var data = JSON.parse(raw); if (typeof data.package_b64 !== "string" || !data.package_b64.trim()) throw new Error("JSON 缺少 package_b64");
          return data.package_b64.trim();
        });
        if (!/\.owskill$/i.test(file.name)) throw new Error("请选择 .owskill 或 JSON 文件");
        return file.arrayBuffer().then(function (buffer) {
          var bytes = new Uint8Array(buffer), binary = "";
          for (var i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 32768));
          if (!bytes.length) throw new Error("文件为空");
          return window.btoa(binary);
        });
      });
      return pending.then(function (b64) {
        if (!self._isCurrent(lifecycle, root) || request !== self.fileGeneration || input.value !== before) return;
        self.inputChanged();
        input.value = b64;
        self._text(".owo-team-file-status", file.name + " · 已读取，尚未提交");
      }).catch(function (error) {
        if (!self._isCurrent(lifecycle, root) || request !== self.fileGeneration) return;
        self._text(".owo-team-file-status", "文件读取失败：" + self.friendlyError(error));
      });
    },
    doExport: function (id) {
      if (!this.root) return Promise.resolve();
      id = String(id || "").trim();
      if (!id) { this._text(".owo-team-export-result", "请填写技能包 ID 或名称"); return Promise.resolve(); }
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration;
      var request = this.exportGeneration = (this.exportGeneration || 0) + 1;
      this.exported = null;
      root.querySelector(".owo-team-export-actions").hidden = true;
      var code = root.querySelector(".owo-team-export-code"); code.hidden = true; code.value = "";
      this._text(".owo-team-export-result", "正在导出…");
      var button = root.querySelector(".owo-team-exportbtn"); this._busy(button, true);
      return this.post("/team/export", { type: "flow", id: id }).then(function (data) {
        if (!self._isCurrent(lifecycle, root) || request !== self.exportGeneration) return;
        if (!data || typeof data.package_b64 !== "string" || !data.package_b64 || !data.manifest || !data.manifest.id) throw new Error("技能包响应不完整");
        self.exported = data; self.renderExport(); self.refreshAudit();
      }).catch(function (error) {
        if (self._isCurrent(lifecycle, root) && request === self.exportGeneration) self._text(".owo-team-export-result", "导出失败：" + self.friendlyError(error));
      }).finally(function () { if (self._isCurrent(lifecycle, root) && request === self.exportGeneration) self._busy(button, false); });
    },
    renderExport: function () {
      if (!this.root || !this.exported) return;
      var data = this.exported, m = data.manifest;
      this._text(".owo-team-export-result", m.id + " · v" + (m.version || "—") + " · " + (data.size_bytes == null ? "大小未知" : data.size_bytes + " 字节") + " · 导出成功");
      this.root.querySelector(".owo-team-export-actions").hidden = false;
    },
    copyExport: function () {
      if (!this.root || !this.exported) return Promise.resolve();
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration, data = this.exported;
      return Promise.resolve().then(function () {
        if (!window.navigator || !window.navigator.clipboard) throw new Error("剪贴板不可用");
        return window.navigator.clipboard.writeText(data.package_b64);
      }).then(function () {
        if (self._isCurrent(lifecycle, root) && self.exported === data) self._text(".owo-team-export-result", "技能包编码已复制");
      }).catch(function () {
        if (!self._isCurrent(lifecycle, root) || self.exported !== data) return;
        var code = root.querySelector(".owo-team-export-code"); code.value = data.package_b64; code.hidden = false;
        if (code.focus) code.focus(); if (code.select) code.select();
        self._text(".owo-team-export-result", "复制不可用，已展开编码，可手动复制。");
      });
    },
    downloadExport: function () {
      if (!this.root || !this.exported) return;
      try {
        var data = this.exported, binary = window.atob(data.package_b64);
        var bytes = Uint8Array.from(binary, function (c) { return c.charCodeAt(0); });
        var url = window.URL.createObjectURL(new window.Blob([bytes], { type: "application/zip" }));
        var link = document.createElement("a");
        link.href = url; link.download = String(data.manifest.name || data.manifest.id).replace(/[^\w\u4e00-\u9fff.-]/g, "_") + ".owskill";
        link.click(); window.setTimeout(function () { window.URL.revokeObjectURL(url); }, 1000);
        this._text(".owo-team-export-result", "技能包已交给浏览器下载");
      } catch (error) { this._text(".owo-team-export-result", "下载失败：" + this.friendlyError(error)); }
    },
    doReview: function (b64) {
      if (!this.root || this.importBusy) return Promise.resolve();
      if (!String(b64 || "").trim()) { this._findings("请填写 package_b64"); return Promise.resolve(); }
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration;
      var request = this.reviewGeneration = (this.reviewGeneration || 0) + 1;
      var button = root.querySelector(".owo-team-reviewbtn"); this._busy(button, true);
      return this.post("/team/review", { package_b64: b64.trim() }).then(function (data) {
        if (!self._isCurrent(lifecycle, root) || request !== self.reviewGeneration) return;
        self.renderFindings(data);
      }).catch(function (error) {
        if (self._isCurrent(lifecycle, root) && request === self.reviewGeneration) self._findings("评审失败：" + self.friendlyError(error));
      }).finally(function () {
        if (self._isCurrent(lifecycle, root) && request === self.reviewGeneration && !self.importBusy) self._busy(button, false);
      });
    },
    doImport: function (b64) {
      if (!this.root || this.importBusy) return Promise.resolve();
      if (!String(b64 || "").trim()) { this._findings("请填写 package_b64"); return Promise.resolve(); }
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration, scope = this.scope, lock = {};
      this.importLocks[scope] = lock; this.importBusy = true;
      this.reviewGeneration = (this.reviewGeneration || 0) + 1;
      this._importButtons(true);
      return this.post("/team/import", { package_b64: b64.trim() }).then(function (data) {
        if (!data || typeof data.blocked !== "boolean" || !data.package) throw new Error("导入响应不完整，请查询版本历史确认");
        if (scope !== self.scope) return;
        if (!self._isCurrent(lifecycle, root)) {
          if (self.root) { self._findings(data.blocked ? "后台评审已完成并阻止导入；请重新评审确认结果。" : "导入已在后台完成；可查询版本历史确认。"); self.refreshAudit(); }
          return;
        }
        self.renderFindings(data);
        if (!data.blocked) {
          var pkg = data.package;
          self._findingsHtml('<div class="owo-team-pkg"><strong>' + self.esc(pkg.id) + '</strong><span>v' + self.esc(pkg.version) +
            '</span><span class="owo-team-badge owo-team-badge-ok">导入成功</span></div><div class="sub">版本历史 ' +
            self.esc(Array.isArray(data.versions) ? data.versions.length : "未知") + ' 条</div>');
          root.querySelector(".owo-team-versions-id").value = pkg.id || "";
        }
        self.refreshAudit();
      }).catch(function (error) {
        if (scope === self.scope && self.root) self._findings("导入失败：" + self.friendlyError(error));
      }).finally(function () {
        if (self.importLocks[scope] !== lock) return;
        delete self.importLocks[scope];
        if (scope === self.scope) { self.importBusy = false; self._importButtons(false); }
      });
    },
    renderFindings: function (data) {
      if (!data || typeof data.blocked !== "boolean" || !Array.isArray(data.findings)) throw new Error("评审响应不完整，无法确认是否通过");
      var esc = this.esc, pkg = data.package || {}, blocked = data.blocked;
      var html = '<div class="owo-team-pkg"><strong>' + esc(pkg.id || "技能包") + '</strong><span>' + esc(pkg.version ? "v" + pkg.version : "") +
        '</span><span class="owo-team-badge ' + (blocked ? "owo-team-badge-bad" : "owo-team-badge-ok") + '">' + (blocked ? "已阻断" : "通过") + '</span></div>';
      html += data.findings.length ? data.findings.map(function (f) {
        var sev = ["high", "medium", "low"].indexOf(f.severity) >= 0 ? f.severity : "unknown";
        return '<div class="owo-team-finding"><strong>' + esc(f.category) + '</strong><span>' + esc(f.detail) +
          '</span><span class="owo-team-sev-' + sev + '">' + esc(f.severity || "未知") + '</span></div>';
      }).join("") : '<div class="sub">' + (blocked ? "服务已阻止导入，未提供详细问题。" : "未发现评审问题") + '</div>';
      this._findingsHtml(html);
    },
    doVersions: function (id) {
      if (!this.root) return Promise.resolve();
      if (!String(id || "").trim()) { this._text(".owo-team-versions", "请填写技能包 ID"); return Promise.resolve(); }
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration;
      var request = this.versionsGeneration = (this.versionsGeneration || 0) + 1;
      this._text(".owo-team-versions", "正在查询…");
      return this.get("/team/versions?id=" + encodeURIComponent(id.trim())).then(function (data) {
        if (!self._isCurrent(lifecycle, root) || request !== self.versionsGeneration) return;
        if (!data || !Array.isArray(data.versions)) throw new Error("版本响应不完整");
        root.querySelector(".owo-team-versions").innerHTML = data.versions.length ? data.versions.map(function (v) {
          return '<div class="owo-team-ver"><strong>v' + self.esc(v.version) + '</strong><time>' + self.esc(self._time(v.imported_at)) +
            '</time><code title="' + self.esc(v.sha256) + '">' + self.esc(String(v.sha256 || "").slice(0, 12)) + '</code></div>';
        }).join("") : '<div class="owo-insight-empty">无版本记录</div>';
      }).catch(function (error) {
        if (self._isCurrent(lifecycle, root) && request === self.versionsGeneration) self._text(".owo-team-versions", "版本查询失败：" + self.friendlyError(error));
      });
    },
    _time: function (value) {
      var n = Number(value), d = typeof value === "number" || /^\d+$/.test(String(value)) ? new Date(n * 1000) : new Date(value);
      return isNaN(d.getTime()) ? String(value || "—") : d.toLocaleString();
    },
    refreshAudit: function () {
      if (!this.root) return Promise.resolve();
      var self = this, root = this.root, lifecycle = this.lifecycleGeneration;
      var request = this.auditGeneration = (this.auditGeneration || 0) + 1;
      return this.get("/team/audit").then(function (data) {
        if (!self._isCurrent(lifecycle, root) || request !== self.auditGeneration) return;
        if (!data || !Array.isArray(data.entries)) throw new Error("审计响应不完整");
        root.querySelector(".owo-team-audit").innerHTML = data.entries.length ? data.entries.map(function (line) {
          var m = String(line).match(/^\[(\d+)\]\s*(.*)$/);
          return '<div class="owo-team-audit-line">' + (m ? '<time>' + self.esc(self._time(m[1])) + '</time><span>' + self.esc(m[2]) : '<span>' + self.esc(line)) + '</span></div>';
        }).join("") : '<div class="owo-insight-empty">暂无审计记录</div>';
      }).catch(function (error) {
        if (self._isCurrent(lifecycle, root) && request === self.auditGeneration) self._text(".owo-team-audit", "审计读取失败：" + self.friendlyError(error) + "。可点击刷新审计重试。");
      });
    },
    _findings: function (text) { this._text(".owo-team-findings", text); },
    _findingsHtml: function (html) { var el = this.root && this.root.querySelector(".owo-team-findings"); if (el) el.innerHTML = html; },
    _root: function () { return this.root; },
    _isCurrent: function (generation, root) {
      return generation === this.lifecycleGeneration && root === this.root && !!root && (typeof root.isConnected !== "boolean" || root.isConnected);
    },

    // ---- helpers 缺省实现 ----

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
