import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", "..");
const jsPath = join(here, "..", "config", "provider-presets.js");
const jsSource = readFileSync(jsPath, "utf8");
const rsSource = readFileSync(
  join(root, "crates", "owo-agent-core", "src", "gateway", "presets.rs"),
  "utf8"
);

function loadFrontend() {
  const sandbox = { window: {} };
  vm.runInNewContext(jsSource, sandbox);
  return sandbox.window.OwoProviderPresets.presets();
}

function idsOf(source) {
  return new Set([...source.matchAll(/id:\s*"([a-z0-9_]+)"/g)].map((match) => match[1]));
}

function backendBlock(id) {
  const source = rsSource.split("#[cfg(test)]")[0];
  const start = source.indexOf(`id: "${id}"`);
  assert.ok(start > 0, `未找到预设 ${id}`);
  const next = source.indexOf('id: "', start + 6);
  return source.slice(start, next > 0 ? next : source.length);
}

function rustList(block, field) {
  const match = block.match(new RegExp(`${field}:\\s*&\\[([^\\]]*)\\]`));
  if (!match) return [];
  return [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]);
}

test("前端预设与后端预设的服务商集合一致", () => {
  const frontend = idsOf(jsSource);
  const backend = idsOf(rsSource.split("#[cfg(test)]")[0]);
  const missing = [...backend].filter((id) => !frontend.has(id));
  const extra = [...frontend].filter((id) => !backend.has(id));
  assert.deepEqual(missing, [], `前端缺少服务商：${missing.join(", ")}`);
  assert.deepEqual(extra, [], `前端多出服务商：${extra.join(", ")}`);
  assert.ok(backend.has("custom") && backend.has("vllm"), "自建入口必须保留");
});

test("自定义端点必须允许空地址与空模型", () => {
  assert.match(backendBlock("custom"), /base_url:\s*""/, "custom 预设不应写死地址");
});

test("文生图 / 语音模型清单与后端一致", () => {
  const presets = new Map(loadFrontend().map((item) => [item.id, item]));
  for (const id of idsOf(rsSource.split("#[cfg(test)]")[0])) {
    const block = backendBlock(id);
    const frontend = presets.get(id);
    assert.ok(frontend, `前端缺少预设 ${id}`);
    assert.deepEqual(
      [...(frontend.imageModels || [])].sort(),
      rustList(block, "image_models").sort(),
      `${id} 的文生图模型清单不一致`
    );
    assert.deepEqual(
      [...(frontend.speechModels || [])].sort(),
      rustList(block, "speech_models").sort(),
      `${id} 的语音模型清单不一致`
    );
  }
});
