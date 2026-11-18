import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const PET = join(WEB, "pet");
const index = JSON.parse(readFileSync(join(PET, "assets", "skins", "index.json"), "utf8"));
const squid = index.skins.find((skin) => skin.id === "owo-squid");
const pink = index.skins.find((skin) => skin.id === "pink-squid");

test("3D 墨鱼皮肤以默认形象登记并携带用户 GLB", () => {
  assert.equal(index.default, "owo-squid");
  assert.deepEqual(index.skins.map((skin) => skin.id), ["owo-squid", "pink-squid", "owo-figurine"]);
  const figurine = index.skins[2];
  assert.equal(figurine.kind, "gltf");
  assert.equal(figurine.motion, "figurine");
  assert.equal(figurine.studioLighting, true);
  assert.deepEqual(figurine.contentInsets, { left: 63, right: 63, top: 43, bottom: 12 });
  assert.deepEqual(squid?.contentInsets, { left: 76, right: 74, top: 116, bottom: 54 });
  assert.equal(readFileSync(join(PET, "assets", "skins", figurine.id, figurine.model))
    .subarray(0, 4).toString("ascii"), "glTF");
  assert.equal(squid?.kind, "gltf");
  assert.equal(squid?.model, "Squid.glb");
  const model = join(PET, "assets", "skins", "owo-squid", squid.model);
  assert.ok(statSync(model).size > 1024 * 1024, "GLB 不能是空占位文件");
  assert.equal(readFileSync(model).subarray(0, 4).toString("ascii"), "glTF");
  assert.equal(pink?.kind, "gltf");
  const pinkModel = join(PET, "assets", "skins", "pink-squid", pink.model);
  assert.ok(statSync(pinkModel).size > 1024 * 1024, "粉色 GLB 不能是空占位文件");
  assert.equal(readFileSync(pinkModel).subarray(0, 4).toString("ascii"), "glTF");
});

test("桌宠页面加载 3D canvas 与 GLB 渲染器，同时保留精灵图入口", () => {
  const html = readFileSync(join(PET, "index.html"), "utf8");
  const js = readFileSync(join(PET, "pet.js"), "utf8");
  const renderer = readFileSync(join(PET, "model-renderer.js"), "utf8");
  assert.match(html, /pet-avatar-3d/);
  assert.match(html, /model-renderer\.js/);
  assert.match(js, /kind === "gltf"/);
  assert.match(js, /setModel\(config\.model\.url/);
  assert.match(renderer, /0x46546c67/);
  assert.match(renderer, /getContext\("webgl2"/);
  assert.match(renderer, /colors/);
  assert.match(renderer, /TEXCOORD_0/);
  assert.match(renderer, /baseColorTexture/);
  assert.match(renderer, /texImage2D/);
  assert.match(renderer, /rotate\(dx,dy\)/);
  assert.match(js, /modelRenderer\.rotate/);
  assert.match(js, /按住 Shift 拖动/);
  assert.match(js, /if \(skinIndex\.skins\.length\) \{/);
  assert.doesNotMatch(js, /skinIndex\.skins\.length && !globalThis\.OwoFeedUI/);
  assert.match(js, /menuItem\("打开工作台"/);
  assert.match(js, /menuItem\("隐藏桌宠"/);
  assert.match(js, /label: "更换形象"/);
  assert.match(js, /submenu: skinIndex\.skins\.map/);
  assert.match(js, /shellBridge\.showMenu\(items\)/);
  assert.doesNotMatch(js, /喂入剪贴板需求/);
});
