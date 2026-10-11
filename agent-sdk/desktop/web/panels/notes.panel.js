/* notes 面板（Lane A）：文档列表 + 新建（标题+markdown）+ 块树渲染 + 搜索 + 导出 + 内联编辑。
 * 纯脚本 IIFE 注册；helpers 缺省时复用统一 ApiClient；样式 owo-notes- 前缀，mount 时注入 <style>。 */
(function () {
  window.OwoPanels = window.OwoPanels || {};
  window.OwoPanels.notes = {
    id: "notes",
    title: "笔记",

    nav: function () {
      return (
        '<section data-panel="notes" class="owo-notes-root">' +
        '<div class="owo-notes-bar">' +
        '<input class="owo-notes-search" type="search" aria-label="搜索笔记全文" aria-describedby="owo-notes-core-hint" data-core-action placeholder="全文搜索…">' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-new" data-core-action aria-label="新建笔记">＋ 新建</button>' +
        "</div>" +
        '<p class="owo-notes-hint" id="owo-notes-core-hint" data-core-action-hint>连接并授权本地核心后可搜索笔记全文和管理笔记。</p>' +
        '<ul class="owo-notes-list"></ul>' +
        '<div class="owo-notes-editor" hidden>' +
        '<input class="owo-notes-title" type="text" aria-label="笔记标题" placeholder="标题" required>' +
        '<textarea class="owo-notes-md" aria-label="Markdown 正文" rows="8" placeholder="Markdown 正文"></textarea>' +
        '<div class="owo-notes-editor-actions">' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-save" data-core-action>保存</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-cancel">取消</button>' +
        "</div>" +
        "</div>" +
        '<div class="owo-notes-detail" hidden>' +
        '<div class="owo-notes-detail-head"><h3 class="owo-notes-detail-title"></h3>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-rename" data-core-action>修改标题</button></div>' +
        '<div class="owo-notes-detail-actions">' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-export-md">导出 MD</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-export-html">导出 HTML</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-del" data-core-action>删除</button>' +
        "</div>" +
        '<div class="owo-notes-detail-meta">' +
        '<span class="owo-notes-meta-item" id="owo-notes-detail-count"></span>' +
        "</div>" +
        '<div class="owo-notes-tree"></div>' +
        "</div>" +
        "</section>"
      );
    },

    mount: function (root, helpers) {
      var self = this;
      this.dispose();
      this.lifecycleGeneration = (this.lifecycleGeneration || 0) + 1;
      this.listGeneration = 0;
      this.detailGeneration = 0;
      this.editorGeneration = 0;
      this.root = root;
      root.innerHTML = this.nav();
      this.setSaveBusy(root, !!this.saveInFlight);
      this.helpers = helpers || {};
      this.baseUrl = this.helpers.baseUrl || window.OwoPanels.baseUrl || window.location.origin;
      this.get = this.helpers.get || function (path) {
        return window.OwoApi.stream(path).then(function (r) { return r.json(); });
      };
      this.post = this.helpers.post || function (path, body) {
        return window.OwoApi.stream(path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body || {}),
        }).then(function (r) { return r.json(); });
      };
      this.put = this.helpers.put || function (path, body) {
        return window.OwoApi.stream(path, {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body || {}),
        }).then(function (r) { return r.json(); });
      };
      this.del = this.helpers.del || function (path) {
        return window.OwoApi.stream(path, { method: "DELETE" }).then(function (r) { return r.json(); });
      };
      // 宿主提供样式化弹窗时优先使用（Promise<boolean> / Promise<string|null>）
      this.confirm = this.helpers.confirm || function (opts) {
        return Promise.resolve(window.confirm((opts && opts.message) || ""));
      };
      this.prompt = this.helpers.prompt || function (opts) {
        return Promise.resolve(window.prompt((opts && opts.label) || "", (opts && opts.value) || ""));
      };
      this.esc = this.helpers.esc || function (s) {
        return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
          return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
        });
      };
      this.friendlyError = this.helpers.friendlyError || function (e) { return String(e); };
      var style = document.createElement("style");
      style.textContent =
        ".owo-notes-root{display:flex;flex-direction:column;gap:8px}" +
        ".owo-notes-bar{display:flex;gap:8px}" +
        ".owo-notes-search{flex:1;padding:6px}" +
        ".owo-notes-btn{padding:6px 10px;cursor:pointer}" +
        ".owo-notes-list{margin:0;padding:0;list-style:none}" +
        ".owo-notes-list li{padding:6px 4px;border-bottom:1px solid var(--border);cursor:pointer;display:flex;justify-content:space-between}" +
        ".owo-notes-list li:hover{background:var(--surface-2)}" +
        ".owo-notes-open{display:flex;width:100%;justify-content:space-between;gap:12px;padding:4px;border:0;background:transparent;color:inherit;text-align:left;cursor:pointer;font:inherit}" +
        ".owo-notes-open:focus-visible,.owo-notes-btn:focus-visible{outline:2px solid var(--accent);outline-offset:2px}" +
        ".owo-notes-detail-head{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}" +
        ".owo-notes-list li.owo-notes-state{list-style:none;cursor:default;display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px;border:1px solid var(--border);border-radius:8px;background:var(--surface-1);color:var(--text-2)}" +
        ".owo-notes-list li.owo-notes-state:hover{background:var(--surface-1)}" +
        ".owo-notes-retry{flex:none;border:1px solid var(--border-strong);border-radius:6px;padding:5px 10px;background:var(--surface);color:var(--text);cursor:pointer}" +
        ".owo-notes-retry:hover{border-color:var(--accent);color:var(--accent)}" +
        ".owo-notes-editor{display:flex;flex-direction:column;gap:8px;border:1px solid var(--border-strong);padding:10px}" +
        ".owo-notes-editor[hidden]{display:none}" +
        ".owo-notes-title,.owo-notes-md{padding:6px}" +
        ".owo-notes-detail{border:1px solid var(--border-strong);padding:10px}" +
        // 块树（原 pre JSON 直出 → 嵌套树卡片）
        ".owo-notes-detail-meta{display:flex;gap:12px;flex-wrap:wrap;margin:6px 0;color:var(--text-3);font-size:12px}" +
        ".owo-notes-tree{background:var(--surface-2);border-radius:8px;padding:8px;max-height:50vh;overflow:auto;font-size:12px}" +
        ".owo-notes-block-tree,.owo-notes-block-tree ul{list-style:none;margin:0;padding:0}" +
        ".owo-notes-block-tree ul{margin-left:14px;border-left:1px solid var(--border);padding-left:8px}" +
        ".owo-notes-block{margin:2px 0}" +
        ".owo-notes-block-row{display:flex;gap:8px;align-items:baseline;padding:2px 0}" +
        ".owo-notes-kind{flex:none;font-size:12px;padding:0 8px;border-radius:999px;background:var(--surface-3);color:var(--text-2)}" +
        ".owo-notes-kind.k-heading{background:var(--accent-soft);color:var(--accent)}" +
        ".owo-notes-kind.k-code{background:var(--yellow-soft);color:var(--yellow)}" +
        ".owo-notes-kind.k-table{background:var(--green-soft);color:var(--green)}" +
        ".owo-notes-block-id{flex:none;font-size:12px;color:var(--text-3);font-family:monospace}" +
        ".owo-notes-block-preview{color:var(--text-2);word-break:break-all}" +
        ".owo-notes-block-missing{color:var(--red);font-size:12px}" +
        ".owo-notes-hint{color:var(--text-3);font-size:12px}";
      root.appendChild(style);
      this.styleElement = style;
      this.refresh();
      this.bind();
    },

    setSaveBusy: function (root, busy, fallbackButton) {
      var button = root && root.querySelector ? root.querySelector(".owo-notes-btn-save") : null;
      button = button || fallbackButton;
      if (!button) return;
      button.disabled = !!busy;
      if (busy) {
        if (!button.dataset.idleText) button.dataset.idleText = button.textContent || "保存";
        button.setAttribute("aria-busy", "true");
        button.textContent = "保存中…";
      } else {
        button.removeAttribute("aria-busy");
        button.textContent = button.dataset.idleText || "保存";
        delete button.dataset.idleText;
      }
    },

    dispose: function () {
      this.lifecycleGeneration = (this.lifecycleGeneration || 0) + 1;
      this.listGeneration = (this.listGeneration || 0) + 1;
      this.detailGeneration = (this.detailGeneration || 0) + 1;
      if (this.styleElement && this.styleElement.parentNode) {
        this.styleElement.parentNode.removeChild(this.styleElement);
      }
      this.styleElement = null;
      this.root = null;
    },

    isCurrent: function (generation) {
      return generation === this.lifecycleGeneration && !!this.root;
    },

    refresh: function () {
      var self = this;
      var generation = this.lifecycleGeneration;
      var request = ++this.listGeneration;
      var initialSection = this.rootEl();
      var initialList = initialSection && initialSection.querySelector(".owo-notes-list");
      if (initialList) initialList.innerHTML = '<li class="owo-notes-state" role="status">正在加载笔记…</li>';
      return Promise.resolve()
        .then(function () { return self.get("/notes"); })
        .then(function (data) {
          // 页面已卸载或更新请求已先返回时，忽略旧结果。
          if (!self.isCurrent(generation) || request !== self.listGeneration) return;
          var section = self.rootEl();
          if (!section) return;
          var ul = section.querySelector(".owo-notes-list");
          if (!ul) return;
          var list = data.notes || [];
          if (!list.length) {
            ul.innerHTML = '<li class="owo-notes-hint">（暂无笔记，点"新建"创建）</li>';
            return;
          }
          ul.innerHTML = list
            .map(function (n) {
              var title = n.title || n.id;
              return (
                '<li><button type="button" class="owo-notes-open" data-notes-open data-id="' + self.esc(n.id) +
                '" aria-label="打开笔记：' + self.esc(title) + '">' +
                '<span class="owo-notes-name">' + self.esc(title) + "</span>" +
                '<span class="owo-notes-hint">' + self.esc((n.updated_at || "").slice(0, 19)) + "</span>" +
                "</button></li>"
              );
            })
            .join("");
        })
        .catch(function (e) {
          if (!self.isCurrent(generation) || request !== self.listGeneration) return;
          var section = self.rootEl();
          var ul = section && section.querySelector(".owo-notes-list");
          if (!ul) return;
          ul.innerHTML = '<li class="owo-notes-state" role="status"><span>' +
            self.esc("列表加载失败：" + self.friendlyError(e)) +
            '</span><button type="button" class="owo-notes-retry" data-notes-retry>重试</button></li>';
        });
    },

    bind: function () {
      var self = this;
      var generation = this.lifecycleGeneration;
      var root = this.rootEl();
      var on = function (sel, ev, fn) {
        var el = root.querySelector(sel);
        if (el) el.addEventListener(ev, fn);
      };
      on(".owo-notes-btn-new", "click", function () {
        self.editorGeneration++;
        self.detailGeneration++;
        var editor = root.querySelector(".owo-notes-editor");
        var detail = root.querySelector(".owo-notes-detail");
        editor.dataset.returnNoteId = detail.hidden ? "" : (detail.dataset.id || "");
        editor.hidden = false;
        editor.querySelector(".owo-notes-title").value = "";
        editor.querySelector(".owo-notes-md").value = "";
        detail.hidden = true;
        detail.dataset.id = "";
      });
      on(".owo-notes-btn-save", "click", async function () {
        var editor = root.querySelector(".owo-notes-editor");
        var saveButton = root.querySelector(".owo-notes-btn-save");
        if (saveButton.disabled || self.saveInFlight) return;
        var title = root.querySelector(".owo-notes-title").value.trim();
        var md = root.querySelector(".owo-notes-md").value;
        if (!title) { self.alert("标题不能为空"); return; }
        var editorGeneration = self.editorGeneration;
        var detailGeneration = self.detailGeneration;
        self.saveInFlight = true;
        self.setSaveBusy(root, true, saveButton);
        try {
          var created = await self.post("/notes", { title: title, markdown: md });
          if (!self.isCurrent(generation)) return;
          if (editorGeneration === self.editorGeneration) {
            editor.hidden = true;
            editor.dataset.returnNoteId = "";
          }
          if (detailGeneration === self.detailGeneration) {
            root.querySelector(".owo-notes-detail").hidden = true;
          }
          self.refresh();
          if (created && created.id && detailGeneration === self.detailGeneration) {
            Promise.resolve()
              .then(function () { return self.get("/notes/" + encodeURIComponent(created.id)); })
              .then(function (doc) {
                if (self.isCurrent(generation) && detailGeneration === self.detailGeneration) self.renderDetail(doc);
              })
              .catch(function (e) {
                if (self.isCurrent(generation)) self.alert("笔记已创建，但读取详情失败：" + self.friendlyError(e));
              });
          }
        } catch (e) {
          if (self.isCurrent(generation)) self.alert("创建失败：" + self.friendlyError(e));
        } finally {
          self.saveInFlight = false;
          // 释放当前挂载页的按钮，即使请求发起后用户切走并重新进入了笔记页。
          self.setSaveBusy(self.root, false, saveButton);
        }
      });
      on(".owo-notes-btn-cancel", "click", function () {
        self.editorGeneration++;
        self.detailGeneration++;
        var editor = root.querySelector(".owo-notes-editor");
        var detail = root.querySelector(".owo-notes-detail");
        var returnId = editor.dataset.returnNoteId || "";
        editor.hidden = true;
        editor.dataset.returnNoteId = "";
        detail.dataset.id = returnId;
        detail.hidden = !returnId;
      });
      var runSearch = function () {
        var q = root.querySelector(".owo-notes-search").value.trim();
        if (!q) { self.refresh(); return; }
        var searchGeneration = self.lifecycleGeneration;
        var request = ++self.listGeneration;
        var list = root.querySelector(".owo-notes-list");
        if (list) list.innerHTML = '<li class="owo-notes-state" role="status">正在搜索…</li>';
        Promise.resolve()
          .then(function () { return self.get("/notes/search?q=" + encodeURIComponent(q)); })
          .then(function (data) {
            if (!self.isCurrent(searchGeneration) || request !== self.listGeneration) return;
            var section = self.rootEl();
            if (!section) return;
            var ul = section.querySelector(".owo-notes-list");
            if (!ul) return;
            var hits = data.hits || [];
            if (!hits.length) {
              ul.innerHTML = '<li class="owo-notes-hint">没有找到匹配笔记</li>';
              return;
            }
            ul.innerHTML = hits
              .map(function (h) {
                var snippet = h.snippet || h.doc_id;
                return '<li><button type="button" class="owo-notes-open" data-notes-open data-id="' +
                  self.esc(h.doc_id) + '" aria-label="打开搜索结果：' + self.esc(snippet) + '">' +
                  '<span class="owo-notes-name">' + self.esc(snippet) + "</span>" +
                  '<span class="owo-notes-hint">' + self.esc(h.doc_id.slice(0, 8)) + "</span>" +
                  "</button></li>";
              })
              .join("");
          })
          .catch(function (err) {
            if (!self.isCurrent(searchGeneration) || request !== self.listGeneration) return;
            var section = self.rootEl();
            var ul = section && section.querySelector(".owo-notes-list");
            if (ul) ul.innerHTML = '<li class="owo-notes-state" role="alert"><span>' +
              self.esc("搜索失败：" + self.friendlyError(err)) +
              '</span><button type="button" class="owo-notes-retry" data-notes-search-retry>重试搜索</button></li>';
          });
      };
      on(".owo-notes-search", "keydown", function (e) {
        if (e.key === "Escape") {
          var search = root.querySelector(".owo-notes-search");
          if (search && search.value) {
            search.value = "";
            self.refresh();
          }
          return;
        }
        if (e.key === "Enter") runSearch();
      });
      on(".owo-notes-list", "click", function (e) {
        if (e.target.closest("[data-notes-retry]")) { self.refresh(); return; }
        if (e.target.closest("[data-notes-search-retry]")) { runSearch(); return; }
        var openButton = e.target.closest("[data-notes-open]");
        if (!openButton) return;
        var id = openButton.dataset.id;
        var detailGeneration = self.lifecycleGeneration;
        var request = ++self.detailGeneration;
        Promise.resolve()
          .then(function () { return self.get("/notes/" + encodeURIComponent(id)); })
          .then(function (doc) {
            if (!self.isCurrent(detailGeneration) || request !== self.detailGeneration) return;
            self.renderDetail(doc);
          })
          .catch(function (err) {
            if (!self.isCurrent(detailGeneration) || request !== self.detailGeneration) return;
            self.alert("读取失败：" + self.friendlyError(err));
          });
      });
      on(".owo-notes-btn-export-md", "click", function () {
        var id = root.querySelector(".owo-notes-detail").dataset.id;
        Promise.resolve()
          .then(function () { return self.get("/notes/" + encodeURIComponent(id) + "/export/md"); })
          .then(function (r) { if (self.isCurrent(generation)) self.download(id + ".md", r.content); })
          .catch(function (e) { if (self.isCurrent(generation)) self.alert("导出失败：" + self.friendlyError(e)); });
      });
      on(".owo-notes-btn-export-html", "click", function () {
        var id = root.querySelector(".owo-notes-detail").dataset.id;
        Promise.resolve()
          .then(function () { return self.get("/notes/" + encodeURIComponent(id) + "/export/html"); })
          .then(function (r) { if (self.isCurrent(generation)) self.download(id + ".html", r.content); })
          .catch(function (e) { if (self.isCurrent(generation)) self.alert("导出失败：" + self.friendlyError(e)); });
      });
      on(".owo-notes-btn-del", "click", function () {
        var detail = root.querySelector(".owo-notes-detail");
        var id = detail.dataset.id;
        if (!id) return;
        var request = self.detailGeneration;
        var owner = self.lifecycleGeneration;
        var titleEl = detail.querySelector(".owo-notes-detail-title");
        var title = titleEl && titleEl.textContent.trim();
        self.confirm({
          title: "删除笔记",
          message: title ? "确认删除《" + title + "》？" : "确认删除这篇笔记？",
          confirmText: "删除",
          kind: "danger",
        }).then(function (ok) {
          // The confirmation may outlive the selected note or the panel itself.
          if (!ok || !self.isCurrent(owner) || request !== self.detailGeneration || detail.dataset.id !== id) return;
          self
            .del("/notes/" + encodeURIComponent(id))
            .then(function () {
              if (!self.isCurrent(owner)) return;
              // Deletion applies to the captured note; never hide a different note selected meanwhile.
              if (request === self.detailGeneration && detail.dataset.id === id) detail.hidden = true;
              self.refresh();
            })
            .catch(function (err) {
              if (self.isCurrent(owner)) self.alert("删除失败：" + self.friendlyError(err));
            });
        });
      });
      var renameNote = function () {
        var detail = root.querySelector(".owo-notes-detail");
        var id = detail.dataset.id;
        if (!id) return;
        var request = self.detailGeneration;
        var current = detail.querySelector(".owo-notes-detail-title").textContent;
        self
          .prompt({ title: "修改标题", label: "新标题：", value: current, confirmText: "保存" })
          .then(function (title) {
            if (!title || !self.isCurrent(generation) || request !== self.detailGeneration || detail.dataset.id !== id) return;
            return self.put("/notes/" + encodeURIComponent(id), { title: title });
          })
          .then(function (result) {
            if (!result || !self.isCurrent(generation) || request !== self.detailGeneration || detail.dataset.id !== id) return;
            self.refresh();
            return self.get("/notes/" + encodeURIComponent(id)).then(function (doc) {
              if (self.isCurrent(generation) && request === self.detailGeneration && detail.dataset.id === id) self.renderDetail(doc);
            });
          })
          .catch(function (err) {
            if (self.isCurrent(generation) && request === self.detailGeneration) self.alert("改标题失败：" + self.friendlyError(err));
          });
      };
      on(".owo-notes-btn-rename", "click", renameNote);
      on(".owo-notes-detail-title", "dblclick", renameNote);
    },

    renderDetail: function (doc) {
      var self = this;
      var section = this.rootEl();
      if (!section) return;
      var detail = section.querySelector(".owo-notes-detail");
      if (!detail) return;
      detail.hidden = false;
      detail.dataset.id = doc.id;
      detail.querySelector(".owo-notes-detail-title").textContent = doc.title || doc.id;
      var blocks = doc.blocks || {};
      var meta = detail.querySelector("#owo-notes-detail-count");
      if (meta) {
        meta.textContent = Object.keys(blocks).length + " 个块 ｜ 更新于 " + (doc.updated_at || "—");
      }
      var tree = detail.querySelector(".owo-notes-tree");
      if (!tree) return;
      if (!blocks[doc.root]) {
        tree.innerHTML = '<div class="owo-notes-hint">（根块缺失，无法渲染块树）</div>';
        return;
      }
      tree.innerHTML = '<ul class="owo-notes-block-tree">' + self.blockList(blocks, [doc.root]) + "</ul>";
    },

    // 递归渲染块树：kind 徽章 + 块 id + 内容预览 + 子块缩进列表
    blockList: function (blocks, ids) {
      var self = this;
      var html = "";
      for (var i = 0; i < ids.length; i++) {
        var b = blocks[ids[i]];
        if (!b) {
          html += '<li class="owo-notes-block"><span class="owo-notes-block-missing">缺失块 ' +
            self.esc(ids[i]) + "</span></li>";
          continue;
        }
        html +=
          '<li class="owo-notes-block">' +
          '<div class="owo-notes-block-row">' +
          '<span class="owo-notes-kind ' + self.kindClass(b.kind) + '">' + self.esc(self.kindLabel(b.kind)) + "</span>" +
          '<span class="owo-notes-block-id">' + self.esc(b.id || ids[i]) + "</span>" +
          '<span class="owo-notes-block-preview">' + self.esc(self.kindPreview(b.kind)) + "</span>" +
          "</div>" +
          (b.children && b.children.length
            ? "<ul>" + self.blockList(blocks, b.children) + "</ul>"
            : "") +
          "</li>";
      }
      return html;
    },

    // BlockKind（外标签 serde enum，如 {"Heading":{"level":1,"text":"…"}}）→ 中文标签
    kindLabel: function (kind) {
      var k = kind && typeof kind === "object" ? Object.keys(kind)[0] : "";
      if (k === "Paragraph") return "段落";
      if (k === "Heading") return "标题";
      if (k === "List") return "列表";
      if (k === "ListItem") return "列表项";
      if (k === "Code") return "代码";
      if (k === "Table") return "表格";
      if (k === "Image") return "图片";
      if (k === "File") return "文件";
      if (k === "Quote") return "引用";
      if (k === "HtmlEmbed") return "嵌入";
      if (k === "Canvas") return "画布";
      if (k === "AiGenerated") return "AI 生成";
      return k || "未知";
    },

    kindClass: function (kind) {
      var k = kind && typeof kind === "object" ? Object.keys(kind)[0] : "";
      if (k === "Heading") return "k-heading";
      if (k === "Code") return "k-code";
      if (k === "Table") return "k-table";
      return "";
    },

    // BlockKind → 单行内容预览
    kindPreview: function (kind) {
      var k = kind && typeof kind === "object" ? Object.keys(kind)[0] : "";
      var v = kind && typeof kind === "object" ? kind[k] : null;
      if (!v || typeof v !== "object") return "";
      if (k === "Heading") return "H" + (v.level || 1) + " " + (v.text || "");
      if (k === "Code") return (v.language ? "[" + v.language + "] " : "") + (v.text || "");
      if (k === "Table") return (v.rows || []).length + " 行";
      if (k === "Image") return (v.alt || "") + (v.src ? "（" + v.src + "）" : "");
      if (k === "File") return (v.path || "") + (v.mime ? "（" + v.mime + "）" : "");
      if (k === "List") return v.ordered ? "有序列表" : "无序列表";
      if (k === "HtmlEmbed") return "已消毒 HTML 片段";
      if (k === "Canvas") {
        var d = v.data || {};
        return (d.rects || []).length + " 矩形 / " + (d.notes || []).length + " 便签";
      }
      if (k === "AiGenerated") return (v.model || "") + "：" + (v.prompt || "");
      return v.text || "";
    },

    download: function (name, content) {
      var blob = new Blob([content], { type: "text/plain;charset=utf-8" });
      var a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = name;
      a.click();
      URL.revokeObjectURL(a.href);
    },

    alert: function (msg) {
      if (this.helpers && this.helpers.notify) { this.helpers.notify(msg, "error"); return; }
      if (window.OwoToast) { window.OwoToast(msg); return; }
      window.alert(msg);
    },

    rootEl: function () {
      if (!this.root) return null;
      // 必须限定在 #panelRoot 内查找：全局 querySelector('[data-panel="notes"]')
      // 会先匹配到 #panelNav 里的 tab 按钮（DOM 顺序在前），导致拿到错误元素。
      var root = document.getElementById("panelRoot");
      return root ? root.querySelector('[data-panel="notes"]') : null;
    },
  };
})();
