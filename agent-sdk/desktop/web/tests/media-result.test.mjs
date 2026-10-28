import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";
import {webcrypto, createHash} from "node:crypto";

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, "..");
const source = readFileSync(join(web, "core", "media-result.js"), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(source, sandbox);
const media = sandbox.window.OwoMediaResult;

test("文生图结果解析出图片路径", () => {
  const result = media.mediaFromToolResult({
    tool: "generate_image",
    ok: true,
    preview: '{"path":"outputs/hero.png","bytes":20480,"model":"gpt-image-1"}',
  });
  // 跨 realm 对象（vm 沙箱）不能直接 deepEqual 比较原型，逐字段断言。
  assert.equal(result.kind, "image");
  assert.equal(result.tool, "generate_image");
  assert.equal(result.path, "outputs/hero.png");
  assert.equal(result.bytes, 20480);
  assert.equal(media.describeMedia(result), "已生成图片（20 KB）：outputs/hero.png");
});

test("语音结果解析出音频路径与大小", () => {
  const result = media.mediaFromToolResult({
    tool: "text_to_speech",
    ok: true,
    preview: '语音合成完成 {"path":"outputs/voice.mp3","bytes":1048576}',
  });
  assert.equal(result.kind, "audio");
  assert.equal(media.describeMedia(result), "已生成语音（1.0 MB）：outputs/voice.mp3");
});

test("失败结果、非媒体工具与坏 JSON 都返回 null", () => {
  assert.equal(media.mediaFromToolResult({ tool: "generate_image", ok: false, preview: '{"path":"outputs/a.png"}' }), null);
  assert.equal(media.mediaFromToolResult({ tool: "read_file", ok: true, preview: '{"path":"a.txt"}' }), null);
  assert.equal(media.mediaFromToolResult({ tool: "generate_image", ok: true, preview: "not json" }), null);
  assert.equal(media.mediaFromToolResult({ tool: "generate_image", ok: true, preview: '{"path":""}' }), null);
  assert.equal(media.mediaFromToolResult(null), null);
});

test("tool_result 分支已接入 noteMediaOutput", () => {
  const domain = readFileSync(join(web, "app-domain.js"), "utf8");
  assert.match(domain, /case "tool_result":[\s\S]{0,120}noteMediaOutput\(payload\)/, "未接入媒体消息");
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /function noteMediaOutput\(payload\) \{/, "缺少 noteMediaOutput 定义");
});

test("小图 data_url 透出，非 image 前缀与超长内容丢弃", () => {
  const ok = media.mediaFromToolResult({
    tool: "generate_image",
    ok: true,
    preview: '{"path":"outputs/a.png","bytes":8,"data_url":"data:image/png;base64,iVBORw0KGgo="}',
  });
  assert.equal(ok.dataUrl, "data:image/png;base64,iVBORw0KGgo=");

  const audio = media.mediaFromToolResult({
    tool: "generate_image",
    ok: true,
    preview: '{"path":"outputs/a.png","bytes":8,"data_url":"data:audio/mp3;base64,AAA"}',
  });
  assert.equal(audio.dataUrl, "", "非 image/* 的 data URL 不得当作图片预览");

  const missing = media.mediaFromToolResult({
    tool: "generate_image",
    ok: true,
    preview: '{"path":"outputs/big.png","bytes":900000}',
  });
  assert.equal(missing.dataUrl, "", "大图不带内联数据");
});

test("应用层只对 dataUrl 存在时插入 img，且限制尺寸", () => {
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /if \(media\.dataUrl && bubble\)/, "缺少内联预览守卫");
  assert.match(app, /className = "owo-media-inline"/, "缺少缩略图类名");
  const css = readFileSync(join(web, "experience.css"), "utf8");
  assert.match(css, /\.owo-media-inline\s*\{/, "缺少缩略图样式");
  assert.match(css, /max-width:\s*min\(420px, 100%\)/, "缩略图必须有宽度上限");
});

test("音频 data_url 只对 text_to_speech 生效，并按 kind 校验前缀", () => {
  const audioInlined = media.mediaFromToolResult({
    tool: "text_to_speech",
    ok: true,
    preview: '{"path":"outputs/v.mp3","bytes":12,"data_url":"data:audio/mpeg;base64,SUQz"}',
  });
  assert.equal(audioInlined.kind, "audio");
  assert.equal(audioInlined.dataUrl, "data:audio/mpeg;base64,SUQz");

  const wrongType = media.mediaFromToolResult({
    tool: "text_to_speech",
    ok: true,
    preview: '{"path":"outputs/v.mp3","bytes":12,"data_url":"data:image/png;base64,AAA"}',
  });
  assert.equal(wrongType.dataUrl, "", "图像 data URL 不能塞进音频播放器");
});

test("应用层为音频插入 audio 控件而不是 img", () => {
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /media\.kind === "audio"/, "缺少音频分支");
  assert.match(app, /node\.controls = true/, "audio 必须有 controls");
  assert.match(app, /owo-media-audio/, "缺少音频类名");
  const css = readFileSync(join(web, "experience.css"), "utf8");
  assert.match(css, /\.owo-media-audio\s*\{/, "缺少音频样式");
});

test("模型菜单展示媒体能力，且未配置时给出明确提示", () => {
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /function mediaHintHtml\(\)/, "缺少能力提示函数");
  assert.match(app, /mediaConnectionInfo\("image"\)/, "图片连接状态读取错误");
  assert.match(app, /mediaConnectionInfo\("audio"\)/, "语音连接状态读取错误");
  assert.match(app, /typeof mediaHintHtml === "function" \? mediaHintHtml\(\)/, "菜单未挂载提示");
  assert.match(app, /OPENAI_API_KEY/, "提示应告诉用户配哪个变量");

  const domain = readFileSync(join(web, "app-domain.js"), "utf8");
  assert.match(domain, /state\.capabilities = capabilities;/, "能力清单未缓存");
});

test("输入栏媒体入口：插入模板而不是直接发请求", () => {
  const html = readFileSync(join(web, "index.html"), "utf8");
  assert.match(html, /id="mediaBtn"/, "缺少媒体按钮");
  assert.match(html, /aria-haspopup="menu"/, "媒体按钮应声明弹出菜单");

  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /function openMediaMenu\(\)/, "缺少菜单函数");
  assert.match(app, /const MEDIA_TEMPLATES = \{/, "缺少指令模板表");
  assert.match(app, /prompt\.value = MEDIA_TEMPLATES\[kind\]/, "点击后应只插入模板");
  assert.match(app, /closeComposerMenu\(\);/, "插入后应关闭菜单");
  assert.match(app, /\$\("mediaBtn"\)\.addEventListener\("click", \(\) => openMediaMenu\(\)\)/, "按钮未绑定");
  // 关键约束：不直接调用 API，仍由模型发起工具调用并走审批。
  assert.equal(/openMediaMenu[\s\S]{0,1200}OwoApi\.stream/.test(app), false, "媒体入口不得绕过回合直接发请求");
  // 未配置时给出提示，而不是禁用入口。
  const capability = readFileSync(join(web, "core/media-capability.js"), "utf8");
  assert.match(capability, /未配置/, "缺少未配置提示");
  assert.match(capability, /请配置该能力/, "缺少按能力配置的提示");
});

test("媒体菜单的键盘可达性与收起行为", () => {
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /const first = menu\.querySelector\("\[data-media-kind\]"\)/, "打开后未聚焦首项");
  assert.match(app, /\$\("mediaBtn"\)\.setAttribute\("aria-expanded", "true"\)/, "未声明展开态");
  assert.match(app, /\$\("mediaBtn"\)\?\.setAttribute\("aria-expanded", "false"\)/, "关闭时未复位展开态");
  assert.match(app, /#emptyStateWorkspaceBtn,#mediaBtn/, "外部点击未把 mediaBtn 视为触发器");
});

test("会话重载回放也会还原媒体产物", () => {
  const domain = readFileSync(join(web, "app-domain.js"), "utf8");
  // 实时事件与历史回放两条路径都必须调用 noteMediaOutput，否则刷新后图片/语音消失。
  const calls = domain.match(/noteMediaOutput\(/g) || [];
  assert.equal(calls.length, 2, "实时与回放各应调用一次 noteMediaOutput");
  assert.match(domain, /if \(typeof noteMediaOutput === "function"\) noteMediaOutput\(value\);/, "回放分支缺少媒体还原");
  assert.match(domain, /noteMediaOutput\(payload\);/, "实时分支缺少媒体还原");
});

test("大文件媒体不内联时给出可复制的路径", () => {
  const big = media.mediaFromToolResult({ ok: true, tool: "generate_image", preview: JSON.stringify({ path: "outputs/huge.png", bytes: 4096 * 1024 }) });
  assert.equal(big.dataUrl, "", "无 data_url 时不应伪造内联地址");

  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /文件较大，未内联预览/, "缺少未内联说明");
  assert.match(app, /copy\.addEventListener\("click", async \(\) => \{/, "缺少复制按钮逻辑");
  assert.match(app, /await copyText\(media\.path\)/, "复制内容应为产物路径");
  assert.match(app, /else if \(bubble\) \{/, "兜底分支应只在有气泡时渲染");

  const css = readFileSync(join(web, "experience.css"), "utf8");
  assert.match(css, /\.owo-media-file \{/, "缺少路径行样式");
  assert.match(css, /\.owo-media-copy \{/, "缺少复制按钮样式");
});


function output(kind, bytes, id = "media-fixture") {
  return { artifact_id: id, kind, mime_type: kind === "image" ? "image/png" : "audio/mpeg", bytes, path: `outputs/${id}` };
}

test("typed media survives a truncated preview at every acceptance size", () => {
  for (const kind of ["image", "audio"]) for (const size of [2048, 100 * 1024, 1024 * 1024]) {
    const payload = { ok: true, tool: kind === "image" ? "generate_image" : "text_to_speech", preview: '{"data_url":"truncated', outputs: [output(kind, size)] };
    // Historical replay serializes the exact same event contract.
    for (const event of [payload, JSON.parse(JSON.stringify(payload))]) {
      const result = media.mediaFromToolResult(event);
      assert.equal(result.bytes, size);
      assert.equal(result.contentPath, "/artifacts/media-fixture/content?raw=true");
      assert.equal(result.dataUrl, "");
    }
  }
});

test("typed outputs reject invalid MIME, ids and sizes without parsing a legacy fallback", () => {
  for (const item of [
    { ...output("image", 1), mime_type: "image/svg+xml" },
    { ...output("image", 1), artifact_id: "../../secret" },
    { ...output("image", 1), bytes: 33 * 1024 * 1024 },
    { ...output("image", 1), bytes: -1 },
    output("audio", 1),
  ]) assert.equal(media.mediaFromToolResult({ ok: true, tool: "generate_image", outputs: [item], preview: '{"path":"legacy.png"}' }), null);
});

function rendererHarness(api, expectedSha256) {
  const urls = [], revoked = [];
  let cleanup;
  class Node {
    constructor(tag) { this.tagName = tag; this.children = []; this.attrs = {}; this.listeners = {}; this.isConnected = true; this.ownerDocument = doc; }
    append(...nodes) { for (const n of nodes) this.appendChild(n); }
    appendChild(n) { n.parent = this; this.children.push(n); return n; }
    setAttribute(k, v) { this.attrs[k] = v; }
    getAttribute(k) { return this.attrs[k] || null; }
    replaceWith(node) { this.parent.current = node; node.isConnected = true; node.parent = this.parent; }
    removeAttribute(k) { delete this[k]; delete this.attrs[k]; }
    remove() { this.isConnected = false; if (this.parent) this.parent.children = this.parent.children.filter(x => x !== this); }
    addEventListener(k, v) { this.listeners[k] = v; }
    removeEventListener(k) { delete this.listeners[k]; }
  }
  const doc = { createElement: tag => new Node(tag), documentElement: {} };
  const root = {
    AbortController, OwoApi: api, crypto: webcrypto,
    URL: { createObjectURL: blob => { const url = "blob:" + urls.length; urls.push({ url, blob }); return url; }, revokeObjectURL: url => revoked.push(url) },
    MutationObserver: class { constructor(fn) { cleanup = fn; } observe() {} disconnect() {} },
  };
  vm.runInNewContext(source, { window: root });
  const container = new Node("div");
  const item = root.OwoMediaResult.mediaFromToolResult({ tool: "generate_image", ok: true, outputs: [output("image", 2048)] });
  if (expectedSha256) item.expectedSha256 = expectedSha256;
  const handle = root.OwoMediaResult.mount(container, item);
  return { root, container, handle, urls, revoked, cleanup: () => cleanup() };
}

test("authenticated renderer bounds downloads, creates a blob preview and releases it on removal", async () => {
  const calls = [];
  const h = rendererHarness({ download: async (path, opts) => { calls.push({ path, opts }); return new Blob([new Uint8Array(2048)], { type: "image/png" }); } });
  assert.equal(calls.length, 0, "media loads on demand");
  await h.handle.load();
  assert.equal(calls[0].path, "/artifacts/media-fixture/content?raw=true");
  assert.equal(calls[0].opts.maxResponseBytes, 32 * 1024 * 1024);
  assert.ok(calls[0].opts.signal);
  assert.equal(h.container.children[2].src, "blob:0");
  h.container.isConnected = false;
  h.cleanup();
  assert.deepEqual(h.revoked, ["blob:0"]);
});

test("media loading can be cancelled and a late response never creates a URL", async () => {
  let finish, signal;
  const h = rendererHarness({ download: (_, opts) => { signal = opts.signal; return new Promise(resolve => { finish = resolve; }); } });
  const pending = h.handle.load();
  h.handle.cancel();
  assert.equal(signal.aborted, true);
  finish(new Blob([new Uint8Array(2048)], { type: "image/png" }));
  await pending;
  assert.equal(h.urls.length, 0);
  h.handle.dispose();
});

test("missing files, denied access and mismatched MIME/size remain retryable", async () => {
  for (const failure of [404, 403, "mime", "size"]) {
    const h = rendererHarness({ download: async () => {
      if (typeof failure === "number") throw Object.assign(new Error("denied"), { status: failure });
      return new Blob([new Uint8Array(failure === "size" ? 1 : 2048)], { type: failure === "mime" ? "text/html" : "image/png" });
    } });
    await h.handle.load();
    assert.equal(h.urls.length, 0);
    assert.match(h.container.children[1].textContent, /不存在|权限|不符/);
    assert.equal(h.container.children[0].textContent, "重试加载");
    h.handle.dispose();
  }
});


test("Team updates preserve an already loaded player instead of replacing its URL", async () => {
  let requests = 0;
  const h = rendererHarness({ download: async () => { requests++; return new Blob([new Uint8Array(2048)], { type: "image/png" }); } });
  await h.handle.load();
  const key = JSON.stringify({ ok: true, tool: "generate_image", outputs: [output("image", 2048)] });
  h.container.setAttribute("data-media-artifact", key);
  const outer = {
    current: h.container,
    querySelectorAll() { return [this.current]; },
    set innerHTML(value) {
      this.current.isConnected = false;
      this.current = new h.container.constructor("div");
      this.current.setAttribute("data-media-artifact", key);
      this.current.parent = this;
    },
  };
  h.container.parent = outer;
  h.root.OwoMediaResult.renderInto(outer, "new conversation message");
  assert.equal(outer.current, h.container);
  assert.equal(outer.current.children[2].src, "blob:0");
  assert.equal(requests, 1);
  h.cleanup();
  assert.deepEqual(h.revoked, []);
  h.handle.dispose();
});

test("immutable input images verify SHA-256 before creating a preview URL", async () => {
 const bytes=new Uint8Array(2048),hash=createHash("sha256").update(bytes).digest("hex");
 for(const expected of [hash,"b".repeat(64)]){
  const h=rendererHarness({download:async()=>new Blob([bytes],{type:"image/png"})},expected);
  await h.handle.load();
  assert.equal(h.urls.length,expected===hash?1:0);
  if(expected!==hash)assert.match(h.container.children[1].textContent,/摘要不符/);
  h.handle.dispose();
 }
 const item=media.mediaFromToolResult({tool:"generate_image",ok:true,outputs:[{...output("image",2048),sha256:hash}]});
 assert.equal(item.expectedSha256,hash);
 assert.equal(media.mediaFromToolResult({tool:"generate_image",outputs:[{...output("image",2048),sha256:"bad"}]}),null);
});
test("cancellation while verifying a digest never creates a late media URL", async () => {
 let finish;const h=rendererHarness({download:async()=>new Blob([new Uint8Array(2048)],{type:"image/png"})},"a".repeat(64));
 h.root.crypto={subtle:{digest:()=>new Promise(r=>finish=r)}};
 const loading=h.handle.load();await new Promise(r=>setImmediate(r));assert(finish);
 h.handle.cancel();finish(new ArrayBuffer(32));await loading;assert.equal(h.urls.length,0);h.handle.dispose();
});

test("disposing a visible inspector immediately releases its previews but preserves other views",async()=>{
 const a=rendererHarness({download:async()=>new Blob([new Uint8Array(2048)],{type:"image/png"})});
 await a.handle.load();a.root.OwoMediaResult.disposeWithin({contains:node=>node===a.container});
 assert.deepEqual(a.revoked,["blob:0"]);assert.equal(a.container.children.some(n=>n.src),false);
 await a.handle.load();assert.equal(a.urls.length,1);
 const b=rendererHarness({download:async()=>new Blob([new Uint8Array(2048)],{type:"image/png"})});
 await b.handle.load();b.root.OwoMediaResult.disposeWithin({contains:()=>false});
 assert.deepEqual(b.revoked,[]);b.handle.dispose();
});
