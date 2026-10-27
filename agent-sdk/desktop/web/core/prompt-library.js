// 自定义提示词（persona 预设）的前端纯逻辑。
//
// 后端 `prompt_api` 是唯一存储事实源；这里只做：
// - 校验（与后端上限一致，先在前端拦一次，避免无谓往返）
// - 从"当前会话 system_prompt"反查是哪条预设（UI 显示名字而不是一大段文本）
(function (root) {
  "use strict";

  var LIMITS = { maxPresets: 50, maxNameChars: 60, maxContentChars: 20000 };

  function validate(name, content) {
    var trimmedName = String(name == null ? "" : name).trim();
    var trimmedContent = String(content == null ? "" : content).trim();
    if (!trimmedName) return { ok: false, error: "提示词名称不能为空" };
    if (Array.from(trimmedName).length > LIMITS.maxNameChars) {
      return { ok: false, error: "名称最多 " + LIMITS.maxNameChars + " 字" };
    }
    if (!trimmedContent) return { ok: false, error: "提示词内容不能为空" };
    if (Array.from(trimmedContent).length > LIMITS.maxContentChars) {
      return { ok: false, error: "内容最多 " + LIMITS.maxContentChars + " 字" };
    }
    return { ok: true, name: trimmedName, content: trimmedContent };
  }

  /// 按正文反查预设：服务端只存 system_prompt 字符串，没有"激活 id"。
  function findActive(presets, systemPrompt) {
    var content = String(systemPrompt == null ? "" : systemPrompt).trim();
    if (!content) return null;
    var list = Array.isArray(presets) ? presets : [];
    for (var i = 0; i < list.length; i += 1) {
      if (String(list[i] && list[i].content || "").trim() === content) return list[i];
    }
    return null;
  }

  function chipLabel(presets, systemPrompt) {
    if (!String(systemPrompt == null ? "" : systemPrompt).trim()) return "默认提示词";
    var active = findActive(presets, systemPrompt);
    return active ? String(active.name) : "自定义提示词";
  }

  /// 应用时的请求体：null 表示清除（回到默认提示词）。
  function applyBody(systemPrompt) {
    var content = String(systemPrompt == null ? "" : systemPrompt).trim();
    return { prompt: content ? content : null };
  }

  /// 建会话时只有非空才带上 system_prompt（空串会让服务端存成空 persona）。
  function createSessionPrompt(systemPrompt) {
    var content = String(systemPrompt == null ? "" : systemPrompt).trim();
    return content || null;
  }

  /// 导出为可分享的 JSON（只含 name/content，不泄露 id 与时间戳）。
  function serialize(presets) {
    var list = Array.isArray(presets) ? presets : [];
    var items = [];
    for (var i = 0; i < list.length; i += 1) {
      var name = String((list[i] && list[i].name) || "").trim();
      var content = String((list[i] && list[i].content) || "").trim();
      if (!name || !content) continue;
      items.push({ name: name, content: content });
    }
    return JSON.stringify({ version: 1, prompts: items }, null, 2);
  }

  /// 解析导入文本。接受 {version,prompts:[…]} 或裸数组；同名预设视为更新（带 id），
  /// 这样 Hermes 式"再导入一次"不会堆出一堆同名条目。
  function planImport(text, existing) {
    var raw = String(text == null ? "" : text).trim();
    if (!raw) return { ok: false, error: "请先粘贴要导入的 JSON" };
    var parsed = null;
    try {
      parsed = JSON.parse(raw);
    } catch (_) {
      return { ok: false, error: "不是合法的 JSON" };
    }
    var items = Array.isArray(parsed) ? parsed : (parsed && parsed.prompts);
    if (!Array.isArray(items)) return { ok: false, error: "缺少 prompts 数组" };
    if (!items.length) return { ok: false, error: "没有可导入的提示词" };

    var current = Array.isArray(existing) ? existing : [];
    var byName = {};
    for (var i = 0; i < current.length; i += 1) {
      var key = String((current[i] && current[i].name) || "").trim();
      if (key && !byName[key]) byName[key] = current[i];
    }

    var entries = [];
    var seen = {};
    var updated = 0;
    var created = 0;
    for (var j = 0; j < items.length; j += 1) {
      var item = items[j] || {};
      var checked = validate(item.name, item.content);
      if (!checked.ok) {
        return { ok: false, error: "第 " + (j + 1) + " 条无效：" + checked.error };
      }
      if (seen[checked.name]) continue; // 同文件内重名：保留第一条
      seen[checked.name] = true;
      var match = byName[checked.name];
      entries.push({
        id: match ? String(match.id) : "",
        name: checked.name,
        content: checked.content,
      });
      if (match) updated += 1;
      else created += 1;
    }
    if (!entries.length) return { ok: false, error: "没有可导入的提示词" };
    var over = current.length + created - LIMITS.maxPresets;
    if (over > 0) {
      return {
        ok: false,
        error: "导入后将超出上限（" + LIMITS.maxPresets + " 条），请先删除 " + over + " 条",
      };
    }
    return { ok: true, entries: entries, created: created, updated: updated };
  }

  root.OwoPromptLibrary = Object.freeze({
    LIMITS: LIMITS,
    validate: validate,
    findActive: findActive,
    chipLabel: chipLabel,
    applyBody: applyBody,
    createSessionPrompt: createSessionPrompt,
    serialize: serialize,
    planImport: planImport,
  });
})(typeof window !== "undefined" ? window : globalThis);
