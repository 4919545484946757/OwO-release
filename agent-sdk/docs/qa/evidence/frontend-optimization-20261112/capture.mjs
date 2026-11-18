// Isolated Electron/CDP screenshot harness for the 2026-11-12 frontend visual QA.
// It never seeds models, sessions, teams or other business data. It only fixes
// theme/viewport preferences and, when missing, adds one real loopback model
// connection through the app's own desktop model bridge (no key is stored).
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const REPO = 'D:\\OwO\\OwO-agent-dev';
const WEB = path.join(REPO, 'agent-sdk', 'desktop', 'web');
const APP = path.join(REPO, 'agent-sdk', 'desktop', 'electron');
const ELECTRON = path.join(APP, 'node_modules', 'electron', 'dist', 'electron.exe');
const CORE = path.join(REPO, 'agent-sdk', 'target', 'debug', 'owo-agent.exe');
const QA_DIR = path.join(REPO, '.run', 'frontend-visual-qa-20261112');
const EVIDENCE = path.join(REPO, 'agent-sdk', 'docs', 'qa', 'evidence', 'frontend-optimization-20261112');
const CONFIG = path.join(QA_DIR, 'config.json');
const DATA_ROOT = path.join(QA_DIR, 'desktop-data');
const AGENT_DATA = path.join(QA_DIR, 'agent-data');
const WORKSPACE = path.join(QA_DIR, 'workspace');
const CHROME_PROFILE = path.join(QA_DIR, 'chrome-profile');
const PORT_FILE = path.join(CHROME_PROFILE, 'DevToolsActivePort');
const RUNTIME = path.join(QA_DIR, 'runtime.json');
const LOG = path.join(QA_DIR, 'electron.log');
const LOOPBACK = 'http://127.0.0.1:3425/v1';
const MODEL = 'auto/GLM-5.3-Flash';
const PLACEHOLDER = 'loopback-only-placeholder';
const PANELS = ['home','evidence','notes','automations','plugin-market','workflow','custom-workflows','goal','team','eval','product-eval','observability','diagnostics-ledger','memory','command','fleet','capabilities','agent-tasks','permissions','action-center','project-history','project-launcher','workswarm','about'];
const SETTINGS = ['general','models','appearance','voice','ime','personalization','notifications','shortcuts','usage','import','account'];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const scrub = value => String(value ?? '').split(PLACEHOLDER).join('***').slice(0, 600);

function ensureDirs() {
  for (const dir of [QA_DIR, DATA_ROOT, AGENT_DATA, WORKSPACE, CHROME_PROFILE, EVIDENCE, path.join(EVIDENCE, 'before', 'shots'), path.join(EVIDENCE, 'after', 'shots')]) fs.mkdirSync(dir, { recursive: true });
}
function writeConfig() {
  const config = {
    version: 1,
    model: {
      provider: 'bigmodel', deferred: false, base_url: LOOPBACK, name: MODEL,
      api_key_env: 'OPENAI_API_KEY', context_window: null, max_output_tokens: 32000,
      model_output_tokens: {}, temperature: null, timeout_secs: null, keep_recent: null,
      compaction: null, models: [],
    },
  };
  fs.writeFileSync(CONFIG, JSON.stringify(config, null, 2) + '\n', 'utf8');
  fs.writeFileSync(path.join(DATA_ROOT, 'workspace.json'), JSON.stringify({ path: WORKSPACE }) + '\n', 'utf8');
}
function readPort() {
  return Number(fs.readFileSync(PORT_FILE, 'utf8').trim().split(/\r?\n/)[0]);
}
async function targets(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`);
  if (!response.ok) throw new Error(`CDP list HTTP ${response.status}`);
  return await response.json();
}
class Cdp {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); this.events = []; }
  static async connect(url) {
    const ws = new WebSocket(url);
    const cdp = new Cdp(ws);
    await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
    ws.onmessage = event => {
      const msg = JSON.parse(event.data);
      if (msg.id && cdp.pending.has(msg.id)) {
        const { resolve, reject } = cdp.pending.get(msg.id);
        cdp.pending.delete(msg.id);
        if (msg.error) reject(new Error(JSON.stringify(msg.error))); else resolve(msg.result);
      } else if (msg.method) {
        if (cdp.events.length < 6000) cdp.events.push(msg);
      }
    };
    return cdp;
  }
  send(method, params = {}) {
    const id = ++this.id;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
      setTimeout(() => {
        if (this.pending.has(id)) { this.pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }
      }, 30000);
    });
  }
  async evaluate(expression) {
    const result = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, userGesture: true });
    if (result.exceptionDetails) return { __error: result.exceptionDetails.text, details: scrub(result.exceptionDetails.exception?.description || '') };
    return result.result?.value;
  }
  close() { try { this.ws.close(); } catch {} }
}
async function connectPage() {
  let last;
  for (let i = 0; i < 60; i++) {
    try {
      const port = readPort();
      const list = await targets(port);
      const httpPages = list.filter(t => t.type === 'page' && t.url.startsWith('http://127.0.0.1:'));
      const page = httpPages.find(t => new URL(t.url).pathname === '/') ?? httpPages.find(t => !new URL(t.url).pathname.startsWith('/pet/')) ?? httpPages[0]
        ?? list.find(t => t.type === 'page' && !t.url.startsWith('devtools://'));
      if (page) {
        const cdp = await Cdp.connect(page.webSocketDebuggerUrl);
        return { cdp, port, page };
      }
      last = `No workbench page: ${JSON.stringify(list.map(t => ({ type: t.type, url: t.url })))}`;
    } catch (error) { last = error.message; }
    await sleep(1000);
  }
  throw new Error(last || 'CDP page did not appear');
}
async function launch() {
  ensureDirs(); writeConfig();
  if (fs.existsSync(PORT_FILE)) fs.rmSync(PORT_FILE, { force: true });
  const env = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (!key.startsWith('OWO_CUSTOM_MODEL_SECRET_') && !key.startsWith('OWO_CUSTOM_MODEL_BINDING_')) env[key] = value;
  }
  Object.assign(env, {
    OWO_DESKTOP_DATA_ROOT: DATA_ROOT,
    OWO_CONFIG_FILE: CONFIG,
    OWO_AGENT_DATA: AGENT_DATA,
    OWO_CORE_EXE: CORE,
    OWO_WEB_UI_DIR: WEB,
    OWO_CLOUD_ENABLED: 'false',
    OPENAI_BASE_URL: LOOPBACK,
    OPENAI_MODEL: MODEL,
    OPENAI_API_KEY: PLACEHOLDER,
  });
  const log = fs.openSync(LOG, 'a');
  const child = spawn(ELECTRON, [APP, '--remote-debugging-port=0', `--user-data-dir=${CHROME_PROFILE}`, '--disable-gpu', '--force-device-scale-factor=1'], {
    env, cwd: APP, detached: true, stdio: ['ignore', log, log], windowsHide: false,
  });
  child.unref(); fs.closeSync(log);
  for (let i = 0; i < 60; i++) {
    try {
      const port = readPort();
      const list = await targets(port);
      if (list.some(t => t.type === 'page')) {
        const runtime = { pid: child.pid, port, launchedAt: new Date().toISOString(), coreSha256: sha(CORE), webDir: WEB };
        fs.writeFileSync(RUNTIME, JSON.stringify(runtime, null, 2) + '\n', 'utf8');
        return { ...runtime, reused: false };
      }
    } catch {}
    await sleep(1000);
  }
  throw new Error('Electron CDP target did not appear');
}
async function ensureRuntime() {
  if (fs.existsSync(RUNTIME)) {
    try {
      const runtime = JSON.parse(fs.readFileSync(RUNTIME, 'utf8'));
      process.kill(runtime.pid, 0);
      const port = readPort();
      const list = await targets(port);
      if (list.some(t => t.type === 'page' && t.url.startsWith('http://127.0.0.1:'))) return { ...runtime, port, reused: true };
    } catch {}
  }
  return await launch();
}
async function waitForApp(cdp) {
  let status;
  for (let i = 0; i < 60; i++) {
    status = await cdp.evaluate(`(async () => {
      let health = {};
      try { health = await fetch('/health').then(r => r.json()); } catch (error) { health = { error: error.message }; }
      return {
        ready: document.readyState === 'complete' && Boolean(document.querySelector('#panelRoot')) && Object.keys(window.OwoPanels || {}).length > 10,
        location: location.href,
        readyState: document.readyState,
        panelCount: Object.keys(window.OwoPanels || {}).length,
        healthStatus: health.status || health.state || null,
        healthError: health.error || null,
        modelChip: document.querySelector('#modelChip') ? { disabled: document.querySelector('#modelChip').disabled, text: document.querySelector('#modelChipText')?.textContent || '' } : null,
      };
    })()`);
    if (status?.ready && !status.healthError) return status;
    await sleep(1000);
  }
  return status;
}
async function enableDomains(cdp) {
  for (const method of ['Runtime.enable', 'Log.enable', 'Network.enable', 'Page.enable']) await cdp.send(method).catch(() => {});
}
async function setViewport(cdp, width, height, dpr = 1) {
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width, height, deviceScaleFactor: dpr, mobile: false,
    screenWidth: width, screenHeight: height,
  });
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: false }).catch(() => {});
  await sleep(180);
}
async function setTheme(cdp, theme) {
  await cdp.evaluate(`(() => {
    let prefs = {};
    try { prefs = JSON.parse(localStorage.getItem('owo.prefs') || '{}'); } catch {}
    prefs.theme = ${JSON.stringify(theme)};
    prefs.zoom = 1;
    localStorage.setItem('owo.prefs', JSON.stringify(prefs));
    localStorage.setItem('owo.theme', ${JSON.stringify(theme)});
    localStorage.setItem('owo.theme-version', '2');
    return true;
  })()`);
  await cdp.send('Page.reload', { ignoreCache: true });
  await sleep(2200);
  await enableDomains(cdp);
  await setViewport(cdp, 1440, 900, 1);
  return await waitForApp(cdp);
}
async function ensureModelConnection(cdp) {
  const result = await cdp.evaluate(`(async () => {
    const fallback = ${JSON.stringify(MODEL)};
    const existing = window.OwoCustomModels?.models?.() || [];
    const id = String((typeof window.getDefaultModel === 'function' && window.getDefaultModel()) || existing[0]?.id || fallback).trim() || fallback;
    if (!window.owo?.customModels?.upsert || existing.some(model => model.id === id)) {
      return { created: false, id, count: existing.length };
    }
    await window.owo.customModels.upsert({ id, label: id, baseUrl: ${JSON.stringify(LOOPBACK)}, apiFormat: 'openai' }, '', null);
    return { created: true, id };
  })()`);
  if (result?.created) {
    await cdp.send('Page.reload', { ignoreCache: true });
    await sleep(1800);
    await enableDomains(cdp);
    await setViewport(cdp, 1440, 900, 1);
    await waitForApp(cdp);
  }
  return result;
}
async function resetUi(cdp) {
  await cdp.evaluate(`(() => {
    if (typeof window.closeComposerMenu === 'function') window.closeComposerMenu();
    if (typeof window.closeMenu === 'function') window.closeMenu();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    document.querySelectorAll('.composer-menu, .menu-popover:not(#menuPopover)').forEach(el => el.remove());
    if (typeof window.setSettingsPageVisible === 'function') window.setSettingsPageVisible(false);
    if (typeof window.setToolsVisible === 'function') window.setToolsVisible(false, true);
    document.querySelector('#sidebarHome')?.click();
    history.replaceState(null, '', location.pathname + location.search);
    window.scrollTo(0, 0);
    for (const el of [document.querySelector('#panelRoot'), document.querySelector('#sidebar'), document.querySelector('#settingsContent')]) if (el) el.scrollTop = 0;
    return true;
  })()`);
  await sleep(260);
}
async function openPanel(cdp, id) {
  await resetUi(cdp);
  const result = await cdp.evaluate(`(() => {
    if (typeof window.openPanelById === 'function') return window.openPanelById(${JSON.stringify(id)});
    location.hash = ${JSON.stringify('#' + id)};
    return true;
  })()`);
  await sleep(950);
  await cdp.evaluate(`(() => {
    const root = document.querySelector('#panelRoot');
    if (root) root.scrollTop = 0;
    document.querySelector('#sidebar')?.scrollTo(0, 0);
    return true;
  })()`);
  await sleep(160);
  return result;
}
async function openSettingsTab(cdp, tab) {
  await resetUi(cdp);
  const result = await cdp.evaluate(`(() => {
    document.querySelector('#codexSettingsEntry')?.click();
    const button = document.querySelector('#settingsNav .settings-nav-item[data-settings-tab="${tab}"]');
    if (button) button.click();
    const content = document.querySelector('#settingsContent');
    if (content) content.scrollTop = 0;
    return { clicked: Boolean(button), open: document.body.classList.contains('settings-open') };
  })()`);
  await sleep(650);
  return result;
}
async function openWorkswarmView(cdp, view) {
  await openPanel(cdp, 'workswarm');
  return await cdp.evaluate(`(() => {
    const tab = document.querySelector('.owo-ws-tab[data-view="${view}"]');
    if (tab) tab.click();
    const root = document.querySelector('#panelRoot');
    if (root) root.scrollTop = 0;
    return { clicked: Boolean(tab), view: ${JSON.stringify(view)} };
  })()`);
}
async function capture(cdp, phase, manifest, name, state = {}) {
  await cdp.send('Page.bringToFront').catch(() => {});
  await sleep(120);
  let shot, format = 'png';
  try { shot = await cdp.send('Page.captureScreenshot', { format, captureBeyondViewport: false }); }
  catch {
    try { shot = await cdp.send('Page.captureScreenshot', { format, fromSurface: false, captureBeyondViewport: false }); }
    catch {
      format = 'jpeg';
      shot = await cdp.send('Page.captureScreenshot', { format, quality: 86, fromSurface: false, captureBeyondViewport: false });
    }
  }
  const ext = format === 'png' ? 'png' : 'jpg';
  const file = path.join(EVIDENCE, phase, 'shots', `${name}.${ext}`);
  fs.writeFileSync(file, Buffer.from(shot.data, 'base64'));
  const metrics = await cdp.evaluate(`(() => {
    const root = document.querySelector('#panelRoot');
    const apiKey = document.querySelector('#providerApiKey');
    const activeTab = [...document.querySelectorAll('.settings-tab')].find(el => !el.hidden);
    return {
      url: location.href,
      viewport: [innerWidth, innerHeight],
      dpr: devicePixelRatio,
      theme: document.body.classList.contains('dark-theme') ? 'dark' : 'light',
      bodyClasses: [...document.body.classList],
      routeHash: location.hash,
      panelTitle: document.querySelector('#workbenchPanelTitle')?.textContent || '',
      settingsTab: activeTab?.dataset.settingsTab || '',
      panelRoot: root ? { scrollTop: root.scrollTop, scrollWidth: root.scrollWidth, clientWidth: root.clientWidth, textLength: root.innerText.length } : null,
      documentOverflowX: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      popovers: [...document.querySelectorAll('.menu-popover, .composer-menu')].filter(el => !el.hidden && !el.classList.contains('hidden')).length,
      modelChip: document.querySelector('#modelChip') ? { disabled: document.querySelector('#modelChip').disabled, text: document.querySelector('#modelChipText')?.textContent || '' } : null,
      apiKeyField: apiKey ? { type: apiKey.type, valueLength: apiKey.value.length } : null,
    };
  })()`);
  manifest.shots.push({
    name, file: path.relative(EVIDENCE, file).replace(/\\/g, '/'),
    bytes: fs.statSync(file).size, format, state, metrics,
  });
}
async function captureAll(cdp, phase, runtime, modelSetup, bootStatus) {
  const manifest = {
    phase,
    capturedAt: new Date().toISOString(),
    harness: 'isolated Electron/CDP; fixed theme and viewport; no fake business data',
    runtime: { pid: runtime.pid, port: runtime.port, reused: runtime.reused, coreSha256: runtime.coreSha256 || sha(CORE), webDir: WEB },
    conditions: {
      primaryViewport: [1440, 900], primaryDpr: 1, primaryTheme: 'light',
      windowState: 'isolated Electron window; CDP viewport emulation',
      dataState: 'fresh isolated core data plus one real loopback model connection; no seeded sessions, teams, automations or results',
      noFakeData: true,
    },
    modelSetup, bootStatus,
    sourceIdentity: sourceIdentity(),
    shots: [], consoleErrors: [], networkFailures: [], notes: [
      'Windows OS DPI was not changed; 125/150% shots use CDP deviceScaleFactor emulation and are not OS-level DPI certification.',
      'The debug core binary is prebuilt; this run serves current desktop/web files through OWO_WEB_UI_DIR without rebuilding Rust.',
      'Only localStorage theme/zoom preferences were fixed; no business data was injected into the renderer.',
    ],
  };
  await enableDomains(cdp);
  await setViewport(cdp, 1440, 900, 1);
  await setTheme(cdp, 'light');
  modelSetup = await ensureModelConnection(cdp);
  await setTheme(cdp, 'light');
  await resetUi(cdp);

  await capture(cdp, phase, manifest, 'page-01-chat-home', { scenario: 'PAGE-01', label: '主聊天首页（冻结骨架）' });

  await cdp.evaluate(`document.querySelector('#prompt')?.focus()`);
  await capture(cdp, phase, manifest, 'state-focus-composer', { scenario: 'STATE', label: '输入框键盘焦点' });
  await cdp.evaluate(`document.querySelector('#prompt')?.blur()`);
  const hoverRect = await cdp.evaluate(`(() => { const b=document.querySelector('#sidebarHome'); if(!b) return null; const r=b.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}; })()`);
  if (hoverRect?.x) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: hoverRect.x, y: hoverRect.y }).catch(() => {});
  await capture(cdp, phase, manifest, 'state-hover-sidebar-home', { scenario: 'STATE', label: '侧栏首页 hover' });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 }).catch(() => {});

  for (let i = 0; i < PANELS.length; i++) {
    const id = PANELS[i];
    await openPanel(cdp, id);
    await capture(cdp, phase, manifest, `panel-${String(i + 1).padStart(2, '0')}-${id}`, { scenario: 'PANEL', panel: id, label: `功能页 ${i + 1}/24` });
  }

  for (let i = 0; i < SETTINGS.length; i++) {
    const tab = SETTINGS[i];
    const result = await openSettingsTab(cdp, tab);
    await capture(cdp, phase, manifest, `settings-${String(i + 1).padStart(2, '0')}-${tab}`, { scenario: 'SETTINGS', tab, clicked: result?.clicked, label: `设置分类 ${i + 1}/11` });
  }

  await resetUi(cdp);
  const modelMenu = await cdp.evaluate(`(() => {
    const chip = document.querySelector('#modelChip');
    if (!chip || chip.disabled) return { opened: false, disabled: Boolean(chip?.disabled), text: document.querySelector('#modelChipText')?.textContent || '' };
    chip.click();
    return { opened: true, disabled: false, text: document.querySelector('#modelChipText')?.textContent || '' };
  })()`);
  await sleep(520);
  await capture(cdp, phase, manifest, 'page-03-model-selector', { scenario: 'PAGE-03', label: '模型选择器', modelMenu });
  await resetUi(cdp);

  const accountMenu = await cdp.evaluate(`(() => {
    if (typeof window.openUserMenu === 'function') window.openUserMenu();
    else document.querySelector('#codexUserCard')?.click();
    const menu = document.querySelector('#menuPopover');
    const rect = menu?.getBoundingClientRect();
    return {
      opened: Boolean(menu && !menu.classList.contains('hidden')),
      items: menu?.querySelectorAll('.menu-item').length || 0,
      insideViewport: Boolean(rect && rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth + 1 && rect.bottom <= innerHeight + 1),
    };
  })()`);
  await sleep(380);
  await capture(cdp, phase, manifest, 'page-06-account-menu', { scenario: 'PAGE-06', label: '账户菜单', ...accountMenu });
  await resetUi(cdp);

  await openSettingsTab(cdp, 'appearance');
  await capture(cdp, phase, manifest, 'page-04-appearance', { scenario: 'PAGE-04', label: '外观设置' });
  await openSettingsTab(cdp, 'models');
  await capture(cdp, phase, manifest, 'page-05-config', { scenario: 'PAGE-05', label: '配置设置（密钥遮蔽）' });

  await openPanel(cdp, 'home');
  await capture(cdp, phase, manifest, 'page-02-project-home', { scenario: 'PAGE-02', label: '项目首页' });

  await openWorkswarmView(cdp, 'list');
  await sleep(650);
  await capture(cdp, phase, manifest, 'page-07-team-list', { scenario: 'PAGE-07', label: '团队列表' });

  await openWorkswarmView(cdp, 'create');
  await sleep(500);
  await capture(cdp, phase, manifest, 'page-08-create-team', { scenario: 'PAGE-08', label: '创建团队' });

  const roles = await cdp.evaluate(`(() => {
    const add = document.querySelector('#ws-role-add');
    const before = document.querySelector('#ws-roles')?.children.length || 0;
    if (add) add.click();
    const block = document.querySelector('#ws-roles-block');
    if (block) block.scrollIntoView({ block: 'start', behavior: 'auto' });
    return { before, after: document.querySelector('#ws-roles')?.children.length || 0, clicked: Boolean(add) };
  })()`);
  await sleep(420);
  await capture(cdp, phase, manifest, 'page-09-role-specs', { scenario: 'PAGE-09', label: '角色规格', roles });
  await cdp.evaluate(`document.querySelector('#panelRoot')?.scrollTo(0,0)`);

  await openPanel(cdp, 'automations');
  const automation = await cdp.evaluate(`(() => { const b=document.querySelector('#owo-aut-new'); const e=document.querySelector('#owo-aut-editor'); if(b && e?.hidden!==false) b.click(); return { clicked: Boolean(b), expanded: e ? !e.hidden : false }; })()`);
  await sleep(420);
  await capture(cdp, phase, manifest, 'page-10-automations-editor', { scenario: 'PAGE-10', label: '定时任务编辑器', automation });

  await openPanel(cdp, 'custom-workflows');
  await capture(cdp, phase, manifest, 'page-11-custom-workflow', { scenario: 'PAGE-11', label: '自定义工作流' });
  const catalog = await cdp.evaluate(`(() => { const d=document.querySelector('.owo-wf-catalog'); if(d && !d.open) d.querySelector('summary')?.click(); return { opened: Boolean(d?.open) }; })()`);
  await sleep(380);
  await capture(cdp, phase, manifest, 'state-workflow-catalog-open', { scenario: 'STATE', label: '工作流能力折叠区展开', catalog });

  await openPanel(cdp, 'project-launcher');
  const attachments = await cdp.evaluate(`(() => { const f=document.querySelector('[data-task-inputs] fieldset'); if(f) f.scrollIntoView({ block:'center', behavior:'auto' }); return { found: Boolean(f), hint: f?.innerText.slice(0,180) || '' }; })()`);
  await sleep(420);
  await capture(cdp, phase, manifest, 'page-12-attachments', { scenario: 'PAGE-12', label: '任务附件区', attachments });

  await resetUi(cdp);
  for (const [w, h] of [[1280, 800], [1366, 768], [1920, 1080], [1024, 768]]) {
    await setViewport(cdp, w, h, 1);
    await resetUi(cdp);
    await capture(cdp, phase, manifest, `state-window-${w}x${h}-home`, { scenario: 'WINDOW', viewport: [w, h], label: `${w}×${h} 窗口适配` });
    if (w === 1024) {
      await openWorkswarmView(cdp, 'list');
      await capture(cdp, phase, manifest, 'state-narrow-team-list', { scenario: 'WINDOW', viewport: [w, h], label: '窄窗口团队列表' });
    }
  }
  await setViewport(cdp, 1152, 720, 1.25);
  await resetUi(cdp);
  await capture(cdp, phase, manifest, 'state-dpi-125-home', { scenario: 'DPI', dpr: 1.25, viewport: [1152, 720], label: 'CDP 125% DPI 仿真' });
  await setViewport(cdp, 960, 600, 1.5);
  await resetUi(cdp);
  await capture(cdp, phase, manifest, 'state-dpi-150-home', { scenario: 'DPI', dpr: 1.5, viewport: [960, 600], label: 'CDP 150% DPI 仿真' });
  await setViewport(cdp, 1440, 900, 1);

  await setTheme(cdp, 'dark');
  await resetUi(cdp);
  await capture(cdp, phase, manifest, 'state-dark-chat-home', { scenario: 'THEME', theme: 'dark', label: '深色主题首页' });
  await openWorkswarmView(cdp, 'list');
  await capture(cdp, phase, manifest, 'state-dark-team-list', { scenario: 'THEME', theme: 'dark', label: '深色主题团队列表' });
  await setTheme(cdp, 'light');
  await resetUi(cdp);

  await openPanel(cdp, 'workswarm');
  const panelTypography = await cdp.evaluate(`(() => {
    const el = document.querySelector('#workbenchPanelTitle');
    const style = el ? getComputedStyle(el) : null;
    return {
      bodyClasses: [...document.body.classList],
      fontSize: style?.fontSize || null,
      lineHeight: style?.lineHeight || null,
      fontWeight: style?.fontWeight || null,
      letterSpacing: style?.letterSpacing || null,
    };
  })()`);
  await openSettingsTab(cdp, 'models');
  const settingsTypography = await cdp.evaluate(`(() => {
    const el = document.querySelector('.settings-page-title strong');
    const style = el ? getComputedStyle(el) : null;
    return {
      bodyClasses: [...document.body.classList],
      fontSize: style?.fontSize || null,
      lineHeight: style?.lineHeight || null,
      fontWeight: style?.fontWeight || null,
      letterSpacing: style?.letterSpacing || null,
    };
  })()`);
  await resetUi(cdp);
  const audit = await cdp.evaluate(`(() => {
    const style = document.querySelector('link[href*="frontend-visual-system.css"]');
    const panelTitle = document.querySelector('#workbenchPanelTitle');
    const settingsTitle = document.querySelector('.settings-page-title strong');
    const overflow = document.documentElement.scrollWidth - document.documentElement.clientWidth;
    return {
      visualStylesheet: Boolean(style && style.sheet),
      panelTitleFontSize: panelTitle ? getComputedStyle(panelTitle).fontSize : null,
      settingsTitleFontSize: settingsTitle ? getComputedStyle(settingsTitle).fontSize : null,
      focusRing: getComputedStyle(document.querySelector('#panelRoot') || document.body).getPropertyValue('--focus-ring').trim(),
      documentOverflowX: overflow,
      apiKeyType: document.querySelector('#providerApiKey')?.type || null,
      apiKeyValueLength: document.querySelector('#providerApiKey')?.value.length || 0,
      scriptCount: document.querySelectorAll('script[src]').length,
      stylesheetCount: document.querySelectorAll('link[rel="stylesheet"]').length,
    };
  })()`);
  manifest.computedStyleAudit = audit;
  manifest.typographyAudit = { panel: panelTypography, settings: settingsTypography };

  for (const event of cdp.events) {
    if (event.method === 'Runtime.exceptionThrown') {
      const details = event.params?.exceptionDetails || {};
      manifest.consoleErrors.push({ text: scrub(details.text), description: scrub(details.exception?.description || ''), url: scrub(details.url || '') });
    } else if (event.method === 'Log.entryAdded' && ['error', 'warning'].includes(event.params?.entry?.level)) {
      const entry = event.params.entry;
      manifest.consoleErrors.push({ level: entry.level, text: scrub(entry.text), url: scrub(entry.url || '') });
    } else if (event.method === 'Network.loadingFailed') {
      manifest.networkFailures.push({ type: 'loadingFailed', errorText: scrub(event.params?.errorText || ''), requestId: event.params?.requestId || '' });
    } else if (event.method === 'Network.responseReceived' && event.params?.response?.status >= 400) {
      manifest.networkFailures.push({ type: 'http', status: event.params.response.status, url: scrub(event.params.response.url || '') });
    }
  }
  manifest.consoleErrors = manifest.consoleErrors.slice(0, 100);
  manifest.networkFailures = manifest.networkFailures.slice(0, 100);
  fs.writeFileSync(path.join(EVIDENCE, phase, 'metadata.json'), JSON.stringify(manifest, null, 2) + '\n', 'utf8');
  return manifest;
}
function sourceIdentity() {
  const css = {};
  for (const file of fs.readdirSync(WEB).filter(name => name.endsWith('.css')).sort()) css[file] = sha(path.join(WEB, file));
  const html = fs.readFileSync(path.join(WEB, 'index.html'), 'utf8').replace(/\r\n/g, '\n');
  const rawPrefix = html.slice(0, html.indexOf('  <aside id="right"'));
  const prefix = rawPrefix.replace('  <link rel="stylesheet" href="task-details.css?v=20261101-restore">\n', '');
  return {
    coreSha256: sha(CORE),
    indexHtmlSha256: createHash('sha256').update(html).digest('hex'),
    frozenHomePrefixSha256: createHash('sha256').update(prefix).digest('hex'),
    appJsSha256: sha(path.join(WEB, 'app.js')),
    css,
    captureScriptSha256: sha(new URL(import.meta.url)),
  };
}
async function main() {
  const [command = 'before'] = process.argv.slice(2);
  ensureDirs();
  if (command === 'stop') {
    if (!fs.existsSync(RUNTIME)) return console.log(JSON.stringify({ stopped: false, reason: 'no runtime record' }));
    const runtime = JSON.parse(fs.readFileSync(RUNTIME, 'utf8'));
    if (process.platform === 'win32') spawnSync('taskkill', ['/PID', String(runtime.pid), '/T', '/F'], { stdio: 'ignore' });
    else { try { process.kill(runtime.pid, 'SIGTERM'); } catch {} }
    fs.rmSync(PORT_FILE, { force: true });
    return console.log(JSON.stringify({ stopped: true, pid: runtime.pid }));
  }
  const runtime = await ensureRuntime();
  const { cdp, port } = await connectPage();
  try {
    await enableDomains(cdp);
    await setViewport(cdp, 1440, 900, 1);
    const bootStatus = await waitForApp(cdp);
    const modelSetup = await ensureModelConnection(cdp);
    if (command === 'probe') {
      const status = await cdp.evaluate(`(() => ({
        url: location.href, readyState: document.readyState,
        theme: document.body.classList.contains('dark-theme') ? 'dark' : 'light',
        panels: Object.keys(window.OwoPanels || {}).length,
        modelChip: document.querySelector('#modelChip') ? { disabled: document.querySelector('#modelChip').disabled, text: document.querySelector('#modelChipText')?.textContent } : null,
        customModels: (window.OwoCustomModels?.models?.() || []).map(m => ({ id: m.id, hasKey: Boolean(m.hasKey) })),
        frozenApiKeyField: document.querySelector('#providerApiKey') ? { type: document.querySelector('#providerApiKey').type, valueLength: document.querySelector('#providerApiKey').value.length } : null,
      }))()`);
      console.log(JSON.stringify({ runtime: { pid: runtime.pid, port, reused: runtime.reused }, bootStatus, modelSetup, status }, null, 2));
      return;
    }
    const phase = command === 'after' ? 'after' : 'before';
    const manifest = await captureAll(cdp, phase, { ...runtime, port }, modelSetup, bootStatus);
    console.log(JSON.stringify({ phase, shots: manifest.shots.length, metadata: path.join(EVIDENCE, phase, 'metadata.json'), consoleErrors: manifest.consoleErrors.length, networkFailures: manifest.networkFailures.length }, null, 2));
  } finally { cdp.close(); }
}
main().catch(error => { console.error(error.stack || error.message); process.exit(1); });
