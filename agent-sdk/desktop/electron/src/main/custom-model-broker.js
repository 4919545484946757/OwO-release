"use strict";
const { normalize } = require("./custom-model-vault.js");
const http = require("node:http"), https = require("node:https");
function assertFrame(event, mainWindow, core) {
  if (!mainWindow || mainWindow.isDestroyed() || event.sender !== mainWindow.webContents ||
      (event.senderFrame?.routingId !== mainWindow.webContents.mainFrame.routingId || event.senderFrame?.processId !== mainWindow.webContents.mainFrame.processId)) throw Error("credentials/untrusted_frame");
  const url = new URL(event.senderFrame.url);
  if (!core || url.origin !== "http://127.0.0.1:" + core.port || !["/", "/index.html"].includes(url.pathname)) throw Error("credentials/untrusted_origin");
}
function probe(vault, input) {
  const model = normalize(input);
  const saved = vault.read().models.find(item => item.id === String(input.previousId || model.id) && !item.disabled);
  if (model.useFullUrl) return Promise.resolve({ ok: false, error: "credentials/full_url_requires_turn", note: "完整请求地址请通过实际回合验证；本次没有发起模型请求。" });
  if (!input.apiKey && saved?.sealed && (saved.baseUrl !== model.baseUrl || saved.apiFormat !== model.apiFormat)) throw Error("credentials/reenter_for_new_endpoint");
  const key = String(input.apiKey || (saved ? vault.secret(saved) : ""));
  const url = new URL(model.baseUrl.replace(/\/+$/, "") + "/models");
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (process.env.OWO_CLOUD_ENABLED === "false" && !local) throw Error("credentials/cloud_disabled");
  return new Promise((resolve, reject) => {
    const headers = model.apiFormat === "anthropic" ? { "x-api-key": key, "anthropic-version": "2023-06-01" } : (key ? { Authorization: "Bearer " + key } : {});
    const req = (url.protocol === "https:" ? https : http).request(url, { method: "GET", headers, timeout: 5000 }, response => {
      let bytes = 0, data = "";
      response.on("data", chunk => { bytes += chunk.length; if (bytes > 262144) req.destroy(Error("credentials/probe_too_large")); else data += chunk; });
      response.on("end", () => {
        let count = null; try { const body = JSON.parse(data); if (Array.isArray(body.data)) count = body.data.length; } catch {}
        resolve({ ok: response.statusCode >= 200 && response.statusCode < 300, status: response.statusCode, models: count });
      });
    });
    req.on("timeout", () => req.destroy(Error("credentials/probe_timeout")));
    req.on("error", () => reject(Error("credentials/probe_failed")));
    req.end();
  });
}
module.exports = { assertFrame, probe };
