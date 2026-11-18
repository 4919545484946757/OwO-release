// Desktop owns the gateway process and ephemeral origin key. No secrets cross IPC.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { spawn, execFile } = require("node:child_process");
const { promisify } = require("node:util");
const run = promisify(execFile);

function connectorSettings(localAppData) {
  return path.join(localAppData, "OwO", "InputMethod", "plugins", "data", "org.owo.agent-ipc", "settings.json");
}
function patchPairing(file, credential) {
  const settings = JSON.parse(fs.readFileSync(file, "utf8").replace(/^\uFEFF/, ""));
  const previous = { require_pairing: settings.require_pairing, pairing_credential_file: settings.pairing_credential_file };
  settings.require_pairing = true; settings.pairing_credential_file = credential;
  fs.writeFileSync(file, JSON.stringify(settings, null, 2) + "\n", { encoding: "utf8", mode: 0o600 });
  return () => {
    try {
      const current = JSON.parse(fs.readFileSync(file, "utf8").replace(/^\uFEFF/, ""));
      if (current.pairing_credential_file !== credential || current.require_pairing !== true) return;
      for (const key of Object.keys(previous)) {
        if (previous[key] === undefined) delete current[key]; else current[key] = previous[key];
      }
      fs.writeFileSync(file, JSON.stringify(current, null, 2) + "\n", "utf8");
    } catch { /* Preserve externally changed or unavailable settings. */ }
  };
}
async function installedConnector(localAppData) {
  // The IME launches this plugin on demand; never launch a standalone copy.
  const script = "$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); @(Get-CimInstance Win32_Process -Filter \"Name='owo-agent-ipc-plugin.exe'\" | Select-Object ProcessId,ExecutablePath) | ConvertTo-Json -Compress";
  const { stdout } = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { windowsHide: true, timeout: 10000, maxBuffer: 65536, encoding: "utf8" });
  const parsed = stdout.trim() ? JSON.parse(stdout.replace(/^\uFEFF/, "")) : [];
  const processes = Array.isArray(parsed) ? parsed : [parsed];
  const installedRoot = fs.realpathSync(path.join(localAppData, "OwO", "InputMethod", "plugins", "versions", "org.owo.agent-ipc"));
  const valid = processes.filter(item => {
    if (!Number.isInteger(item.ProcessId) || !item.ExecutablePath) return false;
    const relative = path.relative(installedRoot, fs.realpathSync(item.ExecutablePath));
    return relative && !relative.startsWith("..") && !path.isAbsolute(relative)
      && path.basename(relative).toLowerCase() === "owo-agent-ipc-plugin.exe";
  });
  if (valid.length !== 1) throw new Error(valid.length ? "检测到多个连接器，请保留当前输入法连接器后重试。" : "请先在 OwO 输入法中进入 Agent 模式，再点击连接。");
  const item = valid[0];
  return { pid: item.ProcessId, sha256: crypto.createHash("sha256").update(fs.readFileSync(item.ExecutablePath)).digest("hex") };
}
function gatewayFailure(code, reason) {
  const messages = {
    declined: "配对未获确认，请点击连接并在本机配对窗口选择“是”。",
    identity: "连接器与当前桌面登录身份不一致，请从当前用户的 OwO 输入法重新启动连接器。",
    installation: "连接器程序已变化，请重新启动输入法连接器后再连接。",
  };
  return messages[reason] || ({
    2: "网关启动参数或来源配置无效，请重启 Agent Desktop 后重试。",
    3: "未找到 Agent 核心，请在桌面设置中重启核心。",
    4: "无法创建网关工作目录，请检查 Agent 数据目录是否可写。",
    5: "本机配对失败，请确认连接器仍在运行且配对目录可写，然后重新连接。",
  })[code] || "输入法网关意外退出，请检查连接器进程和管道占用后重新连接。";
}
function create({ runtime, executable, localAppData, dataRoot, originKey, onOpenSession = () => {}, onOpenTask = () => {}, discoverConnector = installedConnector, spawnGateway = spawn }) {
  let current = null, starting = false, generation = 0;
  let state = { state: "disconnected", message: "尚未连接输入法" };
  function cleanup(item) {
    if (item.cleaned) return; item.cleaned = true;
    clearTimeout(item.expiry); item.restore?.();
    if (item.credential && path.dirname(item.credential) === item.directory) {
      try { fs.unlinkSync(item.credential); } catch {}
    }
    try { fs.rmdirSync(item.directory); } catch {}
  }
  async function stop() {
    const stopGeneration = ++generation;
    starting = false;
    const item = current;
    if (item) {
      current = null;
      // before-quit/process-exit cannot wait for a child exit event.
      item.restore?.(); item.restore = null;
      item.proc.kill();
      await new Promise(resolve => { if (item.proc.exitCode !== null) resolve(); else { item.proc.once("exit", resolve); setTimeout(resolve, 3000).unref(); } });
      cleanup(item);
    }
    if (generation === stopGeneration) state = { state: "disconnected", message: "已断开输入法；已提交的 Agent 任务可在工作台继续处理" };
    return state;
  }
  async function start() {
    if (current || starting) return state;
    starting = true;
    const startGeneration = ++generation;
    try {
    const core = runtime();
    if (!core?.owned || !core.port) throw new Error("请先在桌面设置中重启核心，再连接输入法。");
    const exe = executable();
    if (!exe || !fs.existsSync(exe)) throw new Error("缺少输入法网关程序，请安装包含输入法支持的桌面版本。");
    const connector = await discoverConnector(localAppData());
    if (generation !== startGeneration) return state;
    const directory = path.join(dataRoot(), "ime-pairings", crypto.randomUUID());
    fs.mkdirSync(path.dirname(directory), { recursive: true });
    const proc = spawnGateway(exe, ["--pair-connector-pid", String(connector.pid), "--pair-image-sha256", connector.sha256, "--pair-credential-directory", directory], {
      windowsHide: true, stdio: ["ignore", "pipe", "pipe"], cwd: path.dirname(exe),
      env: { ...process.env, OWO_AGENT_DATA: core.dataRoot, OWO_GATEWAY_DAEMON_URL: "http://127.0.0.1:" + core.port, OWO_EXPRESSION_ORIGIN_KEY: originKey, RUST_LOG: "info", NO_COLOR: "1", OWO_GATEWAY_DESKTOP_LINK: "1" },
    });
    const item = { proc, directory, restore: null, credential: null, expiry: null, cleaned: false };
    current = item; state = { state: "connecting", message: "请在本机配对窗口核对连接器并确认" };
    let output = "", lineBuffer = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", chunk => {
      output = (output + chunk).slice(-16384);
      lineBuffer = (lineBuffer + chunk).slice(-16384);
      if (item !== current) return;
      try {
        let end;
        while ((end = lineBuffer.indexOf("\n")) >= 0) {
          const line = lineBuffer.slice(0, end).trim(); lineBuffer = lineBuffer.slice(end + 1);
          if (line.startsWith("OWO_DESKTOP_OPEN_TASK ")) {
            const ref = JSON.parse(line.slice("OWO_DESKTOP_OPEN_TASK ".length));
            if (typeof ref.task_id === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(ref.task_id)) onOpenTask(ref.task_id);
          } else if (line === "OWO_DESKTOP_OPEN_TASKS") {
            onOpenTask(null);
          } else if (line.startsWith("OWO_DESKTOP_OPEN_SESSION ")) {
            const ref = JSON.parse(line.slice("OWO_DESKTOP_OPEN_SESSION ".length));
            if (/^[0-9a-f-]{36}$/.test(ref.session_id)) onOpenSession(ref.session_id);
          }
        }
        const line = output.match(/configure connector credential: ([^\r\n]+\.owokey)/);
        if (line && !item.credential) {
          const credential = path.resolve(line[1].trim());
          if (path.dirname(credential) !== directory || !/^[0-9a-f]+\.owokey$/.test(path.basename(credential))) throw new Error("连接器凭据路径无效");
          item.credential = credential;
          item.restore = patchPairing(connectorSettings(localAppData()), credential);
        }
        if (item.credential && output.includes("IME Agent 管道服务启动")) {
          state = { state: "connected", message: "输入法已连接 · 可在工作台查看和处理任务（配对有效一小时）" };
          if (!item.expiry) item.expiry = setTimeout(() => { void stop(); }, 3600 * 1000);
        }
      } catch {
        state = { state: "failed", message: "无法更新输入法连接配置，请检查插件设置文件" };
        proc.kill();
      }
    });
    // Never forward gateway output, credentials or environment into renderer IPC.
    // Retain only fixed error categories; never expose arbitrary child output.
    let errorBuffer = "", failureReason = null;
    proc.stderr.setEncoding("utf8");
    proc.stderr.on("data", chunk => {
      errorBuffer = (errorBuffer + chunk).slice(-2048);
      if (errorBuffer.includes("pairing not approved")) failureReason = "declined";
      else if (errorBuffer.includes("pipe identity mismatch")) failureReason = "identity";
      else if (errorBuffer.includes("connector does not match the reviewed installation")) failureReason = "installation";
    });
    proc.on("error", () => {
      if (current === item) { current = null; state = { state: "failed", message: "输入法网关启动失败" }; }
      cleanup(item);
    });
    proc.on("exit", code => {
      if (current === item) {
        current = null;
        if (state.state !== "failed") state = { state: "failed", message: gatewayFailure(code, failureReason) };
      }
      cleanup(item);
    });
    return state;
    } catch (error) {
      if (generation === startGeneration) state = { state: "failed", message: String(error.message || error) };
      return state;
    } finally { if (generation === startGeneration) starting = false; }
  }
  return { start, stop, status: () => ({ ...state }) };
}
module.exports = { create, patchPairing, connectorSettings, gatewayFailure };
