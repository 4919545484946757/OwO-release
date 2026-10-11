// 团队技能包共享面板（Agent 2 子任务 2）：导出/导入评审/版本历史/审计。
(function () {
  "use strict";

  window.OwoPanels = window.OwoPanels || {};

  window.OwoPanels["team"] = {
    id: "team",
    title: "团队技能包",

    nav: function () {
      return (
        '<section data-panel="team" data-layout="custom" class="owo-team-panel">' +
        '<div class="owo-team-tools">' +
        "<h3>导出</h3>" +
        '<div class="inline"><input class="owo-team-export-id" placeholder="技能包 id（本地 store）"><button class="owo-team-exportbtn">导出</button></div>' +
        '<div class="owo-team-export-result sub">—</div>' +
        "</div>" +
        '<div class="owo-team-tools">' +
        "<h3>导入 / 评审</h3>" +
        '<textarea class="owo-team-import-b64" rows="4" spellcheck="false" placeholder="package_b64（base64 打包字节）"></textarea>' +
        '<div class="inline">' +
        '<button class="owo-team-reviewbtn" data-core-action>只评审</button>' +
        '<button class="owo-team-importbtn primary" data-core-action>导入（评审通过才落盘）</button>' +
        "</div>" +
        '<div class="owo-team-findings sub"></div>' +
        "</div>" +
        '<div class="owo-team-tools">' +
        "<h3>版本历史</h3>" +
        '<div class="inline"><input class="owo-team-versions-id" placeholder="技能包 id"><button class="owo-team-versionsbtn">查询</button></div>' +
        '<div class="owo-team-versions sub">—</div>' +
        "</div>" +
        '<div class="owo-team-tools">' +
        "<h3>审计尾部</h3>" +
        '<div class="owo-team-audit sub">—</div>' +
        "</div>" +
        "<style>" +
        ".owo-team-panel { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); align-items: start; gap: 14px; }" +
        ".owo-team-tools { border: 1px solid var(--border, #333); border-radius: 6px; padding: 8px; }" +
        ".owo-team-tools h3 { margin: 0 0 6px; font-size: 13px; }" +
        ".owo-team-findings { max-height: 260px; overflow: auto; }" +
        ".owo-team-versions, .owo-team-audit { max-height: 200px; overflow: auto; }" +
        ".owo-team-panel textarea { width: 100%; box-sizing: border-box; }" +
        ".owo-team-high { color: var(--red, #e5534b); }" +
        ".owo-team-medium { color: var(--yellow, #d29922); }" +
        /* 评审结果卡片 */
        ".owo-team-pkg{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;margin-bottom:6px}" +
        ".owo-team-pkg strong{font-size:13px}" +
        ".owo-team-badge{display:inline-block;padding:1px 8px;border-radius:10px;font-size:11.5px}" +
        ".owo-team-badge-ok{background:var(--green-soft,#1a3d2f);color:var(--green,#3fb950)}" +
        ".owo-team-badge-bad{background:var(--red-soft,#3d1a1a);color:var(--red,#f85149)}" +
        ".owo-team-finding{display:flex;gap:8px;align-items:baseline;padding:5px 8px;border:1px solid var(--border,#333);border-radius:6px;margin-bottom:4px;background:var(--surface,#1c2128)}" +
        ".owo-team-finding-cat{flex:none;font-weight:600;font-size:12px}" +
        ".owo-team-finding-detail{flex:1;min-width:0;font-size:12px;color:var(--text-2,#adbac7);word-break:break-word}" +
        ".owo-team-finding-sev{flex:none;font-size:12px;padding:1px 7px;border-radius:9px}" +
        ".owo-team-sev-high{color:var(--red,#f85149);background:var(--red-soft,#3d1a1a)}" +
        ".owo-team-sev-medium{color:var(--yellow,#d29922);background:rgba(210,153,34,.15)}" +
        ".owo-team-sev-low{color:var(--text-3,#768390);background:var(--surface-2,#22272e)}" +
        /* 版本时间线 */
        ".owo-team-ver{display:flex;gap:9px;align-items:baseline;padding:5px 8px;border:1px solid var(--border,#333);border-radius:6px;margin-bottom:4px;background:var(--surface,#1c2128)}" +
        ".owo-team-ver-v{flex:none;font-weight:700;font-size:12px;color:var(--accent,#58a6ff)}" +
        ".owo-team-ver-time{flex:none;color:var(--text-3,#768390);font-size:11.5px}" +
        ".owo-team-ver-sha{flex:1;min-width:0;font-size:11.5px;color:var(--text-2,#adbac7)}" +
        ".owo-team-ver-sha code{background:var(--surface-2,#22272e);border-radius:4px;padding:0 4px}" +
        /* 审计时间线 */
        ".owo-team-audit-line{display:flex;gap:8px;align-items:baseline;padding:3px 0;border-bottom:1px dashed var(--border,#333);font-size:12px}" +
        ".owo-team-audit-line time{flex:none;color:var(--text-3,#768390);font-size:12px}" +
        ".owo-team-audit-line span{flex:1;min-width:0;word-break:break-word;color:var(--text-2,#adbac7)}" +
        "@media (max-width: 760px) { .owo-team-panel { grid-template-columns: minmax(0, 1fr); } }" +
        "</style>" +
        "</section>"
      );
    },

    mount: function (root, helpers) {
      this.dispose();
      this.lifecycleGeneration = (this.lifecycleGeneration || 0) + 1;
      this.root = root;
      var self = this;
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

      $(".owo-team-exportbtn").addEventListener("click", function () {
        self.doExport($(".owo-team-export-id").value);
      });
      $(".owo-team-reviewbtn").addEventListener("click", function () {
        self.doReview($(".owo-team-import-b64").value);
      });
      $(".owo-team-importbtn").addEventListener("click", function () {
        self.doImport($(".owo-team-import-b64").value);
      });
      $(".owo-team-versionsbtn").addEventListener("click", function () {
        self.doVersions($(".owo-team-versions-id").value);
      });
      var importButton = $(".owo-team-importbtn");
      if (this.importBusy) {
        importButton.disabled = true;
        importButton.textContent = "正在导入…";
      }

      this.refreshAudit();
    },

    dispose: function () {
      this.lifecycleGeneration = (this.lifecycleGeneration || 0) + 1;
      this.root = null;
    },

    refresh: function () {
      this.refreshAudit();
    },

    doExport: function (id) {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.root;
      var request = this.exportGeneration = (this.exportGeneration || 0) + 1;
      if (!id) return this._findings("请填写技能包 id");
      this.post("/team/export", { type: "flow", id: id })
        .then(function (data) {
          if (!self._isCurrent(generation, root) || request !== self.exportGeneration) return;
          var el = root.querySelector(".owo-team-export-result");
          if (el) {
            var m = data.manifest || {};
            el.innerHTML =
              '<div class="owo-team-pkg"><strong>' + self.esc(m.id) + '</strong><span class="sub">v' + self.esc(m.version) + '</span>' +
              '<span class="owo-team-badge owo-team-badge-ok">导出成功</span></div>' +
              '<div class="sub">' + self.esc(data.size_bytes) + " 字节 · base64 " + self.esc(data.package_b64.length) + " 字符</div>";
          }
          self.refreshAudit();
        })
        .catch(function (error) {
          if (!self._isCurrent(generation, root) || request !== self.exportGeneration) return;
          self._findings("导出失败：" + self.friendlyError(error));
        });
    },

    doReview: function (b64) {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.root;
      var request = this.reviewGeneration = (this.reviewGeneration || 0) + 1;
      if (!b64) return this._findings("请填写 package_b64");
      this.post("/team/review", { package_b64: b64 })
        .then(function (data) {
          if (!self._isCurrent(generation, root) || request !== self.reviewGeneration) return;
          self.renderFindings(data);
        })
        .catch(function (error) {
          if (!self._isCurrent(generation, root) || request !== self.reviewGeneration) return;
          self._findings("评审失败：" + self.friendlyError(error));
        });
    },

    doImport: function (b64) {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.root;
      if (!b64) return this._findings("请填写 package_b64");
      if (this.importBusy) return;
      this.importBusy = true;
      var importButton = root && root.querySelector(".owo-team-importbtn");
      if (importButton) {
        importButton.disabled = true;
        importButton.textContent = "正在导入…";
      }
      this.post("/team/import", { package_b64: b64 })
        .then(function (data) {
          if (!self._isCurrent(generation, root)) {
            if (self.root) {
              self._findings(data.blocked
                ? "后台评审已完成并阻止导入；请重新查看当前结果。"
                : "导入已在后台完成；可查询版本历史确认。");
              self.refreshAudit();
            }
            return;
          }
          if (data.blocked) {
            self.renderFindings(data);
            return;
          }
          var pkg = data.package || {};
          self._findingsHtml(
            '<div class="owo-team-pkg"><strong>' + self.esc(pkg.id) + '</strong><span class="sub">v' + self.esc(pkg.version) + '</span>' +
              '<span class="owo-team-badge owo-team-badge-ok">导入成功</span></div>' +
              '<div class="sub">版本历史 ' + self.esc(String((data.versions || []).length)) + " 条</div>"
          );
          self.refreshAudit();
        })
        .catch(function (error) {
          if (!self._isCurrent(generation, root)) return;
          self._findings("导入失败：" + self.friendlyError(error));
        })
        .finally(function () {
          self.importBusy = false;
          var activeRoot = self.root;
          if (activeRoot && (typeof activeRoot.isConnected !== "boolean" || activeRoot.isConnected)) {
            var button = activeRoot.querySelector(".owo-team-importbtn");
            if (button) {
              button.disabled = false;
              button.textContent = "导入（评审通过才落盘）";
            }
          }
        });
    },

    renderFindings: function (data) {
      var esc = this.esc;
      var pkg = data.package || {};
      var blocked = !!data.blocked;
      var html = "";
      if (pkg.id) {
        html +=
          '<div class="owo-team-pkg"><strong>' + esc(pkg.id) + '</strong><span class="sub">v' + esc(pkg.version) + "</span>" +
          (blocked
            ? '<span class="owo-team-badge owo-team-badge-bad">已阻断</span>'
            : '<span class="owo-team-badge owo-team-badge-ok">通过</span>') +
          "</div>";
      } else {
        html +=
          '<div class="owo-team-pkg">' +
          (blocked
            ? '<span class="owo-team-badge owo-team-badge-bad">已阻断</span>'
            : '<span class="owo-team-badge owo-team-badge-ok">通过</span>') +
          "</div>";
      }
      var findings = data.findings || [];
      if (!findings.length) {
        html += '<div class="sub">无 findings（通过）</div>';
      } else {
        html += findings
          .map(function (f) {
            var sev = f.severity === "high" ? "high" : f.severity === "medium" ? "medium" : "low";
            return (
              '<div class="owo-team-finding">' +
              '<span class="owo-team-finding-cat">' + esc(f.category) + "</span>" +
              '<span class="owo-team-finding-detail">' + esc(f.detail) + "</span>" +
              '<span class="owo-team-finding-sev owo-team-sev-' + sev + '">' + esc(f.severity) + "</span>" +
              "</div>"
            );
          })
          .join("");
      }
      this._findingsHtml(html);
    },

    doVersions: function (id) {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.root;
      var request = this.versionsGeneration = (this.versionsGeneration || 0) + 1;
      if (!id) return this._findings("请填写技能包 id");
      this.get("/team/versions?id=" + encodeURIComponent(id))
        .then(function (data) {
          if (!self._isCurrent(generation, root) || request !== self.versionsGeneration) return;
          var el = root.querySelector(".owo-team-versions");
          if (!el) return;
          var versions = data.versions || [];
          if (!versions.length) {
            el.innerHTML = '<div class="sub">无版本记录</div>';
            return;
          }
          el.innerHTML =
            '<div class="sub" style="margin-bottom:6px">共 ' + self.esc(String(data.count == null ? versions.length : data.count)) + " 个版本</div>" +
            versions
              .map(function (v) {
                return (
                  '<div class="owo-team-ver">' +
                  '<span class="owo-team-ver-v">v' + self.esc(v.version) + "</span>" +
                  '<span class="owo-team-ver-time">' + self.esc(v.imported_at) + "</span>" +
                  '<span class="owo-team-ver-sha">sha <code>' + self.esc(String(v.sha256).slice(0, 12)) + "…</code></span>" +
                  "</div>"
                );
              })
              .join("");
        })
        .catch(function (error) {
          if (!self._isCurrent(generation, root) || request !== self.versionsGeneration) return;
          self._findings("版本查询失败：" + self.friendlyError(error));
        });
    },

    refreshAudit: function () {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.root;
      var request = this.auditGeneration = (this.auditGeneration || 0) + 1;
      this.get("/team/audit")
        .then(function (data) {
          if (!self._isCurrent(generation, root) || request !== self.auditGeneration) return;
          var el = root.querySelector(".owo-team-audit");
          if (!el) return;
          var entries = data.entries || [];
          el.innerHTML = entries.length
            ? entries
                .map(function (line) {
                  var s = String(line);
                  var m = s.match(/^\[([^\]]+)\]\s*(.*)$/);
                  return m
                    ? '<div class="owo-team-audit-line"><time>' + self.esc(m[1]) + "</time><span>" + self.esc(m[2]) + "</span></div>"
                    : '<div class="owo-team-audit-line"><span>' + self.esc(s) + "</span></div>";
                })
                .join("")
            : '<div class="sub">（空）</div>';
        })
        .catch(function () {
          /* 审计读取失败不阻断 */
        });
    },

    _findings: function (text) {
      var el = this._root().querySelector(".owo-team-findings");
      if (el) el.textContent = text;
    },

    _findingsHtml: function (html) {
      var el = this._root().querySelector(".owo-team-findings");
      if (el) el.innerHTML = html;
    },

    _root: function () {
      return this.root || this.helpers.root || document;
    },

    _isCurrent: function (generation, root) {
      return generation === this.lifecycleGeneration && root === this.root && !!root &&
        (typeof root.isConnected !== "boolean" || root.isConnected);
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
