/* Role graph editor. Every user operation updates the same portable TeamTemplate document. */
(function (global) {
  "use strict";
  function escape(text) { return String(text == null ? "" : text).replace(/[&<>"']/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]; }); }
  function project(value, helper) {
    var result = global.OwoWorkflowDocument.graphTemplate(value);
    if (!result.ok) return result;
    var saved = Object.create(null);
    result.document.editor.nodes.forEach(function (node) { if (node.position) saved[node.id] = node.position; });
    var layout = helper.graphPositions(result.template, saved);
    if (!layout.ok) {
      layout = helper.graphPositions({ roles: result.template.roles.map(function (role) { return { role: role.role, assignee: role.assignee }; }) }, saved);
      var ids = new Set(result.template.roles.map(function (role) { return role.role; }));
      layout.edges = [];
      result.template.roles.forEach(function (role) { role.depends_on.forEach(function (from) { if (ids.has(from)) layout.edges.push({ from: from, to: role.role }); }); });
    }
    result.layout = layout;
    return result;
  }
  function renderGraphSvg(value, helper, selected, selectedEdge) {
    var result = project(value, helper), esc = escape;
    if (!result.ok) return '<p role="alert">' + esc(result.error) + '</p>';
    var layout = result.layout, labels = new Map(result.document.roles.map(function (role) { return [role.node_id, role.role]; }));
    var points = new Map(layout.nodes.map(function (node) { return [node.id, node]; }));
    var out = ['<svg class="owo-wfc-graph owo-wf-graph" viewBox="0 0 ' + layout.width + ' ' + layout.height + '" width="' + layout.width + '" height="' + layout.height + '" role="group" aria-label="工作流依赖图">'];
    layout.edges.forEach(function (edge) {
      var from = points.get(edge.from), to = points.get(edge.to);
      if (!from || !to) return;
      var active = selectedEdge && selectedEdge.from === edge.from && selectedEdge.to === edge.to;
      var attrs = ' data-edge-from="' + esc(edge.from) + '" data-edge-to="' + esc(edge.to) + '" d="' + esc(helper.edgePath(from, to)) + '" fill="none"';
      out.push('<path class="owo-wfc-edge' + (active ? ' is-selected' : '') + '"' + attrs + '/>');
      out.push('<path class="owo-wfc-edge-hit"' + attrs + ' tabindex="0" role="button" aria-label="依赖 ' + esc(labels.get(edge.from)) + ' 到 ' + esc(labels.get(edge.to)) + '，选择后可删除"/>');
    });
    out.push('<path class="owo-wfc-wire" fill="none" pointer-events="none" hidden/>');
    layout.nodes.forEach(function (node) {
      var label = labels.get(node.id) || node.id;
      out.push('<g class="owo-wf-node owo-wfc-node' + (node.id === selected ? ' is-selected' : '') + '" data-node-id="' + esc(node.id) + '" transform="translate(' + node.x + ' ' + node.y + ')" tabindex="0" role="button" aria-label="编辑节点 ' + esc(label) + '">');
      out.push('<rect width="124" height="48" rx="10" class="owo-wf-node-box"/><text x="12" y="20" font-size="11">' + esc(label.length > 16 ? label.slice(0, 15) + '…' : label) + '</text><text x="12" y="35" font-size="9" opacity="0.68">' + esc(node.assignee) + '</text>');
      out.push('<circle class="owo-wfc-port" data-input-port="' + esc(node.id) + '" cx="0" cy="24" r="6" tabindex="0" role="button" aria-label="连接到 ' + esc(label) + '"/>');
      out.push('<circle class="owo-wfc-port" data-output-port="' + esc(node.id) + '" cx="124" cy="24" r="6" tabindex="0" role="button" aria-label="从 ' + esc(label) + ' 开始连接"/></g>');
    });
    out.push('</svg>');
    if (result.problems.length) out.push('<p class="owo-wfc-problems" role="alert">' + result.problems.map(esc).join('；') + '</p>');
    return out.join('');
  }
  function create(options) {
    var root = options.root, D = global.OwoWorkflowDocument, helper = options.helper;
    var doc = null, selected = null, edge = null, pending = null, drag = null, valid = false, disposed = false, catalog = [], ignoreClick = 0, errorText = "";
    var history = [], future = [];
    function notify(message) { if (options.onMessage) options.onMessage(message); }
    function role() { return doc && doc.roles.find(function (entry) { return entry.node_id === selected; }); }
    function field(label, name, value, multiline) {
      return '<label>' + label + (multiline ? '<textarea rows="2"' : '<input type="text"') + ' class="owo-wfc-field" name="' + name + '"' + (multiline ? '>' + escape(value) + '</textarea>' : ' value="' + escape(value) + '">') + '</label>';
    }
    function list(value) { return Array.isArray(value) ? value.join("\n") : String(value || ""); }
    function toolOptions() { return '<option value="">选择已注册工具</option>' + catalog.filter(function (tool) { return tool.worker_supported === true && tool.status === "registered"; }).map(function (tool) { return '<option value="' + escape(tool.name) + '">' + escape(tool.name) + '</option>'; }).join(""); }
    function form() {
      var current = role();
      if (!current) return '<p class="hint">选择节点查看参数，或添加一个角色节点。</p>';
      return '<form class="owo-wfc-form"><h4>节点配置</h4><code>' + escape(current.node_id) + '</code>' +
        field('角色名', 'role', current.role) +
        '<label>执行者<select name="assignee">' + ['agent', 'human', 'worker'].map(function (name) { return '<option' + (name === (current.assignee || 'agent') ? ' selected' : '') + '>' + name + '</option>'; }).join('') + '</select></label>' +
        field('Worker（可选）', 'worker', current.worker || '') + field('模型（可选）', 'model', current.model || '') +
        field('工具白名单（每行一个；空白使用角色默认工具）', 'tools', list(current.tools), true) +
        '<label>从当前工具目录添加<select class="owo-wfc-add-tool">' + toolOptions() + '</select></label>' +
        field('写范围（每行一个相对路径）', 'write_paths', list(current.write_paths), true) +
        field('职责能力（每行一个）', 'capabilities', list(current.capabilities), true) +
        field('交接契约', 'handoff_contract', current.handoff_contract || '', true) + field('验证断言', 'verify', current.verify || '') +
        '<p class="hint">模型、工具和写范围在运行时仍受宿主权限约束。</p><button type="submit">应用节点配置</button></form>';
    }
    function render() {
      if (disposed || !doc) return;
      var selectedRole = role();
      root.innerHTML = '<div class="owo-wfc-toolbar"><button type="button" data-wfc="add">添加角色</button>' +
        '<button type="button" data-wfc="remove"' + (!selectedRole ? ' disabled' : '') + '>删除节点</button>' +
        '<button type="button" data-wfc="disconnect"' + (!edge ? ' disabled' : '') + '>删除连接</button>' +
        '<button type="button" data-wfc="undo"' + (!history.length ? ' disabled' : '') + '>撤销</button>' +
        '<button type="button" data-wfc="redo"' + (!future.length ? ' disabled' : '') + '>重做</button>' +
        '<button type="button" data-wfc="reset">自动布局</button></div>' +
        '<p class="hint owo-wfc-status" role="status">' + (pending ? '选择目标节点的左侧端口；Esc 取消连接。' : '拖动右侧端口到左侧端口可连线，也可依次点击两个端口。端口表示任务完成依赖。') + '</p>' +
        (errorText ? '<p class="owo-wfc-problems" role="alert">' + escape(errorText) + '</p>' : '') +
        '<div class="owo-wfc-body"><div class="owo-wfc-stage">' + renderGraphSvg(doc, helper, selected, edge) + '</div><aside class="owo-wfc-inspector" aria-label="节点配置">' + form() + '</aside></div>';
    }
    function apply(result, message) {
      if (!valid || !result.ok) { errorText = result.error || 'JSON 无效，请先修正'; pending = null; if (valid) render(); notify(errorText); return false; }
      errorText = '';
      if (JSON.stringify(doc) !== JSON.stringify(result.document)) {
        history.push(JSON.stringify(doc)); if (history.length > 50) history.shift(); future = [];
      }
      doc = result.document; pending = null; edge = null;
      render(); if (options.onChange) options.onChange(JSON.parse(JSON.stringify(doc)), message);
      notify(message); return true;
    }
    function historyStep(back) {
      var from = back ? history : future, to = back ? future : history;
      if (!from.length) return;
      to.push(JSON.stringify(doc)); doc = JSON.parse(from.pop()); selected = role() ? selected : null; edge = null; pending = null;
      render(); if (options.onChange) options.onChange(JSON.parse(JSON.stringify(doc)), back ? '已撤销' : '已重做');
    }
    function hit(target, selector) { return target && target.closest ? target.closest(selector) : null; }
    function connect(from, to) { apply(D.connect(doc, from, to), '连接已写入 depends_on'); }
    function activate(target) {
      if (!valid || disposed) return;
      var action = hit(target, '[data-wfc]'), output = hit(target, '[data-output-port]'), input = hit(target, '[data-input-port]');
      if (action) {
        switch (action.getAttribute('data-wfc')) {
          case 'add': {
            var n = 1; while (doc.roles.some(function (r) { return r.role === '角色 ' + n; })) n += 1;
            var added = D.addNode(doc, { role: '角色 ' + n });
            if (added.ok) selected = added.document.roles[added.document.roles.length - 1].node_id;
            apply(added, '已添加角色，请配置工具与交接契约'); break;
          }
          case 'remove': if (selected) { var removed = D.removeNode(doc, selected); selected = null; apply(removed, '已删除节点及其相连依赖，未自动重接'); } break;
          case 'disconnect': if (edge) apply(D.disconnect(doc, edge.from, edge.to), '已删除依赖连接'); break;
          case 'undo': historyStep(true); break;
          case 'redo': historyStep(false); break;
          case 'reset': { var reset = D.normalize(doc); reset.document.editor.nodes.forEach(function (node) { delete node.position; }); apply(reset, '已恢复自动布局，可随配置保存'); break; }
        }
      } else if (output) { pending = output.getAttribute('data-output-port'); render(); }
      else if (input && pending) connect(pending, input.getAttribute('data-input-port'));
      else {
        var selectedPath = hit(target, '[data-edge-from]'), node = hit(target, '[data-node-id]');
        if (selectedPath) { edge = { from: selectedPath.getAttribute('data-edge-from'), to: selectedPath.getAttribute('data-edge-to') }; selected = null; pending = null; render(); }
        else if (node) { selected = node.getAttribute('data-node-id'); edge = null; render(); }
      }
    }
    root.onclick = function (event) { if (Date.now() < ignoreClick && hit(event.target, 'svg')) return; ignoreClick = 0; activate(event.target); };
    root.onkeydown = function (event) {
      if (event.key === 'Escape') { if (pending || drag) { pending = null; drag = null; render(); } return; }
      if (event.key === 'Enter' || event.key === ' ') {
        if (hit(event.target, '[data-output-port], [data-input-port], [data-node-id], [data-edge-from]')) { event.preventDefault(); activate(event.target); }
      } else if (event.key === 'Delete' && hit(event.target, '[data-edge-from]')) { event.preventDefault(); var path = hit(event.target, '[data-edge-from]'); apply(D.disconnect(doc, path.getAttribute('data-edge-from'), path.getAttribute('data-edge-to')), '已删除依赖连接'); }
    };
    root.onsubmit = function (event) {
      if (!hit(event.target, '.owo-wfc-form')) return;
      event.preventDefault(); if (!valid || !selected) return;
      var patch = {}, fields = event.target.querySelectorAll('[name]');
      Array.prototype.forEach.call(fields, function (input) { patch[input.name] = ['tools', 'write_paths', 'capabilities'].indexOf(input.name) >= 0 ? input.value.split(/\r?\n/).map(function (v) { return v.trim(); }).filter(Boolean) : input.value; });
      apply(D.updateNode(doc, selected, patch), '节点配置与 JSON 已同步');
    };
    root.onchange = function (event) {
      if (!hit(event.target, '.owo-wfc-add-tool') || !event.target.value) return;
      var textarea = root.querySelector('[name="tools"]'); if (!textarea) return;
      var names = textarea.value.split(/\r?\n/); if (names.indexOf(event.target.value) < 0) textarea.value = textarea.value.trim() + (textarea.value.trim() ? '\n' : '') + event.target.value;
      event.target.value = '';
    };
    function point(event, svg) {
      var matrix = svg.getScreenCTM && svg.getScreenCTM();
      if (matrix && svg.createSVGPoint) { var pt = svg.createSVGPoint(); pt.x = event.clientX; pt.y = event.clientY; var at = pt.matrixTransform(matrix.inverse()); return { x: at.x, y: at.y }; }
      var rect = svg.getBoundingClientRect(), layout = project(doc, helper).layout;
      return { x: (event.clientX - rect.left) / Math.max(rect.width, 1) * layout.width, y: (event.clientY - rect.top) / Math.max(rect.height, 1) * layout.height };
    }
    root.onpointerdown = function (event) {
      if (!valid || event.button !== 0) return;
      var port = hit(event.target, '[data-output-port]'), input = hit(event.target, '[data-input-port]'), node = hit(event.target, '[data-node-id]'), svg = root.querySelector('svg');
      if ((!port && !node) || input || !svg) return;
      var id = (port || node).getAttribute(port ? 'data-output-port' : 'data-node-id'), at = point(event, svg), position = project(doc, helper).layout.nodes.find(function (entry) { return entry.id === id; });
      if (!position) return;
      drag = { wire: !!port, id: id, svg: svg, node: node, x: at.x, y: at.y, dx: at.x - position.x, dy: at.y - position.y, moved: false, next: null };
      event.preventDefault();
    };
    function pointerMove(event) {
      if (!drag || !valid) return;
      var at = point(event, drag.svg);
      if (!drag.moved && Math.abs(at.x - drag.x) + Math.abs(at.y - drag.y) < 3) return;
      drag.moved = true;
      if (drag.wire) {
        var source = project(doc, helper).layout.nodes.find(function (entry) { return entry.id === drag.id; }), preview = root.querySelector('.owo-wfc-wire');
        if (preview) { preview.removeAttribute('hidden'); preview.setAttribute('d', 'M' + (source.x + 124) + ',' + (source.y + 24) + ' L' + at.x + ',' + at.y); }
      } else {
        var next = { x: Math.min(100000, Math.max(0, at.x - drag.dx)), y: Math.min(100000, Math.max(0, at.y - drag.dy)) }; drag.next = next;
        drag.node.setAttribute('transform', 'translate(' + next.x + ' ' + next.y + ')');
        var points = new Map(project(doc, helper).layout.nodes.map(function (node) { return [node.id, node]; })); points.set(drag.id, next);
        Array.prototype.forEach.call(root.querySelectorAll('[data-edge-from]'), function (path) { var from = points.get(path.getAttribute('data-edge-from')), to = points.get(path.getAttribute('data-edge-to')); if (from && to) path.setAttribute('d', helper.edgePath(from, to)); });
      }
    }
    function pointerUp(event) {
      if (!drag) return;
      var finished = drag; drag = null;
      if (!finished.moved) return;
      ignoreClick = Date.now() + 150;
      if (finished.wire) {
        var target = root.ownerDocument && root.ownerDocument.elementFromPoint ? root.ownerDocument.elementFromPoint(event.clientX, event.clientY) : event.target;
        var input = hit(target, '[data-input-port]');
        if (input && root.contains(input)) connect(finished.id, input.getAttribute('data-input-port'));
        else { pending = null; render(); notify('未连接到输入端口，配置未改变'); }
      } else apply(D.moveNode(doc, finished.id, finished.next), '布局已写入 JSON，保存或采纳后可跨浏览器恢复');
    }
    if (global.addEventListener) { global.addEventListener('pointermove', pointerMove); global.addEventListener('pointerup', pointerUp); }
    root.onpointercancel = function () { drag = null; pending = null; render(); };
    function setDocument(value) {
      var result = doc ? D.reconcile(doc, value) : D.normalize(value);
      if (!result.ok) { invalidate(result.error); return result; }
      if (doc && doc.template_id !== result.document.template_id) { history = []; future = []; }
      doc = result.document; valid = true; errorText = ''; selected = role() ? selected : null; edge = null; pending = null; drag = null;
      render(); return { ok: true, document: JSON.parse(JSON.stringify(doc)), problems: result.problems };
    }
    function invalidate(message) { valid = false; drag = null; root.innerHTML = '<p role="alert" class="owo-wfc-problems">' + escape(message) + '；画布编辑已停用，请修正 JSON。</p>'; }
    return { setDocument: setDocument, invalidate: invalidate, getDocument: function () { return valid ? JSON.parse(JSON.stringify(doc)) : null; },
      setCatalog: function (tools) { catalog = Array.isArray(tools) ? tools : []; var select = root.querySelector(".owo-wfc-add-tool"); if (select) select.innerHTML = toolOptions(); },
      dispose: function () { disposed = true; drag = null; if (global.removeEventListener) { global.removeEventListener('pointermove', pointerMove); global.removeEventListener('pointerup', pointerUp); } root.onclick = root.onkeydown = root.onsubmit = root.onchange = root.onpointerdown = root.onpointercancel = null; } };
  }
  global.OwoWorkflowCanvas = { create: create, project: project, renderGraphSvg: renderGraphSvg };
})(typeof window !== "undefined" ? window : globalThis);
