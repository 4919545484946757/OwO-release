import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, "..");
const app = readFileSync(join(web, "app.js"), "utf8").replace(/\r\n/g, "\n");
const html = readFileSync(join(web, "index.html"), "utf8");
const presets = readFileSync(join(web, "config", "provider-presets.js"), "utf8");

test("自定义模型对话框挂有服务商预设下拉", () => {
  assert.match(app, /id="cmPreset"/, "缺少预设下拉");
  assert.match(app, /async function fillPresetOptions\(\)/, "缺少填充逻辑");
  assert.match(app, /api\("\/providers\/presets"\)/, "预设必须来自受保护的同源接口");
  assert.match(app, /OwoModelCatalog\?\.presetToForm/, "选中预设应复用 presetToForm");
  assert.match(app, /fillPresetOptions\(\);\n\}/, "对话框打开后要触发填充");
});

test("预设拉取失败时退回手动填写而不是报错阻塞", () => {
  assert.match(app, /catch \{\s*\n\s*\/\/ 预设拉取失败不阻塞手动填写/, "缺少降级分支");
  assert.match(app, /select\.style\.display = "none"/, "失败时应隐藏下拉");
});

test("前端预设注册表不含任何密钥", () => {
  assert.equal(/sk-[A-Za-z0-9]{8,}/.test(presets), false, "预设表出现疑似密钥");
  assert.equal(/\bapiKey\s*[:=]\s*["'][^"']+["']/.test(presets), false, "预设表出现明文 apiKey");
});

test("模型相关脚本已注册且顺序在 app.js 之前", () => {
  for (const src of ["core/model-catalog.js", "core/model-picker.js", "core/turn-model.js", "core/model-bar.js"]) {
    assert.ok(html.includes(`<script src="${src}"></script>`), `缺少脚本 ${src}`);
  }
  const catalogAt = html.indexOf('<script src="core/model-catalog.js"></script>');
  const appAt = html.indexOf('<script src="app.js"></script>');
  assert.ok(catalogAt > 0 && appAt > 0, "脚本标签未找到");
  assert.ok(catalogAt < appAt, "model-catalog 必须先于 app.js 加载");
});
