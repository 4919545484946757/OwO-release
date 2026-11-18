// 提示词增强（✦）用哪个模型：跟随当前模型 / 指定云端模型 / 指定本地模型。
//
// 只存本机 localStorage —— 这是桌面端的一个偏好，不是服务端设置。核心的
// /prompt/enhance 已经支持按请求传 `model`（预设模型）或 `model_connection`
// （自定义连接），所以这里只负责“选谁”，不引入第二套配置存储，也不新增路由。
//
// 所选模型不可用时（被删、服务商未就绪）回退到“跟随当前模型”，并把原因写在
// 设置行的提示里 —— 回退是显式的，不静默改写用户输入。
(function (root) {
  "use strict";

  const KEY = "owo.prompt-enhance.model";
  const MODES = ["follow", "cloud", "local"];
  const HINTS = {
    follow: "与当前对话使用同一个模型",
    cloud: "使用所选云端模型；该模型不可用时回退到跟随当前模型",
    local: "使用所选自定义连接；连接被移除时回退到跟随当前模型",
  };

  /* ---------------- 纯逻辑（供单测） ---------------- */

  function normalize(raw) {
    const mode = raw && MODES.includes(raw.mode) ? raw.mode : "follow";
    const model = mode === "follow" ? "" : String((raw && raw.model) || "").trim();
    return { mode, model };
  }

  // 把偏好解析成请求体真正要用的模型。
  //   follow   → 调用方继续用当前对话的模型；
  //   cloud    → 只传 model（走核心默认 provider 的模型覆盖）；
  //   local    → 传 model + 自定义连接对象；
  // 解析不出来时退回 follow，绝不带着一个不存在的模型去发请求。
  function resolveWith(pref, cat) {
    const p = normalize(pref);
    if (p.mode === "follow" || !p.model) return { mode: "follow", model: "", useConnection: false };
    if (p.mode === "local") {
      const hit = ((cat && cat.customModels) || []).some(
        (item) => item && !item.disabled && String(item.id || "").trim() === p.model
      );
      return hit
        ? { mode: "local", model: p.model, useConnection: true }
        : { mode: "follow", model: "", useConnection: false };
    }
    const hit = ((cat && cat.entries) || []).some(
      (item) => item && !item.custom && String(item.id || "").trim() === p.model
    );
    return hit
      ? { mode: "cloud", model: p.model, useConnection: false }
      : { mode: "follow", model: "", useConnection: false };
  }

  function read() {
    try {
      return normalize(JSON.parse(root.localStorage.getItem(KEY) || "null"));
    } catch (_) {
      return { mode: "follow", model: "" };
    }
  }

  function write(next) {
    const pref = normalize(next);
    try {
      root.localStorage.setItem(KEY, JSON.stringify(pref));
    } catch (_) {
      /* 隐私模式/存储被禁用时保持内存态，不影响本次增强 */
    }
    return pref;
  }

  function catalog() {
    try {
      if (typeof root.composerModelCatalog === "function") return root.composerModelCatalog();
    } catch (_) {
      /* app.js 未就绪：按“没有可选模型”处理，界面会提示先配置 */
    }
    return { entries: [], customModels: [] };
  }

  function resolve() {
    return resolveWith(read(), catalog());
  }

  /* ---------------- 设置页接线 ---------------- */

  function el(id) {
    const doc = root.document;
    return doc && typeof doc.getElementById === "function" ? doc.getElementById(id) : null;
  }

  function fillOptions(select, list, pref) {
    const doc = root.document;
    while (select.firstChild) select.removeChild(select.firstChild);
    const placeholder = doc.createElement("option");
    placeholder.value = "";
    placeholder.textContent = list.length ? "选择模型…" : "暂无可选模型";
    select.appendChild(placeholder);
    for (const item of list) {
      const option = doc.createElement("option");
      option.value = item.id;
      option.textContent = item.label;
      select.appendChild(option);
    }
    select.value = list.some((item) => item.id === pref.model) ? pref.model : "";
    select.disabled = list.length === 0;
  }

  function refresh() {
    const modeSelect = el("enhanceModelMode");
    const targetSelect = el("enhanceModelTarget");
    const hint = el("enhanceModelHint");
    if (!modeSelect || !targetSelect) return read();

    const pref = read();
    modeSelect.value = pref.mode;

    if (pref.mode === "follow") {
      targetSelect.hidden = true;
      while (targetSelect.firstChild) targetSelect.removeChild(targetSelect.firstChild);
      if (hint) hint.textContent = HINTS.follow;
      return pref;
    }

    const cat = catalog();
    const list =
      pref.mode === "local"
        ? ((cat.customModels || [])
            .filter((item) => item && !item.disabled && String(item.id || "").trim())
            .map((item) => ({ id: String(item.id).trim(), label: item.label || String(item.id).trim() })))
        : ((cat.entries || [])
            .filter((item) => item && !item.custom && String(item.id || "").trim())
            .map((item) => ({ id: String(item.id).trim(), label: item.label || String(item.id).trim() })));

    targetSelect.hidden = false;
    fillOptions(targetSelect, list, pref);
    if (hint) {
      hint.textContent = list.length
        ? HINTS[pref.mode]
        : pref.mode === "local"
          ? "还没有自定义模型连接，请先在下方「额外模型连接」里添加"
          : "还没有可用的云端模型，请先在上方配置服务商";
    }
    return pref;
  }

  function bindSelect(select, handler) {
    if (!select || select.dataset.enhanceModelBound) return;
    select.dataset.enhanceModelBound = "1";
    select.addEventListener("change", handler);
  }

  function mount() {
    const modeSelect = el("enhanceModelMode");
    const targetSelect = el("enhanceModelTarget");
    bindSelect(modeSelect, () => {
      const mode = MODES.includes(modeSelect.value) ? modeSelect.value : "follow";
      const prev = read();
      // 换模式时清掉上一个模式的模型，避免把云端 id 带进本地列表。
      write({ mode, model: mode === prev.mode ? prev.model : "" });
      refresh();
    });
    bindSelect(targetSelect, () => {
      const pref = read();
      write({ mode: pref.mode, model: targetSelect.value });
      refresh();
    });
    refresh();
  }

  function boot() {
    mount();
    const doc = root.document;
    // 打开「配置」页时重算候选：服务商与自定义模型可能刚在别处改过。
    if (doc && typeof doc.addEventListener === "function") {
      doc.addEventListener(
        "click",
        (event) => {
          const node = event && event.target;
          const nav =
            node && typeof node.closest === "function"
              ? node.closest('.settings-nav-item[data-settings-tab="models"]')
              : null;
          if (nav) refresh();
        },
        true
      );
    }
    // 自定义模型是异步迁移的：设置页已存在时再补一次刷新。
    if (el("enhanceModelMode") && typeof root.setTimeout === "function") {
      root.setTimeout(refresh, 1200);
    }
  }

  if (root.document && root.document.readyState === "complete") boot();
  else if (root.document && typeof root.document.addEventListener === "function") {
    root.document.addEventListener("DOMContentLoaded", boot);
  }

  root.OwoPromptEnhanceSettings = Object.freeze({
    KEY,
    MODES,
    read,
    write,
    resolve,
    refresh,
    mount,
    _test: { normalize, resolveWith },
  });
})(typeof window !== "undefined" ? window : globalThis);
