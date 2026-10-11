"use strict";
const fs = require("node:fs");
function candidateOrigins(logFile, userData) {
  const found = [];
  if (fs.existsSync(logFile)) {
    const fd = fs.openSync(logFile, "r"), buffer = Buffer.alloc(65536), start = Date.now();
    let rest = "", read, total = 0;
    try {
      while (total < 67108864 && Date.now() - start < 1000 && (read = fs.readSync(fd, buffer, 0, buffer.length, null))) {
        total += read; const lines = (rest + buffer.subarray(0, read).toString("utf8")).split(/\r?\n/); rest = lines.pop().slice(-4096);
        for (const line of lines) {
          if (!line.includes('"event":"core_ready"') && !line.includes('"event": "core_ready"')) continue;
          try { const value = JSON.parse(line.replace(/^\[core\]\s*/, "")); if (value.event === "core_ready") found.push(value.port); } catch {}
        }
      }
      if (rest.includes('"core_ready"')) try { const value = JSON.parse(rest.replace(/^\[core\]\s*/, "")); if (value.event === "core_ready") found.push(value.port); } catch {}
    } finally { fs.closeSync(fd); }
  }
  // Only discover origin names from the application's own storage. Never decode,
  // return or log the stored values.
  if (userData) {
    const dir = require("node:path").join(userData, "Local Storage", "leveldb");
    let total = 0;
    if (fs.existsSync(dir)) for (const name of fs.readdirSync(dir)) {
      const file = require("node:path").join(dir, name);
      if (!/\.(ldb|log)$/.test(name) || fs.statSync(file).size > 8388608 || total > 25165824) continue;
      const bytes = fs.readFileSync(file); total += bytes.length;
      for (const match of bytes.toString("latin1").matchAll(/http:\/\/127\.0\.0\.1:(\d{1,5})/g)) found.push(Number(match[1]));
    }
  }
  return [...new Set(found.filter(port => Number.isInteger(port) && port > 0 && port < 65536).reverse())].slice(0, 8).map(port => "http://127.0.0.1:" + port);
}
async function migrateOrigins({ BrowserWindow, session, net, logFile, userData, vault }) {
  const origins = candidateOrigins(logFile, userData);
  if (!origins.length) return { migrated: 0, conflicts: 0 };
  const allowed = new Set(origins), protocol = session.defaultSession.protocol;
  let window, migrated = 0, conflicts = 0;
  const deadline = Date.now() + 3000;
  // The helper uses the application's own partition. Intercept the known loopback
  // origins before navigation; never visit an old port that may now belong to another app.
  protocol.handle("http", request => {
    if (allowed.has(new URL(request.url).origin)) return new Response("<!doctype html><title>Storage migration</title>", {
      headers: { "Content-Type": "text/html; charset=utf-8", "Content-Security-Policy": "default-src 'none'" },
    });
    return net.fetch(request, { bypassCustomProtocolHandlers: true });
  });
  try {
    window = new BrowserWindow({ show: false, webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false } });
    for (const origin of origins) {
      if (Date.now() >= deadline) break;
      await Promise.race([window.loadURL(origin + "/__owo_storage_migration"), new Promise((_, reject) => setTimeout(() => reject(Error("credentials/migration_timeout")), 1000))]);
      const old = await window.webContents.executeJavaScript("({models:JSON.parse(localStorage.getItem('owo.custom-models')||'[]'),keys:JSON.parse(localStorage.getItem('owo.model-keys')||'{}')})");
      if (!Array.isArray(old.models) || !old.models.length) continue;
      const result = await vault.migrate(old.models, old.keys);
      migrated += result.migratedIds.length; conflicts += result.conflictedIds.length;
      // Compare with the captured version before cleanup; preserve concurrent edits.
      const cleanup = { models: old.models, keys: old.keys, ids: result.migratedIds };
      await window.webContents.executeJavaScript("(function(input){ const models=JSON.parse(localStorage.getItem('owo.custom-models')||'[]'),keys=JSON.parse(localStorage.getItem('owo.model-keys')||'{}');const ids=input.ids.filter(id=>JSON.stringify(models.find(m=>m.id===id))===JSON.stringify(input.models.find(m=>m.id===id))&&keys[id]===input.keys[id]);for(const id of ids)delete keys[id];const kept=models.filter(m=>!ids.includes(m.id));if(kept.length)localStorage.setItem('owo.custom-models',JSON.stringify(kept));else localStorage.removeItem('owo.custom-models');if(Object.keys(keys).length)localStorage.setItem('owo.model-keys',JSON.stringify(keys));else localStorage.removeItem('owo.model-keys');})(" + JSON.stringify(cleanup) + ")");
    }
  } finally { if (window && !window.isDestroyed()) window.destroy(); protocol.unhandle("http"); }
  return { migrated, conflicts };
}
module.exports = { candidateOrigins, migrateOrigins };
