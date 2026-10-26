(function (root) {
  "use strict";

  var MAX_BYTES = 32 * 1024 * 1024;
  var MIME = { image: ["image/png", "image/jpeg", "image/gif", "image/webp"], audio: ["audio/mpeg", "audio/mp3", "audio/wav", "audio/ogg", "audio/flac"] };
  function validMime(kind, mime) { return !!MIME[kind] && MIME[kind].indexOf(String(mime || "").split(";")[0].trim().toLowerCase()) >= 0; }
  var MEDIA_TOOLS = {
    generate_image: "image",
    text_to_speech: "audio",
  };

  function parsePreview(preview) {
    var text = String(preview || "").trim();
    if (!text) return null;
    var start = text.indexOf("{");
    var end = text.lastIndexOf("}");
    if (start < 0 || end <= start) return null;
    try {
      var value = JSON.parse(text.slice(start, end + 1));
      return value && typeof value === "object" ? value : null;
    } catch {
      return null;
    }
  }

  /** 从 tool_result 事件里取出媒体产物：{kind:"image"|"audio", path, bytes}。 */
  function mediaFromToolResult(payload) {
    if (!payload || payload.ok === false) return null;
    var tool = String(payload.tool || "");
    var kind = MEDIA_TOOLS[tool];
    if (!kind) return null;
    // New events carry authoritative metadata independently of the human preview.
    if (Array.isArray(payload.outputs)) {
      var output = payload.outputs.find(function (item) {
        return item && item.kind === kind && /^[A-Za-z0-9_.:-]{1,200}$/.test(item.artifact_id || "") &&
          validMime(kind, item.mime_type) && (item.sha256 == null || /^[a-f0-9]{64}$/.test(item.sha256)) && Number.isSafeInteger(item.bytes) && item.bytes > 0 && item.bytes <= MAX_BYTES;
      });
      if (!output) return null;
      return {
        kind: kind, tool: tool, path: String(output.path || output.artifact_id), bytes: output.bytes,
        artifactId: output.artifact_id, mimeType: output.mime_type, dataUrl: "",
        ...(output.sha256 ? { expectedSha256: output.sha256 } : {}),
        contentPath: "/artifacts/" + encodeURIComponent(output.artifact_id) + "/content?raw=true",
      };
    }
    var value = parsePreview(payload.preview);
    if (!value || typeof value.path !== "string") return null;
    var path = value.path.trim();
    if (!path) return null;
    var prefix = kind === "image" ? /^data:image\// : /^data:audio\//;
    var dataUrl = typeof value.data_url === "string" && prefix.test(value.data_url)
      ? value.data_url
      : "";
    return {
      kind: kind,
      tool: tool,
      path: path,
      bytes: Number.isFinite(Number(value.bytes)) ? Number(value.bytes) : 0,
      dataUrl: dataUrl,
    };
  }

  /** 给用户看的一句话：图片/音频各自说清楚存到哪了。 */
  function describeMedia(media) {
    if (!media) return "";
    var label = media.kind === "image" ? "已生成图片" : "已生成语音";
    var size = media.bytes > 0 ? "（" + formatBytes(media.bytes) + "）" : "";
    return label + size + "：" + media.path;
  }

  function formatBytes(bytes) {
    if (bytes >= 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB";
    if (bytes >= 1024) return Math.round(bytes / 1024) + " KB";
    return bytes + " B";
  }

  // One lifecycle for live chat, historical replay and Team results. Credentials stay
  // in OwoApi headers; no token, local path or remote URL becomes a media src.
  var mounted = new Map();
  var observer;
  function collectRemoved() {
    mounted.forEach(function (handle, node) { if (!node.isConnected) handle.dispose(); });
  }
  function mount(container, media, options) {
    options = options || {};
    if (!container || !media || !media.contentPath) return null;
    if (mounted.has(container)) return mounted.get(container);
    var doc = container.ownerDocument;
    var button = doc.createElement("button");
    button.type = "button";
    button.className = "owo-media-copy";
    var status = doc.createElement("span");
    status.setAttribute("role", "status");
    var preview, controller, objectUrl = "", disposed = false, generation = 0;
    var label = media.kind === "image" ? "查看图片" : "播放语音";
    button.textContent = label;
    container.append(button, status);
    function release() {
      if (preview) { preview.removeAttribute("src"); if (preview.load) preview.load(); preview.remove(); preview = null; }
      if (objectUrl) { root.URL.revokeObjectURL(objectUrl); objectUrl = ""; }
    }
    function cancel() {
      generation += 1;
      if (controller) controller.abort();
      controller = null;
      if (!disposed) { status.textContent = "已取消加载"; button.textContent = "重新加载"; }
    }
    async function load() {
      if (disposed) return;
      if (controller) { cancel(); return; }
      release();
      var api = options.api || root.OwoApi;
      var current = ++generation;
      controller = new root.AbortController();
      status.textContent = "正在加载…";
      button.textContent = "取消加载";
      try {
        if (!api || typeof api.download !== "function") throw new Error("产物服务不可用");
        var blob = await api.download(media.contentPath, { signal: controller.signal, maxResponseBytes: MAX_BYTES });
        if (disposed || current !== generation || !container.isConnected) return;
        if (!validMime(media.kind, blob.type) || blob.type.split(";")[0] !== media.mimeType) throw new Error("媒体格式与登记信息不符");
        if (blob.size !== media.bytes || blob.size > MAX_BYTES) throw new Error("媒体大小与登记信息不符");
        if (media.expectedSha256) {
          if (!/^[a-f0-9]{64}$/.test(media.expectedSha256) || !root.crypto || !root.crypto.subtle) throw new Error("无法校验媒体内容摘要");
          var hash = await root.crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
          if (disposed || current !== generation || !container.isConnected) return;
          var actual = Array.from(new Uint8Array(hash), function (value) { return value.toString(16).padStart(2, "0"); }).join("");
          if (actual !== media.expectedSha256) throw new Error("媒体内容与不可变引用的摘要不符");
        }
        objectUrl = root.URL.createObjectURL(blob);
        preview = doc.createElement(media.kind === "audio" ? "audio" : "img");
        preview.className = media.kind === "audio" ? "owo-media-audio" : "owo-media-inline";
        if (media.kind === "audio") { preview.controls = true; preview.preload = "metadata"; }
        else { preview.alt = media.path; preview.loading = "lazy"; }
        preview.onerror = function () { release(); status.textContent = "媒体文件无法解码，请重新加载"; button.textContent = "重新加载"; };
        preview.src = objectUrl;
        container.appendChild(preview);
        status.textContent = "";
        button.textContent = "重新加载";
        if (options.onLoad) options.onLoad();
      } catch (error) {
        if (disposed || current !== generation) return;
        var code = Number(error && error.status);
        status.textContent = code === 401 || code === 403 ? "没有读取该产物的权限" : code === 404 ? "产物文件不存在" : String(error && error.message || "媒体加载失败");
        button.textContent = "重试加载";
      } finally { if (current === generation) controller = null; }
    }
    button.addEventListener("click", load);
    var handle = { load: load, cancel: cancel, dispose: function () {
      if (disposed) return;
      disposed = true;
      cancel(); release();
      button.removeEventListener("click", load);
      mounted.delete(container);
      if (!mounted.size && observer) { observer.disconnect(); observer = null; }
    } };
    mounted.set(container, handle);
    if (!observer && root.MutationObserver && doc.documentElement) {
      observer = new root.MutationObserver(collectRemoved);
      observer.observe(doc.documentElement, { childList: true, subtree: true });
    }
    return handle;
  }
  function disposeWithin(container) {
    if (!container) return;
    mounted.forEach(function(handle,node) {
      if (node===container || container.contains && container.contains(node)) handle.dispose();
    });
  }
  function hydrate(container) {
    if (!container) return;
    container.querySelectorAll("[data-media-artifact]").forEach(function (node) {
      try {
        var payload = JSON.parse(node.getAttribute("data-media-artifact"));
        var media = mediaFromToolResult(payload);
        if (media) mount(node, media);
      } catch (_) { /* Old or invalid event: keep its readable preview. */ }
    });
  }
  function renderInto(container, html) {
    var previous = new Map();
    container.querySelectorAll("[data-media-artifact]").forEach(function (node) {
      previous.set(node.getAttribute("data-media-artifact"), node);
    });
    container.innerHTML = html;
    container.querySelectorAll("[data-media-artifact]").forEach(function (node) {
      var old = previous.get(node.getAttribute("data-media-artifact"));
      if (old && mounted.has(old)) node.replaceWith(old);
    });
    hydrate(container);
  }
  root.OwoMediaResult = Object.freeze({ mediaFromToolResult: mediaFromToolResult, describeMedia: describeMedia, mount: mount, hydrate: hydrate, renderInto: renderInto, disposeWithin: disposeWithin });
})(typeof window !== "undefined" ? window : globalThis);
