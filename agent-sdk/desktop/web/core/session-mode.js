// 会话工作模式的前端事实源：agent（Work，完整工具链）/ chat（对话）。
//
// 与后端 `owo_agent_core::SessionMode` 的线格式保持一致（"agent" / "chat"）。
// 前端只做展示与选择，真正的工具裁剪在执行层完成——这里绝不"假装"限制生效。
(function (root) {
  "use strict";

  var MODES = {
    agent: {
      id: "agent",
      label: "Work",
      short: "Work",
      description: "完整工作模式：读写工作区、执行命令、调用工具与 MCP",
      notice: "",
    },
    chat: {
      id: "chat",
      label: "对话",
      short: "对话",
      description: "只聊天与查资料，不读写本地文件、不执行命令",
      notice: "对话模式：可以联网查资料，但不会读取或修改本机文件",
    },
  };

  function parse(raw) {
    var value = String(raw == null ? "" : raw).trim().toLowerCase();
    if (value === "chat" || value === "conversation" || value === "dialogue") return "chat";
    if (value === "agent" || value === "default" || value === "") return "agent";
    return "";
  }

  function normalize(raw) {
    return parse(raw) || "agent";
  }

  function isChat(raw) {
    return parse(raw) === "chat";
  }

  function describe(raw) {
    return MODES[normalize(raw)];
  }

  function label(raw) {
    return describe(raw).short;
  }

  function notice(raw) {
    return describe(raw).notice;
  }

  function options() {
    return [MODES.agent, MODES.chat];
  }

  root.OwoSessionMode = Object.freeze({
    parse: parse,
    normalize: normalize,
    isChat: isChat,
    describe: describe,
    label: label,
    notice: notice,
    options: options,
  });
})(typeof window !== "undefined" ? window : globalThis);
