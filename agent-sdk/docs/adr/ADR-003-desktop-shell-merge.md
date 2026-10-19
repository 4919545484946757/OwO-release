# ADR-003：Electron 与 Tauri 双桌面壳合并方案

- **状态**：已批准（方案 A，Electron 壳吸收 Tauri）· **S1–S5 全部实施完成**（2026-10-02）
- **决策**：合并后 UI 以 web 工作台为准（用户 2026-10-02 确认）——壳加载核心服务托管的同一份 `desktop/web` 文件，与浏览器/旧 Tauri 壳所见一致
- **日期**：2026-10-02
- **背景审查**：`docs/reports/archive/2026-10/code-review-2026-10-02.md`（A1/B2/C8 项）

---

## 1. 现状事实（代码证据）

### 1.1 两壳的真实关系

**Electron 壳本来就是 Tauri 壳的"替代品"**，不是平行演化的两套产品：

> `desktop/electron/src/main/main.js:1-7`：
> "Electron 主进程：OwO Agent 桌面壳（**替代 Tauri 壳**）。为什么换：用户明确要求'别再用 Rust 壳、前端改成组件化动态渲染'……职责（**与旧 Tauri 壳对齐，语义不变**）"

时间线：tauri 壳最后提交 2026-09-23（4,664 行 Rust），electron 壳其后接棒（当前 407 行 main.js + 1,547 行含渲染层），web 工作台持续活跃至 10-01。

### 1.2 能力对照（逐文件核对结论）

| 能力 | Tauri 壳（现状） | Electron 壳（现状） | 差距判定 |
|---|---|---|---|
| 核心拉起 + core_ready 握手 | ✅ `core_runtime.rs` launch_once | ✅ main.js:238-317 | 持平 |
| core_fatal 致命行解析 | ✅ 稳定码 layer/name 校验 | ✅ 基本解析（main.js:287） | 基本持平 |
| **实例身份 + API 版本双重校验** | ✅ `core_supervisor.rs:103-141` wait_for_instance（只认自己启动的核心，防端口误连） | ❌ 无（`CORE_API_VERSION` 在 main.js:23 定义了但从未使用） | **electron 缺** |
| **崩溃监管（supervise 循环）** | ✅ `core_runtime.rs:387` supervise + generation 代际守卫（过期监管循环不得操作新核心） | ❌ 核心退出仅置 failed，需手动重启 | **electron 缺** |
| **复用已存活核心** | ✅ `core_runtime.rs:506` try_reuse_existing_core | ❌ 每次先 killCore 再拉新 | **electron 缺** |
| 单实例 + 二次启动唤醒 | ✅ `single_instance.rs`（477 行：命名互斥 + 唤醒管道 + 错误框） | ❌ 无 | **electron 缺**（electron 有内置 API，成本极低） |
| ledger 来源标签 | ✅ 所有壳侧请求带 `x-owo-client: shell`（core_supervisor.rs:58，带单测） | ❌ httpGet/httpPost 未打标 | **electron 缺**（3 行） |
| 模型配置读写/校验/ACL | ✅ `provider.rs`（833 行：ProviderMode、校验、旧配置迁移、ACL 加固） | ✅ 基本等价（main.js:78-165：DEFAULT_CONFIG 合并、icacls 加固）；缺旧路径迁移与 validate | 小差距 |
| 托盘 + 开机自启 | ✅ main.rs:16,127-156（TrayIconBuilder + HKCU Run 读写） | ❌（main.js:401 注释自认"要常驻可以改成托盘模式"） | **electron 缺** |
| 自动更新 | ✅ tauri updater 插件已配（但端点是占位符，main.rs:34 有检测） | ❌ | electron 缺（两端都是占位水平） |
| **NSIS 打包 + sidecar + rg 资源** | ✅ `tauri.conf.json` bundle 完整配置（externalBin owo-agent、rg.exe 五件资源、SimpChinese、currentUser） | ❌ 无打包配置 | **electron 缺** |
| **渲染层 = web 工作台** | ✅ `tauri.conf.json` frontendDist = `../../web` | ❌ 自带旧渲染层（app.js 776 + 自制 reactive.js 216 + api.js 129） | **electron 接错前端** |

### 1.3 关键事实：web 工作台已被 server 静态托管

`owo-agent-server/src/lib.rs:771`：`.fallback_service(ServeDir::new(desktop_web_dir()))` —— 核心服务在 `/` 直接托管 web 工作台。**这意味着 electron 壳根本不需要自带渲染层**，core ready 后 `loadURL("http://127.0.0.1:{port}/")` 即可加载与浏览器/t组里完全一致的工作台（web 的 `core/api-client.js` 自带 token 引导、断线重连、recovery/service-error/setup-guide 视图，全部在浏览器语境下已验证）。

---

## 2. 决策

### 推荐方案 A：**Electron 壳吸收 Tauri 精华，web 工作台为唯一渲染层**

**理由：**

1. **这是既定方向的收尾，不是新决策**——electron 注释明确记录了用户"别再用 Rust 壳"的要求；合并 = 把 tauri 尚未搬完的监管深度搬进 electron，然后让它体面退场。
2. **壳层是高频迭代层**：改 UI/交互不应触发 Rust 编译（用户当初切换的直接动因，见 main.js:3-5）。tauri 的监管栈虽然深，但它解决的是"一次性正确性"问题——移植一次后不再随 UI 演进，**一次性成本换长期迭代速度**。
3. **electron 补齐缺口的大部分成本极低**：单实例是内置 API（6 行 vs Rust 477 行）；托盘/自启 `app.setLoginItemSettings`（约 40 行 vs winreg 手写）；实例校验/监管循环约 150 行 JS。真正有工程量的只有打包链（electron-builder 配置，约 1 天）。
4. **渲染层三合一顺带完成**（审查报告 A2/B2 项）：electron 旧渲染层（自制 mini-Vue + 手写 api.js）删除后，全项目只剩 web 工作台一个前端 + clients/ts 一个类型面。

### 备选方案 B（否决，但记录）：Tauri 为壳、删除 Electron

- 优点：内存占用小约 100MB；监管栈已有单测；NSIS/updater 已配好。
- 否决理由：违背用户"别再用 Rust 壳"的明确要求；壳层迭代回到"改一行编译半天"；electron 侧 6 周活跃开发的语义对齐工作白费。
- **如果未来内存 footprint 成为一票否决项**（例如低端机长驻场景），本方案整体反转的成本 = 本 ADR 的镜像，两方案共享同一张能力映射表。

---

## 3. 能力移植映射（方案 A 的执行清单）

| # | Tauri 来源 | 移植到 electron | 动作 | 预估 |
|---|---|---|---|---|
| M1 | `single_instance.rs` | `main.js` 顶部 | `app.requestSingleInstanceLock()`；`second-instance` 事件 → 聚焦主窗口 | 0.1 天 |
| M2 | `core_supervisor.rs:58` | `httpGet/httpPost` | 两个函数统一加 `x-owo-client: shell` 头 | 0.1 天 |
| M3 | `core_supervisor.rs:103-141` | 新增 `wait-for-ready` 逻辑 | 启动时生成 `OWO_DESKTOP_INSTANCE_ID`（uuid）注入核心环境；core_ready 后轮询 `/health` 校验 `instance_id` + `api_version`（`CORE_API_VERSION` 从 0.7 起真正启用） | 0.5 天 |
| M4 | `core_runtime.rs:257,387` | `startCore` 改造 | generation 计数器：所有异步回调先比对代际再生效；意外退出 → 指数退避自动重启（连续失败 3 次转 failed 待手动）；用户主动 restart 不计入失败 | 1 天 |
| M5 | `core_runtime.rs:506` | 启动前探测 | 启动前 GET `/health`（带超时），发现已存活且 api_version 兼容的核心 → 复用不杀 | 0.5 天 |
| M6 | `provider.rs:318,258` | `writeConfig` 前置 | 补 `validate()`（provider/base_url/model 联合校验）与 legacy `provider.json` 一次性迁移 | 0.5 天 |
| M7 | `main.rs:16,127-156` | 新增 tray.js 模块 | 托盘（显示/隐藏主窗口、退出）+ "开机自启：开/关" 菜单项（`app.setLoginItemSettings`，替代 HKCU Run 手写） | 0.5 天 |
| M8 | `tauri.conf.json` bundle | 新增 `electron-builder.yml` | NSIS target、`extraResources` 携带 owo-agent.exe + tools/rg 五件、SimpChinese、currentUser 安装；`start.ps1`/dev 流程同步 | 1 天 |
| M9 | 渲染层切换 | `createWindow` | 见 §4 | 0.5 天 |
| M10 | `core_supervisor.rs` 的测试 | 新增 `desktop/electron/tests/*.test.mjs` | ready/fatal 行解析、实例校验、代际守卫的纯函数单测（node --test，进 ci-gate 5b 步旁） | 0.5 天 |

**总计约 5 个工作日**（一人，含调试与验收）。

---

## 4. 渲染层切换细节（M9）

```
现状：mainWindow.loadFile(renderer/index.html)   ← 旧自制 Vue 风格渲染层
目标：core ready → mainWindow.loadURL(`http://127.0.0.1:${port}/`)
```

时序处理：

1. 窗口立即创建（保持启动体感），先加载一个极简本地占位页（内联 10 行"正在连接核心…"，不复用旧渲染层任何代码）；
2. `coreState.state === "ready"` 时切 `loadURL` 到核心托管的 web 工作台；
3. `failed` 时占位页展示 `coreState.errorCode/message`（对应 tauri 壳的 Failed 可观测性，`core_fatal` 稳定码直通）；
4. 删除 `src/renderer/` 整目录（app.js、reactive.js、api.js、index.html、style.css）与 `preload.js` 中的渲染层专用通道；preload 仅保留 `app:openExternal`（外链兜底）——**web 工作台原生目录选择已走服务端 `/fs/pick-directory`**（engine 合并 fs 路由），不再需要壳侧 dialog；
5. CSP 交给核心服务（server 侧已带 `no-store` 与静态托管安全头），electron 侧不再自维护一份。

风险与对策：

- **核心崩溃 = 白屏**：web 工作台自带 `recovery.js`/`service-error.view.js`/断线重连（clients 侧 battle-tested），且 M4 的自动重启会把核心拉回来，重连后自愈；
- **端口猜错**：M3 的实例身份校验从根上排除"连到别的核心"；
- **回滚**：渲染层切换是单 commit，revert 即回到旧渲染层。

---

## 5. 实施顺序（每步独立可验收、可提交）

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **S1** | M9 渲染层切换 + 删旧渲染层 | `npm start` 看到完整 web 工作台；core 崩溃后重连自愈；revert 单 commit 可回滚 |
| **S2** | M2+M3+M4+M5（监管四件套）+ M10 单测 | 杀核心进程 3 秒内自动重启；二次启动聚焦既有窗口；壳请求在 server ledger 显示 source=shell；node --test 全绿 |
| **S3** | M1+M6+M7（单实例/配置校验/托盘自启） | 双击二次启动不双开；托盘开关自启后重启机器生效；脏 config.json 被校验拦截 |
| **S4** | M8 打包链 | `electron-builder` 产出 NSIS：装完即用（内含 owo-agent.exe + rg），卸载干净 |
| **S5** | Tauri 退场：`desktop/tauri` 整目录删除（git 历史保全），README/AGENTS.md 壳层描述更新 | 全仓 grep 无 tauri 残留引用；文档同步 |

依赖关系：S1 与 S2 可并行；S3 依赖 S2 的状态机；S4 依赖 S1（打包内容不再含旧渲染层）；S5 最后。

## 6. 规则合规注意

- 新增/修改 `.ps1`（如 ci-gate 增补 electron 测试步）必须带 UTF-8 BOM（AGENTS.md 红线）；
- M10 的 node --test 需要挂进 `scripts/ci-gate.ps1` 现有 5b 步旁（web-tests 步骤扩为 `desktop/web + desktop/electron` 两目录）；
- 本 ADR 全程不动 Rust 代码，`cargo fmt`/`clippy` 面（除 tauri 目录删除外）零影响。

---

## 7. 决策与实施进度

**已决策**：方案 A（2026-10-02 用户确认），UI 目标 = web 工作台。

**S1 已实施（2026-10-02，单 commit 可回滚）**：

| 改动 | 文件 |
|---|---|
| core ready 后 `loadURL("http://127.0.0.1:{port}/")`；核心重启端口重分配时自动重新导航 | `main.js` notifyState（新增 uiPort 记忆） |
| 启动占位页（正在拉起 / 失败错误码），已进工作台后核心崩溃不再抢占画面（交给 web 的 recovery） | `main.js` bootPage / showBootPage |
| 删除自带渲染层 | `src/renderer/` 整目录（app.js 776 + reactive.js 216 + api.js 129 + index.html + style.css） |
| 外链走系统浏览器（web 工作台无需壳 IPC） | `main.js` setWindowOpenHandler |
| 顺带完成 M2：壳侧请求统一 `x-owo-client: shell` ledger 标签 | `main.js` httpGet / httpPost |
| 描述与注释同步（含 .ps1 BOM 校验通过） | `package.json`、`start.ps1` |

**为何"UI 与功能一模一样"**：不是重写一遍，而是加载同一份 `desktop/web` 文件——electron（Chromium/Blink）与 Tauri on Windows（WebView2，同为 Blink）渲染引擎同源，后端同为 127.0.0.1 核心服务，故布局、面板、SSE 流式、权限中心（`permissions/` 模块）、诊断、原生选目录（服务端 `/fs/pick-directory`）全部一致。web 工作台是旧 electron 渲染层的超集，切换只有新增没有丢失。

**S2 监管四件套 + 单测（已实施）**：新增 `src/main/core-supervision.js`（**不 import electron，可单测的纯逻辑模块**）承载
parseReadyLine / parseFatalLine / evaluateHealth（M3 实例身份 + API 版本双重校验）/ pollDelayMs / nextBackoffMs /
shouldAutoRestart（M4 代际守卫）/ discoveryPort（M5）/ normalizeProvider + validateConfig（M6）；主进程只做编排。
配套 `tests/core-supervision.test.mjs` **14 项 node --test 全绿**（对应原 Rust `core_supervisor.rs` 的同名断言），
已挂进 `scripts/ci-gate.ps1` 5b 步（与 web 契约测试同一步，`.ps1` BOM 已校验）。
> 单测过程中揪出一个真 bug：凭据告警原是死代码（`api_key_env` 空串会回落默认值导致永不告警），
> 改为由调用方注入 `envHas` 判定，消除假告警的同时让告警真正生效。

**S3 单实例 / 配置校验 / 托盘自启（已实施）**：`app.requestSingleInstanceLock()` + second-instance 聚焦（替代 Rust 477 行）；
写入配置前 `validateConfig`（结构性错误拒绝写入，凭据缺失只回 warnings）+ 旧 `provider.json` 一次性迁移；
托盘（显示/隐藏、核心状态、重启核心、打开配置目录、开机自启、退出）+ `app.setLoginItemSettings` 替代 HKCU Run 手写；
关窗改为收进托盘（有托盘时），退出走托盘菜单；托盘图标缺失时降级为无托盘，不让壳起不来。

**S4 打包链（已实施）**：新增 `desktop/electron/electron-builder.yml`——NSIS（简体中文、当前用户安装）、
`extraResources` 携带核心 `owo-agent.exe` + `resources/web` + `tools/rg`（对标旧 Tauri bundle 段）。
配套修复：`desktop_web_dir()` 现支持 `OWO_WEB_UI_DIR` 覆盖（对齐 `pet_ui_dir()` 既有约定），
壳在 `app.isPackaged` 时把 `resources/web` 指给核心——**否则打包后核心会回落到编译期源码树路径导致白屏**。
`cargo check -p owo-agent-server -j 2` 通过；顺带修掉 `assist_api.rs` 一处历史 `cargo fmt` diff。
> 自动更新：旧 Tauri updater 端点本就是 example.com 占位符，Electron 侧同样标记为待接入（electron-updater），不宣称已支持。

**S5 Tauri 退场（已实施）**：`desktop/tauri/` 整目录删除（git 历史保全）；README 桌面主客户端章节、
NSIS/更新章节、P1 描述全部改为 Electron 表述；CI 门禁已含 electron 单测。
`ACCEPTANCE.md` 与 `docs/internal/dead-code-inventory.md` 中的 Tauri 条目属**历史验收记录**，按"不改写历史"保留。

**验收方式（本地）**：`cd agent-sdk\desktop\electron && npm install && pwsh -File start.ps1` —— 窗口应先显示"正在拉起核心服务…"，随后进入与浏览器打开 `http://127.0.0.1:<端口>/` 完全相同的工作台；改 `desktop/web` 任意文件后 Ctrl+R 即生效。

---

## 8. S6：删壳后的断裂修复（2026-10-02 复核发现，均已实施）

删除 `desktop/tauri` 之后做了一次"能力面 vs 调用面"的复核，发现**只做 UI 加载是不够的**——web 工作台对壳的依赖不在渲染层，而在一条看不见的 IPC 通道上。以下四项已补齐：

| # | 缺口 | 后果（不补会怎样） | 处置 |
|---|---|---|---|
| **M11** | **Tauri 兼容命令桥**：web 通过 `window.__TAURI_INTERNALS__.invoke(name, args)` 调壳，共 **14 个命令**分布在 `api-client.js`、`folder-picker.js`、`settings-panel.view.js`、`setup-guide.view.js`、`service-error.view.js`、`status-bar.view.js` | 该全局对象随 Tauri 一起消失 → **模型配置保存、原生目录选择器、工作区设置、provider 切换、诊断页「重启核心/打开日志」全部静默降级为"非桌面环境"**。UI 长得一模一样，功能少一半——这正是用户"功能一模一样"这条硬约束最容易翻车的地方 | 新增 `src/main/shell-commands.js`（**纯逻辑、不 import electron**）承载 provider 默认值/密钥掩码/读改写语义；`main.js` 新增 `ipcMain.handle("shell:invoke")` 按**同名同形状**实现 14 个命令；`preload.js` 用 `contextBridge` 原样暴露 `__TAURI_INTERNALS__` 与 `__TAURI__.core`。**desktop/web 一行未改** |
| **M12** | 全局快捷键 `Ctrl+Alt+Shift+O` 唤起工作台（旧壳 `main.rs` 有，初版移植遗漏） | 收进托盘后没有唤起手段（只能靠托盘菜单） | `globalShortcut.register` + `showWorkspace()`（窗口已销毁则重建）；注册失败只降级记日志，与旧壳策略一致；`before-quit` 时 `unregisterAll` |
| **M13** | 进程配对证明（旧壳 `CoreRuntime::pairing`，64 位十六进制） | 不注入也能跑（服务端 `pairing_gate_allows` 对 `None` 放行），但等于**把本地 token 引导开放给任意本地浏览器**，安全强度悄悄降级 | 启动时生成并注入 `OWO_DESKTOP_PAIRING_SECRET`；壳的 HTTP 请求统一带 `x-owo-desktop-pairing`；`desktop_pairing` 命令与 `get_core_connection.pairing` 同步提供；**写日志前 `redactSecrets` 脱敏**（对齐旧壳 `redact`） |
| **M14** | 核心日志落盘（旧壳 `runtime.log_path()`） | 打包形态没有终端，核心 stdout 全丢；诊断页与 `open_core_logs` 无路径可给 | 新增 `logDir()`/`coreLogPath()`（`%LOCALAPPDATA%\OwO\Agent\logs\core.log`），核心 stdout/stderr 追加写盘并在退出时收尾；`get_core_state`/`get_core_connection` 带出 `logPath` |

附带补齐：`choose_data_directory` 需要的数据目录改指能力（旧壳 `save_data_root_override`，指针文件 `data_root.json`），Electron 侧 `dataRoot()` 改为读该指针、缺省回落 `%LOCALAPPDATA%\OwO\Agent`。

### 8.1 脚本迁移（删壳的直接连带影响）

`grep` 出 9 个脚本引用 `desktop/tauri/src-tauri`，逐一定级处理：

| 脚本 | 处置 |
|---|---|
| `ci-gate.ps1` | **无需改**：桌面壳四步本就有 `$shellPresent` 守卫，缺目录即跳过并打印说明 |
| `dev.ps1` | 版本一致性校验改读 `desktop/electron/package.json`（原读 tauri.conf.json + src-tauri/Cargo.toml 两处）；三处版本现均为 0.1.0 |
| `stage-desktop-sidecar.ps1` | 随包 core 目的地改为 `desktop/electron/binaries/owo-agent.exe`（Electron 不做三元组取包）；保留清残留 + 构建身份 + SHA-256 核对 |
| `package-desktop.ps1` | 壳构建由 `cargo build`（src-tauri）改为 `npm run dist:dir`，产物取 `dist\electron\win-unpacked`；便携版说明与 ONNX 核对改指 `OwO Agent.exe` / `resources\owo-agent.exe` |
| `build-installer.ps1` | NSIS 由 `npx @tauri-apps/cli build` 改为 `npm run dist`（electron-builder）；随之删除"临时移开 `.cargo/config.toml` 绕开 tauri-build 的 static_vcruntime 冲突"整段补丁——Electron 侧不存在该链路 |
| `desktop-acceptance-common.ps1`、`verify-desktop-cold-boot.ps1`、`probe-desktop-failure-state.ps1` | **待迁移**：旧模型是"复制单个 exe + 同级 dll 到私有 bin"，Electron 是目录形态，不适用。已改为**显式抛错并指明 ADR-003**，而不是留一条已失效的默认路径把迁移缺口伪装成"文件缺失" |
| `generate-update-manifest.ps1` | **待迁移**：依赖 Tauri signer；Electron 侧 `publish: []` 本就是占位。已改为显式抛错，避免"能跑但签的是不存在的产物" |

`electron-builder.yml` 的 `extraResources` 现从 `binaries/owo-agent.exe` 取核心（而不是直接取 `target/release`），并由 npm 的 `predist` 自动跑 staging——**保留"随包 core 必须经校验"这道门**，避免重演历史上"安装包里装着上一代核心"的错包。

### 8.2 关于 web 契约测试的 38 项失败（重要澄清）

复核时发现 `node --test "desktop/web/tests/*.test.mjs"` 为 **373 测试 / 335 通过 / 38 失败**。已验证**与本次合并无关**：

1. 临时 `git checkout HEAD -- desktop/tauri` 恢复被删目录后重跑，结果仍为 373/335/38；
2. `ci-gate.ps1` 在本次改动**之前**就已执行该测试组（不是我新挂上去才变红）；
3. 失败性质是 `desktop/web/app.js` 缺失功能（`needsSetup`/`renderSetupGuide` 等），而 `desktop/web/` 在本轮**零改动**。

真实成因是合并提交 `2d23cd2`（"工作台 UI 对齐上游 be6298f"）一次性改动 app.js 6,909 行 / index.html 1,282 行 / style.css 3,503 行，**覆盖了本土功能层**：`a18ce87` 版 app.js 含 `needsSetup`/`renderSetupGuide` 共 7 处，当前版本 0 处。

已单独立项为审查报告的 **A0（P0）**，恢复方案见 `docs/reports/archive/2026-10/code-review-2026-10-02.md` §5 的 Phase 0.5。**不混入 ADR-003 提交**——这是两件事：换壳（已完成）与修合并回归（待做）。

### 8.3 复核教训

初版 S1–S5 的验收声明是"加载同一份 web 文件 → UI 与功能一模一样"。**这个推论只对渲染层成立**：web 对壳的 14 条 IPC 依赖不在文件里，删掉 Tauri 之后它们不会报错，只会静默降级。结论写进方法论——**换壳类改动必须做"调用面对账"（grep 出全部 `invoke(` 并逐条确认有实现），而不能只比对 UI。**
