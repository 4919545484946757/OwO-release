import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/plugin-market.panel.js", import.meta.url), "utf8");

function harness(getOverride, postOverride, confirmOverride) {
  const elements = new Map();
  function element(tag = "div") {
    return {
      tagName: tag.toUpperCase(),
      innerHTML: "", textContent: "", value: "", disabled: false, className: "", handlers: {}, children: [],
      addEventListener(name, callback) { this.handlers[name] = callback; },
      setAttribute(name, value) { this[name] = value; },
      removeAttribute(name) { delete this[name]; },
      appendChild(child) { this.children.push(child); return child; },
      querySelector(selector) {
        if (selector === "button") {
          if (!this.button) this.button = element("button");
          return this.button;
        }
        const key = selector.replace(/^\./, "");
        if (!this.childrenByClass) this.childrenByClass = {};
        if (!this.childrenByClass[key]) this.childrenByClass[key] = element();
        return this.childrenByClass[key];
      },
    };
  }
  const root = {
    set innerHTML(_) {},
    querySelector(selector) {
      const key = selector.replace(/^\./, "");
      if (!elements.has(key)) elements.set(key, element());
      return elements.get(key);
    },
  };
  const document = { createElement: tag => element(tag) };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" } };
  runInNewContext(source, { window, document, Promise, String, JSON, encodeURIComponent, Object, Error });
  const panel = window.OwoPanels["plugin-market"];
  const calls = [];
  panel.mount(root, {
    root,
    baseUrl: "http://localhost",
    get(path) {
      calls.push(["GET", path]);
      if (getOverride) return getOverride(path);
      if (path.includes("audit")) return Promise.resolve({ entries: [] });
      return Promise.resolve({ app_version: "1.0.0", require_signature: true, plugins: [] });
    },
    post(path, body) {
      calls.push(["POST", path, body]);
      return postOverride ? postOverride(path, body) : Promise.resolve({ report: { id: body.id, version: body.version, state: "installed" }, removed: [] });
    },
    esc(value) { return String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); },
    friendlyError: error => error.message || String(error),
    confirm: options => {
      calls.push(["CONFIRM", options]);
      return confirmOverride ? confirmOverride(options) : Promise.resolve(true);
    },
  });
  return { panel, root, elements, calls };
}

function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

test("market catalog displays the actual minimum app version field", () => {
  const h = harness();
  h.panel.renderCatalog({ app_version: "2.0.0", require_signature: true, plugins: [{
    source: "market", id: "sample", name: "Sample", version: "1.2.0", min_app_version: "1.5.0", description: "A human description",
  }] });
  const row = h.root.querySelector(".owo-market-catalog").children[0];
  assert.match(row.innerHTML, /最低支持 App 1\.5\.0/);
  assert.doesNotMatch(row.innerHTML, /最低支持 App A human description/);
});

test("remote plugin installation is single-flight and restores its button", async () => {
  const pending = deferred();
  const h = harness(null, () => pending.promise);
  const button = { disabled: false, textContent: "安装", attributes: {}, setAttribute(name, value) { this.attributes[name] = value; }, removeAttribute(name) { delete this.attributes[name]; } };
  const first = h.panel.doInstallRemote("sample", "1.0.0", "", button);
  const second = h.panel.doInstallRemote("sample", "1.0.0", "", button);
  await second;
  assert.equal(h.calls.filter(([method, path]) => method === "POST" && path === "/plugins/market/install-remote").length, 1);
  assert.equal(button.disabled, true);
  pending.resolve({ report: { id: "sample", version: "1.0.0", state: "installed" } });
  await first;
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "安装");
  assert.match(h.root.querySelector(".owo-market-result").textContent, /远端安装完成/);
});

test("plugin action lock survives panel remount and clears when the request settles", async () => {
  const pending = deferred();
  const h = harness(null, () => pending.promise);
  const button = { disabled: false, textContent: "安装", attributes: {}, setAttribute(name, value) { this.attributes[name] = value; }, removeAttribute(name) { delete this.attributes[name]; } };
  const first = h.panel.doInstallRemote("sample", "1.0.0", "", button);
  assert.equal(h.calls.filter(([method, path]) => method === "POST" && path === "/plugins/market/install-remote").length, 1);

  h.panel.dispose();
  h.panel.mount(h.root, {
    root: h.root,
    get(path) {
      h.calls.push(["GET", path]);
      if (path.includes("audit")) return Promise.resolve({ entries: [] });
      return Promise.resolve({ app_version: "1.0.0", require_signature: true, plugins: [] });
    },
    post(path, body) {
      h.calls.push(["POST", path, body]);
      return Promise.resolve({ report: { id: body.id, version: body.version, state: "installed" } });
    },
    esc: value => String(value),
    friendlyError: error => error.message || String(error),
  });
  await h.panel.doInstallRemote("sample", "1.0.0", "");
  const postsWhilePending = h.calls.filter(([method, path]) => method === "POST" && path === "/plugins/market/install-remote");
  assert.equal(postsWhilePending.length, 1, "重挂载后不能再次发起同一安装");
  assert.match(h.root.querySelector(".owo-market-result").textContent, /仍在执行/);

  pending.resolve({ report: { id: "sample", version: "1.0.0", state: "installed" } });
  await first;
  await h.panel.doInstallRemote("sample", "1.0.0", "");
  const postsAfterSettlement = h.calls.filter(([method, path]) => method === "POST" && path === "/plugins/market/install-remote");
  assert.equal(postsAfterSettlement.length, 2, "原请求完成后锁必须释放");
});

test("older catalog refresh responses cannot overwrite newer results or repaint after disposal", async () => {
  const requests = [];
  const h = harness(path => {
    if (path.includes("audit")) return Promise.resolve({ entries: [] });
    const request = deferred();
    requests.push(request);
    return request.promise;
  });
  const first = h.panel.refresh();
  const second = h.panel.refresh();
  requests[2].resolve({ app_version: "new", plugins: [] });
  await second;
  requests[1].resolve({ app_version: "old", plugins: [] });
  await first;
  assert.match(h.root.querySelector(".owo-market-env").textContent, /App new/);
  assert.doesNotMatch(h.root.querySelector(".owo-market-env").textContent, /App old/);

  const afterDispose = h.panel.refresh();
  h.panel.dispose();
  requests[3].resolve({ app_version: "detached", plugins: [] });
  await afterDispose;
  assert.match(h.root.querySelector(".owo-market-env").textContent, /App new/);
});


test("plugin uninstall asks for confirmation and cancellation makes no uninstall request", async () => {
  let confirmOptions;
  const cancelled = harness(null, null, async options => { confirmOptions = options; return false; });
  await cancelled.panel.doUninstall("sample");
  assert.equal(confirmOptions.confirmText, "卸载插件");
  assert.equal(cancelled.calls.some(([method, path]) => method === "POST" && path === "/plugins/market/uninstall"), false);
  assert.match(cancelled.root.querySelector(".owo-market-result").textContent, /已取消卸载/);

  const accepted = harness();
  await accepted.panel.doUninstall("sample");
  assert.ok(accepted.calls.some(([method, path]) => method === "POST" && path === "/plugins/market/uninstall"));
});


test("market filters locally and missing signature state is not rendered as disabled", () => {
  const h=harness(); h.root.querySelector(".owo-market-search").value="needle";
  const before=h.calls.length;
  h.panel.renderCatalog({app_version:"1",plugins:[
    {id:"needle",source:"market",name:"Relevant"},
    {id:"other",source:"market",name:"Other"}
  ]});
  const rows=h.root.querySelector(".owo-market-catalog").children;
  assert.equal(rows.length,1); assert.match(rows[0].innerHTML,/Relevant/);
  assert.match(h.root.querySelector(".owo-market-env").textContent,/状态未知/);
  assert.equal(h.calls.length,before);
});
test("malformed market refresh preserves the snapshot and exposes an error", async () => {
  let bad=false;
  const h=harness(path=>Promise.resolve(path.includes("audit")?{entries:[]}:bad?{}:{app_version:"good",plugins:[]}));
  await h.panel.refresh(); bad=true; await h.panel.refresh();
  assert.equal(h.panel.catalogData.app_version,"good");
  assert.match(h.root.querySelector(".owo-market-env").textContent,/读取失败.*保留上次结果/);
});
test("uninstall confirmation cannot send a request after the panel changes", async () => {
  const confirmation=deferred(); const h=harness(null,null,()=>confirmation.promise);
  const pending=h.panel.doUninstall("sample");
  h.panel.dispose(); confirmation.resolve(true); await pending;
  assert.equal(h.calls.filter(([m,p])=>m==="POST"&&p==="/plugins/market/uninstall").length,0);
});
