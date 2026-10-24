/* Local note library, Markdown creation, semantic document reading, block inspection and export.
 * Uses the shared API transport; all workbench styles live in workbench-pages.css. */
(function () {
  window.OwoPanels = window.OwoPanels || {};
  function checkedResponse(response) {
    if (!response.ok) throw new Error("HTTP " + response.status);
    return response.json();
  }
  window.OwoPanels.notes = {
    id: "notes",
    title: "笔记",

    nav: function () {
      return (
        '<section data-panel="notes" data-layout="custom" class="owo-notes-root">' +
        '<div class="owo-notes-bar">' +
        '<input class="owo-notes-search" type="search" aria-label="搜索笔记全文" aria-describedby="owo-notes-core-hint" data-core-action placeholder="搜索笔记全文">' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-search" data-core-action>搜索</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-refresh" data-core-action>刷新</button>' +
        '<button type="button" class="owo-notes-btn primary owo-notes-btn-new" data-core-action aria-label="新建笔记">新建笔记</button></div>' +
        '<p class="owo-notes-hint" id="owo-notes-core-hint" data-core-action-hint>连接并授权本地核心后可搜索笔记全文和管理笔记。</p>' +
        '<div class="owo-notes-workspace"><aside class="owo-notes-library" aria-label="本地笔记">' +
        '<h3>本地笔记</h3><ul class="owo-notes-list" aria-label="笔记列表"></ul></aside>' +
        '<div class="owo-notes-content"><div class="owo-notes-editor" hidden>' +
        '<h3>新建笔记</h3><label for="owo-notes-title">笔记标题</label>' +
        '<input id="owo-notes-title" class="owo-notes-title" type="text" aria-label="笔记标题" placeholder="标题" required>' +
        '<label for="owo-notes-md">Markdown 正文</label><textarea id="owo-notes-md" class="owo-notes-md" aria-label="Markdown 正文" rows="12" placeholder="Markdown 正文"></textarea>' +
        '<div class="owo-notes-editor-actions"><button type="button" class="owo-notes-btn owo-notes-btn-cancel">取消</button>' +
        '<button type="button" class="owo-notes-btn primary owo-notes-btn-save" data-core-action>保存</button></div></div>' +
        '<p class="owo-notes-welcome">选择一篇笔记，或新建笔记。</p>' +
        '<div class="owo-notes-detail" hidden><div class="owo-notes-detail-head"><h3 class="owo-notes-detail-title"></h3>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-rename" data-core-action>修改标题</button></div>' +
        '<div class="owo-notes-detail-meta"><span class="owo-notes-meta-item" id="owo-notes-detail-count"></span></div>' +
        '<div class="owo-notes-detail-actions"><button type="button" class="owo-notes-btn owo-notes-btn-export-md">导出 MD</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-export-html">导出 HTML</button>' +
        '<button type="button" class="owo-notes-btn owo-notes-btn-del" data-core-action>删除</button></div>' +
        '<article class="owo-notes-reading" aria-label="笔记正文"></article>' +
        '<details class="owo-notes-structure"><summary>块结构</summary><div class="owo-notes-tree"></div></details></div>' +
        '</div></div></section>'
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
      this.helpers = helpers || {};
      this.baseUrl = this.helpers.baseUrl || window.OwoPanels.baseUrl || window.location.origin;
      var scope = String(this.baseUrl) + "#" + String(this.helpers.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
      if (this.hostScope && this.hostScope !== scope) {
        this.draft = null;
        this.selectedDoc = null;
        this.saveInFlight = false;
        this.saveOwner = null;
      }
      this.hostScope = scope;
      this.setSaveBusy(root, !!this.saveInFlight);
      this.get = this.helpers.get || function (path) {
        return window.OwoApi.stream(path).then(function (r) { return checkedResponse(r); });
      };
      this.post = this.helpers.post || function (path, body) {
        return window.OwoApi.stream(path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body || {}),
        }).then(function (r) { return checkedResponse(r); });
      };
      this.put = this.helpers.put || function (path, body) {
        return window.OwoApi.stream(path, {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body || {}),
        }).then(function (r) { return checkedResponse(r); });
      };
      this.del = this.helpers.del || function (path) {
        return window.OwoApi.stream(path, { method: "DELETE" }).then(function (r) { return checkedResponse(r); });
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
      this.restoreDraft();
      this.refresh();
      this.bind();
    },

    setSaveBusy: function (root, busy, fallbackButton) {
      var button = root && root.querySelector ? root.querySelector(".owo-notes-btn-save") : null;
      button = button || fallbackButton;
      if (!button) return;
      if (button.setAttribute) button.setAttribute("data-core-busy", busy ? "true" : "false");
      if (window.OwoCoreActionAvailability && window.OwoCoreActionAvailability.setBusy) window.OwoCoreActionAvailability.setBusy(button,!!busy);
      else button.disabled = !!busy;
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
      this.captureDraft();
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
          if (!data || !Array.isArray(data.notes)) throw new Error("笔记列表响应格式不完整");
          self.notesById = Object.create(null);
          data.notes.forEach(function(note) { if (note && note.id) self.notesById[note.id]=note; });
          var list = data.notes.slice().sort(function (a, b) { return String(b.updated_at || "").localeCompare(String(a.updated_at || "")); });
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
                '<span class="owo-notes-hint">' + self.esc(self.localTime(n.updated_at)) + "</span>" +
                "</button></li>"
              );
            })
            .join("");
          self.markSelection(self.selectedDoc && self.selectedDoc.id);
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
        if (el) el.addEventListener(ev, function (event) { if (self.isCurrent(generation)) return fn(event); });
      };
      on(".owo-notes-btn-new", "click", function () {
        var activeEditor = root.querySelector(".owo-notes-editor");
        if (!activeEditor.hidden && (root.querySelector(".owo-notes-title").value || root.querySelector(".owo-notes-md").value)) {
          var activeTitle = root.querySelector(".owo-notes-title");
          if (activeTitle.focus) activeTitle.focus({ preventScroll: true });
          return;
        }
        self.draft = null;
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
        self.syncWelcome();
        var titleInput = editor.querySelector(".owo-notes-title");
        if (titleInput && titleInput.focus) titleInput.focus({ preventScroll: true });
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
        var transport = self.post;
        var saveOwner = {};
        self.saveOwner = saveOwner;
        self.saveInFlight = true;
        self.setSaveBusy(root, true, saveButton);
        try {
          var created = await transport("/notes", { title: title, markdown: md });
          if (!self.isCurrent(generation)) return;
          if (editorGeneration === self.editorGeneration && root.querySelector(".owo-notes-title").value.trim() === title && root.querySelector(".owo-notes-md").value === md) {
            self.draft = null;
            editor.hidden = true;
            editor.dataset.returnNoteId = "";
          }
          if (detailGeneration === self.detailGeneration && editor.hidden) {
            root.querySelector(".owo-notes-detail").hidden = true;
          }
          self.refresh();
          if (created && created.id && editor.hidden && detailGeneration === self.detailGeneration) {
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
          if (self.saveOwner === saveOwner) {
            self.saveOwner = null;
            self.saveInFlight = false;
            self.setSaveBusy(self.root, false, saveButton);
          }
          if (self.isCurrent(generation)) self.syncWelcome();
        }
      });
      on(".owo-notes-btn-cancel", "click", function () {
        self.editorGeneration++;
        self.detailGeneration++;
        var editor = root.querySelector(".owo-notes-editor");
        var detail = root.querySelector(".owo-notes-detail");
        var returnId = editor.dataset.returnNoteId || "";
        self.draft = null;
        editor.hidden = true;
        editor.dataset.returnNoteId = "";
        detail.dataset.id = returnId;
        detail.hidden = !returnId;
        self.syncWelcome();
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
            if (!data || !Array.isArray(data.hits)) throw new Error("搜索响应格式不完整");
            var hits = data.hits;
            if (!hits.length) {
              ul.innerHTML = '<li class="owo-notes-hint">没有找到匹配笔记</li>';
              return;
            }
            var grouped = Object.create(null);
            hits.forEach(function(hit) {
              if (!hit || typeof hit.doc_id !== "string" || !hit.doc_id) throw new Error("搜索结果缺少笔记编号");
              if (!grouped[hit.doc_id]) grouped[hit.doc_id]=[];
              if (grouped[hit.doc_id].indexOf(hit.snippet || "") === -1) grouped[hit.doc_id].push(hit.snippet || "");
            });
            ul.innerHTML = Object.keys(grouped).map(function(id) {
              var note=self.notesById && self.notesById[id], title=note && note.title || id;
              return '<li><button type="button" class="owo-notes-open" data-notes-open data-id="'+self.esc(id)+'" aria-label="打开搜索结果：'+self.esc(title)+'">' +
                '<span class="owo-notes-name">'+self.esc(title)+'</span>'+
                grouped[id].map(function(snippet){return '<span class="owo-notes-hint">'+self.esc(snippet)+'</span>';}).join("")+'</button></li>';
            }).join("");
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
      on(".owo-notes-btn-search", "click", runSearch);
      on(".owo-notes-btn-refresh", "click", function () {
        var search = root.querySelector(".owo-notes-search");
        if (search && search.value.trim()) return runSearch();
        return self.refresh();
      });
      on(".owo-notes-search", "search", function () { if (!root.querySelector(".owo-notes-search").value) self.refresh(); });
      on(".owo-notes-title", "input", function () { self.captureDraft(); });
      on(".owo-notes-md", "input", function () { self.captureDraft(); });
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
          .then(function (r) { if (self.isCurrent(generation)) self.download(id + ".md", r.content, "text/markdown"); })
          .catch(function (e) { if (self.isCurrent(generation)) self.alert("导出失败：" + self.friendlyError(e)); });
      });
      on(".owo-notes-btn-export-html", "click", function () {
        var id = root.querySelector(".owo-notes-detail").dataset.id;
        Promise.resolve()
          .then(function () { return self.get("/notes/" + encodeURIComponent(id) + "/export/html"); })
          .then(function (r) { if (self.isCurrent(generation)) self.download(id + ".html", r.content, "text/html"); })
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
              if (request === self.detailGeneration && detail.dataset.id === id) {
                detail.hidden = true;
                detail.dataset.id = "";
                self.selectedDoc = null;
                self.syncWelcome();
              }
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
            title = typeof title === "string" ? title.trim() : "";
            if (!title || title === current || !self.isCurrent(generation) || request !== self.detailGeneration || detail.dataset.id !== id) return;
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
      if (!doc || !doc.id || !doc.blocks || typeof doc.root !== "string") {
        this.alert("笔记详情响应格式不完整");
        return;
      }
      this.selectedDoc = doc;
      detail.hidden = false;
      detail.dataset.id = doc.id;
      detail.querySelector(".owo-notes-detail-title").textContent = doc.title || doc.id;
      var blocks = doc.blocks || {};
      var meta = detail.querySelector("#owo-notes-detail-count");
      if (meta) {
        meta.textContent = Object.keys(blocks).length + " 个块 ｜ 更新于 " + (self.localTime(doc.updated_at) + "（本地时间）");
      }
      var article = detail.querySelector(".owo-notes-reading");
      if (article) article.innerHTML = self.renderDocument(doc);
      var editor = section.querySelector(".owo-notes-editor");
      if (editor && !editor.hidden) self.captureDraft();
      if (editor) editor.hidden = true;
      this.syncWelcome();
      this.markSelection(doc.id);
      var tree = detail.querySelector(".owo-notes-tree");
      if (!tree) return;
      if (!blocks[doc.root]) {
        tree.innerHTML = '<div class="owo-notes-hint">（根块缺失，无法渲染块树）</div>';
        return;
      }
      tree.innerHTML = '<ul class="owo-notes-block-tree">' + self.blockList(blocks, [doc.root]) + "</ul>";
    },

    // 递归渲染块树：kind 徽章 + 块 id + 内容预览 + 子块缩进列表
    blockList: function (blocks, ids, ancestors) {
      ancestors = ancestors || [];
      var self = this;
      var html = "";
      for (var i = 0; i < ids.length; i++) {
        var b = blocks[ids[i]];
        if (ancestors.indexOf(ids[i]) !== -1 || ancestors.length >= 80) {
          html += '<li class="owo-notes-block-missing">块引用循环或层级过深</li>';
          continue;
        }
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
            ? "<ul>" + self.blockList(blocks, b.children, ancestors.concat(ids[i])) + "</ul>"
            : "") +
          "</li>";
      }
      return html;
    },

    captureDraft: function () {
      var section = this.rootEl();
      var editor = section && section.querySelector(".owo-notes-editor");
      if (!editor || editor.hidden) return;
      var title = editor.querySelector(".owo-notes-title");
      var body = editor.querySelector(".owo-notes-md");
      this.draft = { title: title.value, markdown: body.value, returnNoteId: editor.dataset.returnNoteId || "" };
    },

    restoreDraft: function () {
      var section = this.rootEl();
      var editor = section && section.querySelector(".owo-notes-editor");
      if (editor && this.draft) {
        editor.hidden = false;
        editor.querySelector(".owo-notes-title").value = this.draft.title;
        editor.querySelector(".owo-notes-md").value = this.draft.markdown;
        editor.dataset.returnNoteId = "";
      }
      this.syncWelcome();
    },

    syncWelcome: function () {
      var section = this.rootEl();
      if (!section) return;
      var welcome = section.querySelector(".owo-notes-welcome");
      var editor = section.querySelector(".owo-notes-editor");
      var detail = section.querySelector(".owo-notes-detail");
      if (welcome) welcome.hidden = !!((editor && !editor.hidden) || (detail && !detail.hidden));
    },

    markSelection: function (id) {
      var section = this.rootEl();
      if (!section || !section.querySelectorAll) return;
      Array.prototype.forEach.call(section.querySelectorAll("[data-notes-open]"), function (button) {
        button.setAttribute("aria-current", button.dataset.id === id ? "true" : "false");
      });
    },

    localTime: function (value) {
      if (!value) return "时间未知";
      var date = new Date(value);
      return Number.isNaN(date.getTime()) ? "时间未知" : date.toLocaleString("zh-CN", { hour12: false });
    },

    renderDocument: function (doc) {
      var self = this, blocks = doc.blocks || {}, remaining = 5000;
      function inline(value) {
        var text=value == null ? "" : String(value);
        if (window.OwoMarkdown && window.OwoMarkdown.inlineMarkdown) return window.OwoMarkdown.inlineMarkdown(text).replace(/\n/g, "<br>");
        return self.esc(text).replace(/\n/g, "<br>");
      }
      function render(id, ancestors) {
        if (--remaining < 0) return '<p class="owo-notes-hint">文档过长，余下内容可导出查看。</p>';
        if (ancestors.indexOf(id) !== -1 || ancestors.length >= 80) return '<p class="owo-notes-hint">块引用循环或层级过深。</p>';
        var block = blocks[id];
        if (!block) return '<p class="owo-notes-hint">缺失块：' + self.esc(id) + '</p>';
        var kind = block.kind || {}, key = Object.keys(kind)[0], value = kind[key] || {};
        var children = Array.isArray(block.children) ? block.children : [];
        var nested = children.map(function (child) { return render(child, ancestors.concat(id)); }).join("");
        if (id === doc.root) return (value.text ? "<p>" + inline(value.text) + "</p>" : "") + nested;
        if (key === "Heading") {
          var level = Math.max(1, Math.min(6, Number(value.level) || 2));
          return "<h" + level + ">" + inline(value.text) + "</h" + level + ">" + nested;
        }
        if (key === "Paragraph" || key === "AiGenerated") return "<p>" + inline(value.text) + "</p>" + nested;
        if (key === "Quote") return "<blockquote>" + inline(value.text) + nested + "</blockquote>";
        if (key === "Code") return "<pre><code>" + self.esc(value.text || "") + "</code></pre>" + nested;
        if (key === "List") { var tag = value.ordered ? "ol" : "ul"; return "<" + tag + ">" + nested + "</" + tag + ">"; }
        if (key === "ListItem") return "<li>" + inline(value.text) + nested + "</li>";
        if (key === "Table") return '<div class="owo-notes-table"><table><tbody>' + (Array.isArray(value.rows) ? value.rows : []).map(function (row) {
          return "<tr>" + (Array.isArray(row) ? row : []).map(function (cell) { return "<td>" + inline(cell) + "</td>"; }).join("") + "</tr>";
        }).join("") + "</tbody></table></div>" + nested;
        // Arbitrary embedded HTML, local files and remote media do not execute on read.
        if (key === "HtmlEmbed") return '<pre class="owo-notes-embedded">' + self.esc(value.html || "") + "</pre>" + nested;
        if (key === "Image") return '<p class="owo-notes-hint">图片：' + inline(value.alt || value.src || "") + "</p>" + nested;
        if (key === "File") return '<p class="owo-notes-hint">文件：' + inline(value.path || "") + "</p>" + nested;
        if (key === "Canvas") return '<p class="owo-notes-hint">画布：' + inline(self.kindPreview(kind)) + "</p>" + nested;
        return '<p class="owo-notes-hint">' + inline(self.kindLabel(kind)) + "</p>" + nested;
      }
      return render(doc.root, []);
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

    download: function (name, content, mime) {
      if (typeof content !== "string") throw new Error("导出响应没有正文");
      var blob = new Blob([content], { type: (mime || "text/plain") + ";charset=utf-8" });
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
      return this.root.querySelector('[data-panel="notes"]') || this.root;
    },
  };
})();
