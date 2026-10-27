/**
 * src/main.js — 3D 跑酷游戏集成层
 *
 * 职责（只做「接线」，不改写已有三个模块）：
 * - 复用 src/world.js（关卡片数据 + 移动平台 + 终点/掉落判定）搭建世界；
 * - 复用 src/player.js（物理步进 / 重生 / 网格同步 / 输入归一化）驱动角色；
 * - 复用 src/ui.js（计时 / 得分 / 状态 / overlay / restart 绑定）驱动 HUD；
 * - three.js 从 CDN（importmap 的 "three" 或 THREEE_CDN_URL 兜底）**动态**加载，
 *   因此本文件在 Node 下可被 `node --test` 直接 import（顶层不碰 DOM、不拉 CDN）；
 * - requestAnimationFrame 主循环 + 键盘输入（WASD / 方向键 / 空格 / R）。
 *
 * 公开 API：
 *   THREE_CDN_URL                 CDN 兜底地址（与 index.html importmap 保持一致）
 *   loadThree(options)            动态加载 THREE（先 import('three')，再回退 CDN）
 *   createGame(options)           无 DOM 也能构造的游戏对象（可注入 fake THREE）
 *   stepGame(game, dt, input)     推进一帧：移动平台 → 角色物理 → 胜负判定 → HUD
 *   restartGame(game)             重新开始（复用 ui.onRestart 重置计时/得分/局号）
 *   resetWorldMotion(game)        重置移动平台相位
 *   syncGame(game)                把玩家状态写进网格、把 HUD 状态写进 DOM（无 DOM 安全降级）
 *   snapshot(game)                当前局面快照（测试 / 调试用）
 *   createPlayerMesh(THREE)       角色网格
 *   updateCamera(camera, state)   第三人称跟随相机
 *   mount(THREE, documentRef)     浏览器装配：renderer / scene / 灯光 / 输入 / RAF 循环
 *   boot(options)                 浏览器启动入口（加载 three.js 后 mount）
 */

import { WORLD_SPEC, buildWorld, getCollisionData } from './world.js';
import {
  PLAYER,
  createPlayerState,
  resetPlayer,
  stepPlayer,
  syncMesh,
  inputFromKeys,
} from './player.js';
import {
  CONTROL_HINT,
  createHudState,
  tickTimer,
  addScore,
  updateHud,
  formatTime,
  formatHud,
  overlayText,
  onRestart,
  createHud as createHudElements,
  bindHudEvents,
} from './ui.js';

/* ------------------------------------------------------------------ */
/* 常量                                                                */
/* ------------------------------------------------------------------ */

/** three.js CDN 地址（与 index.html 的 importmap 同版本，作为加载兜底） */
export const THREE_CDN_URL = 'https://unpkg.com/three@0.160.0/build/three.module.js';

/** 掉落重生上限：达到后本局判负（overlay 里可点「重新开始」） */
export const MAX_RESPAWNS = 5;
/** 通关基础得分 */
export const WIN_SCORE = 1000;
/** 每次掉落的扣分 */
export const FALL_PENALTY = 50;
/** 单帧最大步长（秒），防止切后台回来后穿透 */
export const MAX_DT = 0.1;
/** 默认帧步长 */
export const DEFAULT_DT = 1 / 60;
/** 相机相对玩家的偏移（+Z 在角色身后） */
export const CAMERA_OFFSET = Object.freeze({ x: 0, y: 4.5, z: 9.5 });
/** 角色网格颜色 */
export const PLAYER_COLOR = 0x38e8b0;

/* ------------------------------------------------------------------ */
/* 小工具                                                              */
/* ------------------------------------------------------------------ */

function num(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

/** dt 归一化：非法 / 非正 → 0；过大 → MAX_DT */
export function clampDt(dt) {
  const value = Number(dt);
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.min(value, MAX_DT);
}

function pushEvent(game, event) {
  if (!Array.isArray(game.events)) game.events = [];
  game.events.push(event);
  while (game.events.length > 32) game.events.shift();
  return event;
}

/* ------------------------------------------------------------------ */
/* 关卡 → player 的碰撞上下文                                           */
/* ------------------------------------------------------------------ */

/**
 * 把 world 的碰撞体（平台 / 移动平台 / 障碍）转成 player.collectSolids 认识的 AABB 列表。
 * 移动平台保留 delta，player 才能「站在移动平台上跟随位移」。
 */
export function levelToSolids(world) {
  const solids = [];
  for (const collider of getCollisionData(world)) {
    if (!collider || collider.solid === false || !collider.box) continue;
    solids.push({
      id: collider.id,
      min: {
        x: num(collider.box.min.x),
        y: num(collider.box.min.y),
        z: num(collider.box.min.z),
      },
      max: {
        x: num(collider.box.max.x),
        y: num(collider.box.max.y),
        z: num(collider.box.max.z),
      },
      delta:
        collider.moving && collider.delta
          ? { x: num(collider.delta.x), y: num(collider.delta.y), z: num(collider.delta.z) }
          : null,
    });
  }
  return solids;
}

/**
 * player.stepPlayer 的 ctx。
 * - platforms 里包含全部实心碰撞体（平台 + 移动平台 + 障碍），障碍因此天然阻挡角色；
 * - fallLimit 取 world.killY - 3，留出 3m 余量：正常掉落重生由 world.evaluatePlayer 统一处理，
 *   player 内部的极限兜底只在异常情况（例如渲染掉帧穿越）才触发。
 */
export function playerContext(world) {
  const killY = Number.isFinite(world && world.killY) ? Number(world.killY) : WORLD_SPEC.killY;
  const respawn = (world && world.respawn) || WORLD_SPEC.respawn;
  return {
    platforms: levelToSolids(world),
    obstacles: [],
    groundY: null,
    respawnPoint: { x: num(respawn.x), y: num(respawn.y), z: num(respawn.z) },
    killY,
    fallLimit: killY - 3,
  };
}

/* ------------------------------------------------------------------ */
/* three.js：CDN 动态加载 + 网格                                        */
/* ------------------------------------------------------------------ */

/**
 * 动态加载 three.js。
 * 1) 先试裸模块名 'three'（index.html 里由 importmap 映射到 CDN）；
 * 2) 失败则回退到 options.url 或 THREE_CDN_URL。
 * 注意：Node 环境下两种方式都不可用，本函数只应在浏览器调用。
 */
export async function loadThree(options = {}) {
  const url = options.url || THREE_CDN_URL;
  try {
    const mapped = await import('three');
    if (mapped && (mapped.WebGLRenderer || mapped.Scene || mapped.Mesh)) return mapped;
  } catch (error) {
    // 没有 importmap（例如直接开 file:// 或用 Node 跑测试）时忽略，继续走 CDN
  }
  return import(url);
}

/** 角色网格（盒子近似 AABB：宽 = 2*radius，高 = height） */
export function createPlayerMesh(THREE, color = PLAYER_COLOR) {
  if (!THREE || typeof THREE.Mesh !== 'function') return null;
  const size = { x: PLAYER.radius * 2, y: PLAYER.height, z: PLAYER.radius * 2 };
  const geometry =
    typeof THREE.BoxGeometry === 'function'
      ? new THREE.BoxGeometry(size.x, size.y, size.z)
      : null;
  const material =
    typeof THREE.MeshStandardMaterial === 'function'
      ? new THREE.MeshStandardMaterial({ color, roughness: 0.42, metalness: 0.12 })
      : null;
  const mesh = new THREE.Mesh(geometry, material);
  mesh.name = 'player';
  mesh.userData = { id: 'player', type: 'player' };
  if ('castShadow' in mesh) mesh.castShadow = true;
  if ('receiveShadow' in mesh) mesh.receiveShadow = true;
  return mesh;
}

/* ------------------------------------------------------------------ */
/* 游戏对象                                                            */
/* ------------------------------------------------------------------ */

/**
 * 构造游戏对象（不访问 DOM、不需要真实 three.js）。
 * @param {object} [options]
 * @param {object|null} [options.THREE]        three.js 命名空间（Node 测试可传 fake 或 null）
 * @param {object} [options.level]             关卡定义，默认 world.js 的 WORLD_SPEC
 * @param {object|null} [options.documentRef]  类似 document 的适配器（可选，用于 HUD）
 */
export function createGame(options = {}) {
  const THREE = options.THREE || null;
  const level = options.level || WORLD_SPEC;
  const documentRef = options.documentRef || null;

  const world = buildWorld(THREE, level);
  const hud = createHudState({ status: 'ready', runId: 1, time: 0, score: 0 });
  const state = createPlayerState({ respawnPoint: world.respawn, yaw: 0 });
  const playerMesh = createPlayerMesh(THREE);

  const game = {
    THREE,
    level,
    world,
    hud,
    state,
    ctx: playerContext(world),
    playerMesh,
    hudElements: createHudElements(documentRef),
    elapsed: 0,
    events: [],
    spawn: { ...world.spawn },
    respawnPoint: { ...world.respawn },
  };

  // 初始同步一次，保证 HUD / 网格与初始状态一致
  syncGame(game);
  return game;
}

/* ------------------------------------------------------------------ */
/* 每帧推进                                                            */
/* ------------------------------------------------------------------ */

/**
 * 推进一帧。
 * 顺序：移动平台 → 角色物理（含跳跃 / 重力 / AABB 阻挡）→ world 统一判定 → HUD / 网格同步。
 * 结束态（won / lost）冻结物理与计时，等待 restartGame。
 */
export function stepGame(game, dt, input = null) {
  if (!game) return game;
  const hud = game.hud;

  if (hud.status === 'won' || hud.status === 'lost') {
    syncGame(game);
    return game;
  }

  const delta = clampDt(dt);

  if (hud.status === 'ready' && delta > 0) hud.status = 'running';

  // 1) 关卡：移动平台按正弦轨迹往返，并写入本帧 delta
  game.world.update(delta);
  // 2) 刷新碰撞上下文（含移动平台 delta），再推进角色物理
  game.ctx = playerContext(game.world);
  stepPlayer(game.state, input, delta, game.ctx);
  // 3) 计时
  tickTimer(hud, delta);
  game.elapsed += delta;

  // 4) 统一判定：终点胜利 / 掉落重生（判定逻辑全部来自 world.js）
  const verdict = game.world.evaluatePlayer(game.state.position);
  if (verdict && verdict.win) {
    resolveWin(game);
  } else if (verdict && verdict.reset) {
    resolveFall(game, verdict);
  }

  syncGame(game);
  return game;
}

/** 通关：结算得分、切 won 状态（overlay 由 ui.updateHud 显示） */
export function resolveWin(game) {
  const hud = game.hud;
  hud.status = 'won';
  const bonus = Math.max(0, WIN_SCORE - Math.floor(num(hud.time) * 10));
  addScore(hud, bonus);
  hud.bestScore = Math.max(num(hud.bestScore), hud.score);
  pushEvent(game, {
    type: 'win',
    time: num(hud.time),
    score: num(hud.score),
    bonus,
    position: { ...game.state.position },
  });
  return game;
}

/** 掉落：重置到关卡重生点、扣分、累计重生次数；达到上限则判负 */
export function resolveFall(game, verdict) {
  const from = { ...game.state.position };
  const respawnPoint = (verdict && verdict.respawn) || game.world.respawn;
  resetPlayer(game.state, { respawnPoint });
  game.state.respawns = num(game.state.respawns) + 1;
  game.state.lastEvent = 'respawn';
  addScore(game.hud, -FALL_PENALTY);

  if (game.state.respawns >= MAX_RESPAWNS) {
    game.hud.status = 'lost';
  } else {
    game.hud.status = 'running';
  }

  pushEvent(game, {
    type: 'respawn',
    reason: 'fell-out-of-bounds',
    from,
    to: { ...game.state.position },
    respawns: game.state.respawns,
    status: game.hud.status,
  });
  return game;
}

/* ------------------------------------------------------------------ */
/* restart                                                             */
/* ------------------------------------------------------------------ */

/** 重置移动平台到初始相位（world.time = 0 时偏移量为 amplitude*sin(phase)） */
export function resetWorldMotion(game) {
  const world = game.world;
  world.time = 0;
  world.elapsed = 0;
  if (typeof world.update === 'function') world.update(0, 0);
  for (const mover of world.movers || []) {
    if (mover.delta) {
      mover.delta.x = 0;
      mover.delta.y = 0;
      mover.delta.z = 0;
    }
  }
  return world;
}

/**
 * 重新开始：复用 ui.onRestart 重置计时 / 得分 / 状态 / 局号，并通过钩子重置玩家与关卡。
 * 结束后 overlay 会被 updateHud 隐藏（status = running）。
 */
export function restartGame(game) {
  if (!game) return game;
  onRestart(game.hud, {
    resetPlayer: () => {
      resetPlayer(game.state, { respawnPoint: game.world.respawn });
      game.state.respawns = 0;
      game.state.yaw = num(game.state.yaw);
      game.state.lastEvent = 'restart';
    },
    resetWorld: () => {
      resetWorldMotion(game);
      game.ctx = playerContext(game.world);
    },
  });
  game.elapsed = 0;
  pushEvent(game, { type: 'restart', runId: game.hud.runId });
  syncGame(game);
  return game;
}

/* ------------------------------------------------------------------ */
/* 同步 / 快照                                                         */
/* ------------------------------------------------------------------ */

/** 玩家状态 → 网格；HUD 状态 → DOM（documentRef 缺省时 updateHud 安全降级） */
export function syncGame(game) {
  if (!game) return game;
  if (game.playerMesh) syncMesh(game.playerMesh, game.state);
  if (game.hudElements) updateHud(game.hudElements, game.hud);
  return game;
}

/** 当前局面快照（测试 / 调试） */
export function snapshot(game) {
  if (!game) return null;
  return {
    status: game.hud.status,
    time: num(game.hud.time),
    score: num(game.hud.score),
    runId: num(game.hud.runId),
    timeLabel: formatTime(game.hud.time),
    hudLine: formatHud(game.hud),
    overlay: overlayText(game.hud),
    controls: CONTROL_HINT,
    respawns: num(game.state.respawns),
    position: { ...game.state.position },
    velocity: { ...game.state.velocity },
    onGround: Boolean(game.state.onGround),
    standingPlatformId: game.state.standingPlatformId,
    elapsed: num(game.elapsed),
    lastEvent: game.events.length ? game.events[game.events.length - 1] : null,
  };
}

/* ------------------------------------------------------------------ */
/* 相机                                                                */
/* ------------------------------------------------------------------ */

/** 第三人称跟随相机：位于角色身后上方，看向角色胸口 */
export function updateCamera(camera, state, offset = CAMERA_OFFSET) {
  if (!camera || !state || !state.position) return camera;
  const eye = {
    x: state.position.x + num(offset.x),
    y: state.position.y + num(offset.y),
    z: state.position.z + num(offset.z),
  };
  if (camera.position && typeof camera.position.set === 'function') {
    camera.position.set(eye.x, eye.y, eye.z);
  }
  if (typeof camera.lookAt === 'function') {
    camera.lookAt(state.position.x, state.position.y + 1, state.position.z);
  }
  return camera;
}

/* ------------------------------------------------------------------ */
/* 浏览器装配 + 主循环                                                  */
/* ------------------------------------------------------------------ */

/**
 * 浏览器装配：renderer / scene / 灯光 / 世界 / 角色 / 键盘 / RAF 主循环。
 * @returns {{game: object, scene: object, camera: object, renderer: object, stop: Function}}
 */
export function mount(THREE, documentRef = globalThis.document) {
  const doc = documentRef;
  if (!THREE || typeof THREE.WebGLRenderer !== 'function') {
    throw new Error('mount 需要可用的 three.js（WebGLRenderer 缺失）');
  }
  if (!doc || typeof doc.getElementById !== 'function') {
    throw new Error('mount 需要 document（用于取得 canvas 与 HUD 元素）');
  }

  const canvas = doc.getElementById('game');
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  if (typeof renderer.setPixelRatio === 'function') {
    renderer.setPixelRatio(Math.min(globalThis.devicePixelRatio || 1, 2));
  }

  const scene = new THREE.Scene();
  if (typeof THREE.Color === 'function') scene.background = new THREE.Color(0x05070d);

  const camera = new THREE.PerspectiveCamera(60, readAspect(canvas), 0.1, 500);
  updateCamera(camera, { position: WORLD_SPEC.spawn });

  // 灯光：半球光 + 主方向光（投射阴影）
  if (typeof THREE.HemisphereLight === 'function') {
    scene.add(new THREE.HemisphereLight(0xdfe9ff, 0x0a1020, 1.1));
  }
  if (typeof THREE.DirectionalLight === 'function') {
    const sun = new THREE.DirectionalLight(0xffffff, 1.5);
    sun.position.set(12, 22, 10);
    if (sun.shadow && sun.shadow.camera) {
      sun.shadow.camera.left = -30;
      sun.shadow.camera.right = 30;
      sun.shadow.camera.top = 30;
      sun.shadow.camera.bottom = -30;
    }
    scene.add(sun);
  }

  const game = createGame({ THREE, documentRef: doc });
  if (game.world.group) scene.add(game.world.group);
  if (game.playerMesh) scene.add(game.playerMesh);

  // HUD：restart 按钮
  bindHudEvents(game.hudElements, { onRestart: () => restartGame(game) });

  // 键盘：WASD / 方向键移动，空格跳跃，R 重新开始
  const keys = new Set();
  const onKeyDown = (event) => {
    if (!event) return;
    const raw = String(event.key ?? event.code ?? '').toLowerCase();
    if (!raw) return;
    keys.add(raw);
    if (raw === 'r') restartGame(game);
    if (raw === ' ' || raw === 'space' || raw === 'spacebar' || raw.startsWith('arrow')) {
      if (typeof event.preventDefault === 'function') event.preventDefault();
    }
  };
  const onKeyUp = (event) => {
    if (!event) return;
    const raw = String(event.key ?? event.code ?? '').toLowerCase();
    if (raw) keys.delete(raw);
  };
  const onBlur = () => keys.clear();
  const win = typeof globalThis !== 'undefined' ? globalThis : null;
  if (win && typeof win.addEventListener === 'function') {
    win.addEventListener('keydown', onKeyDown);
    win.addEventListener('keyup', onKeyUp);
    win.addEventListener('blur', onBlur);
  }

  // 尺寸自适应
  const onResize = () => {
    if (typeof renderer.setSize === 'function') {
      renderer.setSize(readWidth(canvas), readHeight(canvas), false);
    }
    camera.aspect = readAspect(canvas);
    if (typeof camera.updateProjectionMatrix === 'function') camera.updateProjectionMatrix();
  };
  onResize();
  if (win && typeof win.addEventListener === 'function') win.addEventListener('resize', onResize);

  // 主循环
  let handle = null;
  let last = null;
  let running = true;
  const frame = (timestamp) => {
    if (!running) return;
    const now = num(timestamp, 0);
    const dt = last == null ? DEFAULT_DT : clampDt((now - last) / 1000);
    last = now;
    stepGame(game, dt, inputFromKeys(keys));
    updateCamera(camera, game.state);
    if (typeof renderer.render === 'function') renderer.render(scene, camera);
    handle = globalThis.requestAnimationFrame(frame);
  };
  handle = globalThis.requestAnimationFrame(frame);

  const stop = () => {
    running = false;
    if (handle != null && typeof globalThis.cancelAnimationFrame === 'function') {
      globalThis.cancelAnimationFrame(handle);
    }
    if (win && typeof win.removeEventListener === 'function') {
      win.removeEventListener('keydown', onKeyDown);
      win.removeEventListener('keyup', onKeyUp);
      win.removeEventListener('blur', onBlur);
      win.removeEventListener('resize', onResize);
    }
  };

  return { game, scene, camera, renderer, stop };
}

function readWidth(canvas) {
  const width = canvas && num(canvas.clientWidth);
  return width > 0 ? width : 1280;
}

function readHeight(canvas) {
  const height = canvas && num(canvas.clientHeight);
  return height > 0 ? height : 720;
}

function readAspect(canvas) {
  const width = readWidth(canvas);
  const height = readHeight(canvas);
  return height > 0 ? width / height : 16 / 9;
}

/* ------------------------------------------------------------------ */
/* 启动                                                                */
/* ------------------------------------------------------------------ */

/**
 * 浏览器启动：加载 three.js → 装配 → 返回 mount 结果。
 * @param {object} [options] { THREE?, documentRef?, url? }
 */
export async function boot(options = {}) {
  const doc = options.documentRef || globalThis.document;
  const THREE = options.THREE || (await loadThree(options));
  const mounted = mount(THREE, doc);
  if (typeof globalThis === 'object') globalThis.__PARKOUR__ = mounted;
  return mounted;
}

/** 启动失败时把提示写进 overlay（不抛异常，避免整页静默失败） */
function reportBootError(error) {
  const message = `three.js 加载失败：${error && error.message ? error.message : error}`;
  if (typeof console !== 'undefined' && typeof console.error === 'function') {
    console.error('[parkour]', message);
  }
  const doc = globalThis.document;
  const target = doc && typeof doc.getElementById === 'function' ? doc.getElementById('overlay-text') : null;
  if (target) target.textContent = message;
}

const isBrowser =
  typeof window !== 'undefined' &&
  typeof document !== 'undefined' &&
  typeof document.getElementById === 'function';

if (isBrowser) {
  const start = () => {
    boot().catch(reportBootError);
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
}

export default {
  THREE_CDN_URL,
  MAX_RESPAWNS,
  WIN_SCORE,
  FALL_PENALTY,
  MAX_DT,
  DEFAULT_DT,
  CAMERA_OFFSET,
  clampDt,
  loadThree,
  createPlayerMesh,
  createGame,
  stepGame,
  resolveWin,
  resolveFall,
  resetWorldMotion,
  restartGame,
  syncGame,
  snapshot,
  updateCamera,
  mount,
  boot,
};
