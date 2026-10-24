(function (root) {
  "use strict";

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

  root.OwoMediaResult = Object.freeze({ mediaFromToolResult: mediaFromToolResult, describeMedia: describeMedia });
})(typeof window !== "undefined" ? window : globalThis);
