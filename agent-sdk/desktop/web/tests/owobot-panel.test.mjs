import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/owobot.panel.js", import.meta.url), "utf8");

async function mountWith(connection) {
  const window = {};
  const document = {
    createElement(type) {
      return {
        type,
        setAttribute() {},
        addEventListener() {},
        className: "",
        isConnected: true,
      };
    },
  };
  let request;
  runInNewContext(source, {
    window,
    document,
    URL,
    location: { origin: "http://127.0.0.1:43123" },
  });
  const host = {
    isConnected: true,
    replaceChildren(...children) { this.children = children; },
    appendChild(child) { this.children = [...(this.children || []), child]; },
  };
  await window.OwoPanels.owobot.mount(host, {
    selectedModel: () => "deepseek-flash",
    selectedModelConnection: () => connection,
    post: async (_path, body) => {
      request = body;
      return { url: "/owobot/", groupId: "group-1" };
    },
  });
  return request;
}

test("旧自定义模型记录没有安全凭据引用时回退到桌面默认连接", async () => {
  const request = await mountWith({ model: "deepseek-flash", base_url: "https://api.deepseek.com/", api_format: "openai" });
  assert.equal(request.model, "deepseek-flash");
  assert.equal(request.connection, null);
});

test("有安全凭据引用的自定义模型仍传递连接", async () => {
  const request = await mountWith({
    model: "deepseek-flash",
    base_url: "https://api.deepseek.com/",
    api_format: "openai",
    api_key_env: "OWO_CUSTOM_MODEL_SECRET_0123456789abcdef0123456789abcdef",
  });
  assert.equal(request.connection.api_key_env, "OWO_CUSTOM_MODEL_SECRET_0123456789abcdef0123456789abcdef");
});

async function mountWithRefusal(connection) {
  const window = {};
  const document = {
    createElement(type) {
      return {
        type,
        setAttribute() {},
        addEventListener() {},
        append() {},
        remove() {},
        className: "",
        textContent: "",
        isConnected: true,
      };
    },
  };
  const requests = [];
  runInNewContext(source, {
    window,
    document,
    URL,
    location: { origin: "http://127.0.0.1:43123" },
  });
  const host = {
    isConnected: true,
    replaceChildren(...children) { this.children = children; },
    appendChild(child) { this.children = [...(this.children || []), child]; },
  };
  await window.OwoPanels.owobot.mount(host, {
    selectedModel: () => "cn:deepseek-v4.1-flash",
    selectedModelConnection: () => connection,
    post: async (_path, body) => {
      requests.push(body);
      if (requests.length === 1) throw new Error("OwOBot 模型连接不可用，请检查桌面端模型设置");
      return { url: "/owobot/", groupId: "group-1" };
    },
  });
  return { requests, host };
}

test("模型连接被核心拒绝时退回核心默认模型打开，并给出恢复提示", async () => {
  const connection = {
    model: "cn:deepseek-v4.1-flash",
    base_url: "https://tokenfuture.si/v1",
    api_format: "openai",
    api_key_env: "OWO_CUSTOM_MODEL_SECRET_0123456789abcdef0123456789abcdef",
  };
  const { requests, host } = await mountWithRefusal(connection);
  assert.equal(requests.length, 2, "先带模型连接请求，被拒后重试一次");
  assert.equal(requests[0].connection.api_key_env, connection.api_key_env);
  assert.equal(requests[1].connection, undefined, "回退请求不再带失效的连接");
  assert.equal(requests[1].model, undefined, "回退请求也不带只对自定义连接有效的模型名");
  assert.equal(host.children.length, 2, "提示条 + 工作台 iframe");
  assert.equal(host.children[0].className, "owobot-panel-note");
  assert.equal(host.children[1].className, "owobot-panel-frame");
});

test("非模型连接的失败不重试，直接报错", async () => {
  const window = {};
  const document = {
    createElement(type) {
      return { type, setAttribute() {}, addEventListener() {}, append() {}, remove() {}, className: "", textContent: "", isConnected: true };
    },
  };
  let attempts = 0;
  runInNewContext(source, { window, document, URL, location: { origin: "http://127.0.0.1:43123" } });
  const host = { isConnected: true, replaceChildren(...children) { this.children = children; }, appendChild() {} };
  await window.OwoPanels.owobot.mount(host, {
    selectedModel: () => "deepseek-flash",
    selectedModelConnection: () => null,
    post: async () => { attempts += 1; throw new Error("OwOBot 需要一个存在的项目目录"); },
  });
  assert.equal(attempts, 1, "项目目录类错误不该被当成模型连接问题重试");
  assert.equal(host.children[0].className, "owobot-panel-error");
});
