"use strict";
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const PREFIX = "OWO_CUSTOM_MODEL_SECRET_";
const META = "OWO_CUSTOM_MODEL_BINDING_";
function normalize(input) {
  const id = String(input.id || "").trim();
  if (!id || id === "default" || Buffer.byteLength(id) > 200) throw Error("credentials/invalid_model");
  let url; try { url = new URL(String(input.baseUrl || "").trim()); } catch { throw Error("credentials/invalid_url"); }
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw Error("credentials/invalid_url");
  const format = input.apiFormat || "openai";
  if (!["openai", "anthropic"].includes(format)) throw Error("credentials/invalid_format");
  const out = { id, label: String(input.label || id).slice(0, 200), baseUrl: url.href, apiFormat: format, useFullUrl: input.useFullUrl === true };
  if (input.temperature != null) { const v = Number(input.temperature); if (!Number.isFinite(v) || v < 0 || v > 2) throw Error("credentials/invalid_temperature"); out.temperature = v; }
  if (input.timeoutSecs != null) { const v = Number(input.timeoutSecs); if (!Number.isInteger(v) || v < 1 || v > 3600) throw Error("credentials/invalid_timeout"); out.timeoutSecs = v; }
  return out;
}
function binding(model) { return { model: model.id, base_url: model.baseUrl, api_format: model.apiFormat }; }
function sameBinding(a, b) { return JSON.stringify(binding(a)) === JSON.stringify(binding(b)); }
class CustomModelVault {
  constructor(file, codec) { this.file = file; this.codec = codec; this.tail = Promise.resolve(); }
  requireEncryption() {
    if (!this.codec.isEncryptionAvailable() || this.codec.getSelectedStorageBackend?.() === "basic_text") throw Error("credentials/encryption_unavailable");
  }
  read() {
    if (!fs.existsSync(this.file)) return { version: 1, models: [] };
    let value; try { value = JSON.parse(fs.readFileSync(this.file, "utf8")); } catch { throw Error("credentials/store_corrupt"); }
    if (value.version !== 1 || !Array.isArray(value.models) || value.models.length > 100) throw Error("credentials/store_corrupt");
    return value;
  }
  secret(record) {
    if (!record.sealed) return "";
    this.requireEncryption();
    let payload; try { payload = JSON.parse(this.codec.decryptString(Buffer.from(record.sealed, "base64"))); } catch { throw Error("credentials/unlock_failed"); }
    if (payload.ref !== record.ref || JSON.stringify(payload.binding) !== JSON.stringify(binding(record))) throw Error("credentials/binding_mismatch");
    return String(payload.key || "");
  }
  seal(model, key) {
    this.requireEncryption();
    if (Buffer.byteLength(key) > 8192 || /[\x00-\x1f\x7f]/.test(key)) throw Error("credentials/invalid_key");
    const ref = crypto.randomUUID().replaceAll("-", "");
    return { ...model, ref, sealed: this.codec.encryptString(JSON.stringify({ key, ref, binding: binding(model) })).toString("base64") };
  }
  publicRecord(record) {
    const model = normalize(record);
    return { ...model, disabled: record.disabled === true, hasKey: Boolean(record.sealed) && !record.disabled, apiKeyEnv: record.sealed && !record.disabled ? PREFIX + record.ref : null, storage: "encrypted_desktop" };
  }
  snapshot() { return { version: 1, encryptionAvailable: this.codec.isEncryptionAvailable() && this.codec.getSelectedStorageBackend?.() !== "basic_text", models: this.read().models.map(r => this.publicRecord(r)) }; }
  environment(records = this.read().models) {
    const out = {};
    for (const record of records) {
      if (record.disabled || !record.sealed) continue;
      if (!/^[a-f0-9]{32}$/.test(record.ref)) throw Error("credentials/store_corrupt");
      out[PREFIX + record.ref] = this.secret(record);
      out[META + record.ref] = JSON.stringify(binding(record));
    }
    if (Object.entries(out).reduce((n, [k,v]) => n + k.length + v.length + 2, 0) > 24000) throw Error("credentials/environment_limit");
    return out;
  }
  async acquireLock() {
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    const lock = this.file + ".lock";
    for (let attempt = 0; attempt < 30; attempt++) {
      try { const fd = fs.openSync(lock, "wx", 0o600); fs.writeSync(fd, String(process.pid)); return () => { fs.closeSync(fd); fs.unlinkSync(lock); }; }
      catch (error) {
        if (error.code !== "EEXIST") throw error;
        let owner; try { owner = Number(fs.readFileSync(lock, "utf8")); } catch (e) { if (e.code === "ENOENT") continue; throw e; }
        if (Number.isInteger(owner) && owner > 0) {
          try { process.kill(owner, 0); } catch (e) { if (e.code === "ESRCH") { fs.unlinkSync(lock); continue; } }
        }
        await new Promise(resolve => setTimeout(resolve, 50));
      }
    }
    throw Error("credentials/store_busy");
  }
  transact(action, sync) {
    const run = this.tail.catch(() => {}).then(async () => {
      const release = await this.acquireLock();
      try {
      const data = this.read(), before = data.models.map(record => ({ ...record }));
      const result = action(data);
      if (data.models.length > 100) throw Error("credentials/model_limit");
      this.environment(data.models);
      // Publish new endpoint-bound references; roll back partial host changes on failure.
      if (sync) try { await sync(data.models, before); } catch (error) {
        try { await sync(before, data.models); } catch { throw Error("credentials/host_recovery_required"); }
        throw error;
      }
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const temp = this.file + "." + crypto.randomUUID() + ".tmp";
      try { fs.writeFileSync(temp, JSON.stringify(data), { encoding: "utf8", mode: 0o600 }); fs.renameSync(temp, this.file); }
      catch (error) { if (sync) try { await sync(before, data.models); } catch { throw Error("credentials/host_recovery_required"); } throw error; }
      finally { if (fs.existsSync(temp)) fs.unlinkSync(temp); }
      return { ...this.snapshot(), ...result };
      } finally { release(); }
    });
    this.tail = run; return run;
  }
  upsert(input, sync) {
    return this.transact(data => {
      const model = normalize(input);
      const old = data.models.find(r => r.id === String(input.previousId || model.id) && !r.disabled);
      const given = String(input.apiKey || "").trim();
      if (!given && old?.sealed && (old.baseUrl !== model.baseUrl || old.apiFormat !== model.apiFormat)) throw Error("credentials/reenter_for_new_endpoint");
      const key = given || (old ? this.secret(old) : "");
      const record = key ? this.seal(model, key) : model;
      data.models = data.models.filter(r => r.id !== model.id && r.id !== old?.id);
      if (old && old.id !== model.id) data.models.push({ ...this.publicRecord(old), disabled: true, hasKey: false, apiKeyEnv: null });
      data.models.push(record); return {};
    }, sync);
  }
  remove(id, sync) {
    return this.transact(data => {
      const old = data.models.find(r => r.id === id);
      if (!old) return {};
      data.models = data.models.map(r => r.id === id ? { ...normalize(r), disabled: true } : r);
      return {};
    }, sync);
  }
  migrate(models, keys, sync) {
    return this.transact(data => {
      const migratedIds = [], conflictedIds = [];
      for (const input of models || []) {
        const model = normalize(input), old = data.models.find(r => r.id === model.id);
        const key = String(keys && Object.prototype.hasOwnProperty.call(keys, model.id) ? keys[model.id] : "").trim();
        if (old) {
          const equal = !old.disabled && sameBinding(old, model) && (!key || this.secret(old) === key);
          (equal ? migratedIds : conflictedIds).push(model.id); continue;
        }
        data.models.push(key ? this.seal(model, key) : model); migratedIds.push(model.id);
      }
      return { migratedIds, conflictedIds };
    }, sync);
  }
}
module.exports = { CustomModelVault, normalize, PREFIX, META };
