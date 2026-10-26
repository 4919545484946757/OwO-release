/* Canonical JSON editing for TeamTemplate role graphs. No network or execution authority. */
(function (global) {
  "use strict";
  var sequence = 0;
  var ID = /^[a-zA-Z][a-zA-Z0-9_.-]{0,127}$/;
  var arrays = ["tools", "write_paths", "capabilities"];
  var strings = ["role", "assignee", "worker", "model", "handoff_contract", "verify"];
  function copy(value) { return JSON.parse(JSON.stringify(value)); }
  function hash(text) {
    var value = 2166136261;
    for (var i = 0; i < text.length; i += 1) value = Math.imul(value ^ text.charCodeAt(i), 16777619);
    return (value >>> 0).toString(16);
  }
  function freshId() {
    return global.crypto && typeof global.crypto.randomUUID === "function" ? "node-" + global.crypto.randomUUID() :
      "node-" + Date.now().toString(36) + "-" + (++sequence).toString(36) + "-" + Math.random().toString(36).slice(2, 10);
  }
  function validPosition(position) {
    return position && typeof position.x === "number" && typeof position.y === "number" &&
      Number.isFinite(position.x) && Number.isFinite(position.y) && position.x >= 0 && position.y >= 0 && position.x <= 100000 && position.y <= 100000;
  }
  function normalize(value) {
    try {
      if (!value || typeof value !== "object" || Array.isArray(value) || !Array.isArray(value.roles)) throw Error("配置必须包含 roles 数组");
      var doc = copy(value);
      var roles = new Set();
      doc.roles.forEach(function (role) {
        if (!role || typeof role.role !== "string" || !role.role.trim()) throw Error("存在未命名的角色");
        if (role.role !== role.role.trim()) throw Error("角色名前后不能包含空白");
        if (roles.has(role.role)) throw Error("角色名重复：" + role.role);
        roles.add(role.role);
      });
      if (doc.roles.length > 12) throw Error("最多 12 个角色");
      if (doc.editor === undefined || doc.editor === null) doc.editor = { version: 1, nodes: [] };
      if (doc.editor.version !== 1 || !Array.isArray(doc.editor.nodes) || doc.editor.nodes.length > 12) throw Error("editor 必须使用 version=1 和 nodes 数组");
      var ids = new Set();
      doc.roles.forEach(function (role) {
        if (role.node_id === undefined || role.node_id === null) {
          var legacy = doc.editor.nodes.find(function (entry) { return entry && entry.role === role.role; });
          var base = legacy && legacy.id || "node-" + hash(String(doc.template_id || "") + "\n" + role.role), id = base, collision = 0;
          while (ids.has(id)) id = base + "-" + (++collision);
          role.node_id = id;
        }
        if (typeof role.node_id !== "string" || !ID.test(role.node_id) || ids.has(role.node_id)) throw Error("节点 ID 非法或重复");
        ids.add(role.node_id);
      });
      var covered = new Set();
      doc.editor.nodes.forEach(function (entry) {
        if (!entry || !ids.has(entry.id) || covered.has(entry.id)) throw Error("布局包含不存在或重复的节点 ID");
        if (entry.position !== undefined && !validPosition(entry.position)) throw Error("节点位置必须是范围内的有限数字");
        delete entry.role;
        covered.add(entry.id);
      });
      doc.roles.forEach(function (role) { if (!covered.has(role.node_id)) doc.editor.nodes.push({ id: role.node_id }); });
      return { ok: true, document: doc, problems: validate(doc) };
    } catch (error) { return { ok: false, error: String(error.message || error) }; }
  }
  function validate(doc) {
    var problems = [], roles = new Map();
    (doc.roles || []).forEach(function (role) { roles.set(role.role, role); });
    roles.forEach(function (role) {
      if (role.depends_on !== undefined && (!Array.isArray(role.depends_on) || role.depends_on.some(function (name) { return typeof name !== "string"; }))) {
        problems.push("角色 " + role.role + " 的 depends_on 必须是字符串数组"); return;
      }
      (role.depends_on || []).forEach(function (dependency) { if (!roles.has(dependency)) problems.push("角色 " + role.role + " 依赖不存在的角色：" + dependency); });
    });
    var visiting = new Set(), visited = new Set(), cycle = false;
    function visit(name) {
      if (visiting.has(name)) { cycle = true; return; }
      if (visited.has(name) || !roles.has(name)) return;
      visiting.add(name);
      var dependencies = roles.get(name).depends_on;
      if (Array.isArray(dependencies)) dependencies.forEach(function (dependency) { if (typeof dependency === "string") visit(dependency); });
      visiting.delete(name); visited.add(name);
    }
    roles.forEach(function (_, name) { visit(name); });
    if (cycle) problems.push("角色依赖存在环");
    return problems;
  }
  function node(doc, id) {
    var meta = doc.editor.nodes.find(function (entry) { return entry.id === id; });
    if (!meta) throw Error("节点不存在：" + id);
    var role = doc.roles.find(function (entry) { return entry.node_id === id; });
    if (!role) throw Error("节点角色不存在");
    return { meta: meta, role: role };
  }
  function change(value, action) {
    var result = normalize(value);
    if (!result.ok) return result;
    try {
      action(result.document);
      result.problems = validate(result.document);
      return result;
    } catch (error) { return { ok: false, error: String(error.message || error) }; }
  }
  function addNode(value, fields) {
    return change(value, function (doc) {
      if (doc.roles.length >= 12) throw Error("最多 12 个角色");
      var role = fields && typeof fields.role === "string" ? fields.role.trim() : "";
      if (!role || doc.roles.some(function (entry) { return entry.role === role; })) throw Error("请输入唯一的角色名");
      var id = freshId();
      while (doc.editor.nodes.some(function (entry) { return entry.id === id; })) id = freshId();
      doc.roles.push({ node_id: id, role: role, assignee: "agent", depends_on: [] });
      doc.editor.nodes.push({ id: id });
      applyFields(doc, id, fields);
    });
  }
  function applyFields(doc, id, patch) {
    if (!patch || typeof patch !== "object" || Array.isArray(patch)) throw Error("节点参数必须是对象");
    var target = node(doc, id);
    Object.keys(patch).forEach(function (key) {
      if (arrays.indexOf(key) >= 0) {
        if (!Array.isArray(patch[key]) || patch[key].some(function (value) { return typeof value !== "string"; })) throw Error(key + " 必须是字符串数组");
        target.role[key] = Array.from(new Set(patch[key].map(function (text) { return text.trim(); }).filter(Boolean)));
      } else if (strings.indexOf(key) >= 0) {
        if (typeof patch[key] !== "string") throw Error(key + " 必须是字符串");
        var text = patch[key].trim();
        if (key === "role") {
          if (!text || doc.roles.some(function (entry) { return entry !== target.role && entry.role === text; })) throw Error("请输入唯一的角色名");
          var previous = target.role.role;
          doc.roles.forEach(function (entry) {
            if (Array.isArray(entry.depends_on)) entry.depends_on = entry.depends_on.map(function (name) { return name === previous ? text : name; });
          });
        }
        if (key === "assignee" && ["agent", "human", "worker"].indexOf(text) < 0) throw Error("assignee 必须是 agent/human/worker");
        if (key !== "role" && key !== "assignee" && !text) delete target.role[key];
        else target.role[key] = text;
      } else throw Error("不能直接修改字段：" + key);
    });
  }
  function updateNode(value, id, patch) { return change(value, function (doc) { applyFields(doc, id, patch); }); }
  function removeNode(value, id) {
    return change(value, function (doc) {
      var name = node(doc, id).role.role;
      doc.roles = doc.roles.filter(function (role) { return role.role !== name; });
      doc.editor.nodes = doc.editor.nodes.filter(function (entry) { return entry.id !== id; });
      doc.roles.forEach(function (role) { if (Array.isArray(role.depends_on)) role.depends_on = role.depends_on.filter(function (dependency) { return dependency !== name; }); });
    });
  }
  function connection(value, source, target, remove) {
    return change(value, function (doc) {
      var from = node(doc, source), to = node(doc, target);
      if (source === target) throw Error("不能连接到自身");
      if (!Array.isArray(to.role.depends_on)) to.role.depends_on = [];
      to.role.depends_on = to.role.depends_on.filter(function (name) { return name !== from.role.role; });
      if (!remove) {
        to.role.depends_on.push(from.role.role);
        if (validate(doc).some(function (problem) { return problem.indexOf("存在环") >= 0; })) throw Error("这条连接会形成依赖环");
      }
    });
  }
  function moveNode(value, id, position) {
    return change(value, function (doc) {
      if (!validPosition(position)) throw Error("节点位置必须是范围内的有限数字");
      node(doc, id).meta.position = { x: Math.round(position.x * 10) / 10, y: Math.round(position.y * 10) / 10 };
    });
  }
  function graphTemplate(value) {
    var result = normalize(value);
    if (!result.ok) return result;
    var doc = result.document, ids = new Map(doc.roles.map(function (entry) { return [entry.role, entry.node_id]; }));
    return { ok: true, document: doc, problems: result.problems, template: { roles: doc.roles.map(function (role) {
      return { role: ids.get(role.role), label: role.role, assignee: role.assignee, depends_on: (Array.isArray(role.depends_on) ? role.depends_on : []).map(function (name) { return ids.get(name) || "missing-" + hash(name); }) };
    }) } };
  }
  // JSON keeps node_id authoritative. Renaming an existing ID repairs unchanged dangling name references.
  function reconcile(previous, incoming) {
    var prior = normalize(previous);
    if (!prior.ok) return prior;
    var candidate;
    try { candidate = copy(incoming); } catch (error) { return { ok: false, error: "JSON 文档无法序列化" }; }
    if (candidate && Array.isArray(candidate.roles) && candidate.editor && Array.isArray(candidate.editor.nodes)) {
      var remaining = new Set(candidate.roles.map(function (role) { return role && role.node_id; }));
      var previousIds = new Set(prior.document.roles.map(function (role) { return role.node_id; }));
      candidate.editor.nodes = candidate.editor.nodes.filter(function (node) { return !node || !previousIds.has(node.id) || remaining.has(node.id); });
    }
    var next = normalize(candidate);
    if (!next.ok) return next;
    var byId = new Map(next.document.roles.map(function (role) { return [role.node_id, role.role]; }));
    var names = new Set(next.document.roles.map(function (role) { return role.role; })), renamed = new Map();
    prior.document.roles.forEach(function (role) {
      var current = byId.get(role.node_id);
      if (current && current !== role.role && !names.has(role.role)) renamed.set(role.role, current);
    });
    next.document.roles.forEach(function (role) {
      if (Array.isArray(role.depends_on)) role.depends_on = role.depends_on.map(function (name) { return renamed.get(name) || name; });
    });
    next.problems = validate(next.document);
    return next;
  }
  global.OwoWorkflowDocument = { normalize: normalize, reconcile: reconcile, validate: validate, addNode: addNode, updateNode: updateNode, removeNode: removeNode,
    connect: function (value, from, to) { return connection(value, from, to, false); },
    disconnect: function (value, from, to) { return connection(value, from, to, true); }, moveNode: moveNode, graphTemplate: graphTemplate };
})(typeof window !== "undefined" ? window : globalThis);
