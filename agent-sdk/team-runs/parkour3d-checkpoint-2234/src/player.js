// src/player.js
// three.js 跑酷玩家控制器中的纯逻辑部分。
//
// 设计约定：
// - 本模块不 import three.js、不访问 DOM / window / document，可被 Node（node --test）直接 import。
// - state.position 表示角色脚底（feet）位置，角色本体是一个以 (x,z) 为中心、
//   半径 PLAYER.radius、高 PLAYER.height 的 AABB。
// - 坐标系与 three.js 默认一致：y 向上，yaw = 0 时“前”为 -Z、“右”为 +X。
// - 输入语义（可用键盘事件名直接构造）：{forward, right, jump} 或按键/事件数组，
//   映射表见 KEY_BINDINGS：WASD / 方向键移动，空格跳跃。
// - update(dt, input) 与 syncMesh(mesh, state) 供 main.js 使用；物理推进由 stepPlayer 完成。

const EPS = 1e-6;
const MAX_STEP = 0.1;

/** 玩家可调参数（单位：米 / 秒） */
export const PLAYER = Object.freeze({
  radius: 0.35,
  height: 1.7,
  maxSpeed: 6,
  acceleration: 45,
  friction: 30,
  airAcceleration: 22,
  airFriction: 4,
  gravity: -26,
  jumpSpeed: 9.5,
  maxFallSpeed: 30,
  /** 站立检测向下探针距离 */
  groundProbe: 0.03,
  /** 默认的掉出边界高度（y 低于该值即重生） */
  fallLimit: -15,
  /** mesh 原点（角色中心）相对脚底的偏移 */
  meshYOffset: 0.85,
});

/** 事件名 / 按键 → 动作。WASD、方向键移动，空格跳跃。 */
export const KEY_BINDINGS = Object.freeze({
  w: 'forward',
  arrowup: 'forward',
  up: 'forward',
  s: 'back',
  arrowdown: 'back',
  down: 'back',
  a: 'left',
  arrowleft: 'left',
  left: 'left',
  d: 'right',
  arrowright: 'right',
  right: 'right',
  space: 'jump',
});

/** KEY_BINDINGS 的别名，方便 integrator 按需取用 */
export const KEYS = KEY_BINDINGS;

const ACTIONS = new Set(['forward', 'back', 'left', 'right', 'jump']);

function num(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function clamp(value, min, max) {
  if (value < min) return min;
  if (value > max) return max;
  return value;
}

function approach(current, target, maxDelta) {
  if (!(maxDelta > 0)) return current;
  const diff = target - current;
  if (Math.abs(diff) <= maxDelta) return target;
  return current + Math.sign(diff) * maxDelta;
}

function normalizeKeyName(raw) {
  let name = String(raw).trim();
  if (name === ' ' || name === '') return 'space';
  name = name.toLowerCase();
  if (name === 'spacebar' || name === 'space' || name === ' ') return 'space';
  if (name.startsWith('key')) name = name.slice(3);
  else if (name.startsWith('digit')) name = name.slice(5);
  return name.toLowerCase();
}

/**
 * 把一个按键名 / KeyboardEvent / 动作名解析为动作，无法识别时返回 null。
 * @returns {'forward'|'back'|'left'|'right'|'jump'|null}
 */
export function resolveAction(input) {
  if (input == null) return null;
  if (typeof input === 'string') {
    const direct = input.trim().toLowerCase();
    if (ACTIONS.has(direct)) return direct;
  }
  const raw =
    typeof input === 'object' ? (input.key ?? input.code ?? input.name ?? null) : input;
  if (raw == null) return null;
  const name = normalizeKeyName(raw);
  if (ACTIONS.has(name)) return name;
  return KEY_BINDINGS[name] ?? null;
}

/** 新建一个输入状态对象（main.js 可复用同一对象，避免每帧分配） */
export function createInputState() {
  return { forward: 0, right: 0, jump: false };
}

/**
 * 把按键数组 / Set / 按键状态归一化为 {forward, right, jump}。
 * forward/right ∈ [-1, 1]（前为 +，右为 +），jump 为布尔。
 */
export function normalizeInput(input) {
  if (input == null) return { forward: 0, right: 0, jump: false };
  if (typeof input === 'string') return normalizeInput([input]);

  if (Array.isArray(input) || input instanceof Set) {
    let forward = 0;
    let back = 0;
    let left = 0;
    let right = 0;
    let jump = false;
    for (const item of input) {
      switch (resolveAction(item)) {
        case 'forward':
          forward = 1;
          break;
        case 'back':
          back = 1;
          break;
        case 'left':
          left = 1;
          break;
        case 'right':
          right = 1;
          break;
        case 'jump':
          jump = true;
          break;
        default:
          break;
      }
    }
    return { forward: forward - back, right: right - left, jump };
  }

  if (typeof input !== 'object') return { forward: 0, right: 0, jump: false };

  const axis = (positive, negative) => {
    let value = 0;
    if (typeof positive === 'number') value += num(positive);
    else if (positive) value += 1;
    if (typeof negative === 'number') value -= num(negative);
    else if (negative) value -= 1;
    return clamp(value, -1, 1);
  };

  return {
    forward: axis(input.forward, input.back),
    right: axis(input.right, input.left),
    jump: Boolean(input.jump ?? input.space ?? input.jumpHeld ?? false),
  };
}

/** 把按键/动作集合写入已有的输入状态对象 */
export function applyActions(target, actions) {
  const normalized = normalizeInput(actions);
  target.forward = normalized.forward;
  target.right = normalized.right;
  target.jump = normalized.jump;
  return target;
}

/** 按键数组 → 输入对象 */
export function inputFromKeys(keys) {
  return normalizeInput(keys);
}

/** 创建玩家状态（position 为脚底位置，初始与重生点一致） */
export function createPlayerState(options = {}) {
  const spawn = options.respawnPoint ?? options.spawn ?? options.position ?? { x: 0, y: 0, z: 0 };
  const point = { x: num(spawn.x), y: num(spawn.y), z: num(spawn.z) };
  return {
    position: { ...point },
    velocity: { x: 0, y: 0, z: 0 },
    respawnPoint: { ...point },
    onGround: false,
    standingPlatformId: null,
    jumpHeld: false,
    respawns: 0,
    yaw: num(options.yaw),
    lastEvent: null,
  };
}

/** 角色的世界 AABB */
export function playerBounds(state) {
  const { x, y, z } = state.position;
  const r = PLAYER.radius;
  return {
    min: { x: x - r, y, z: z - r },
    max: { x: x + r, y: y + PLAYER.height, z: z + r },
  };
}

/** 归一化各种 AABB 描述：{min,max} / {x,y,z,width,height,depth} / {aabb: ...} */
export function toBox(raw) {
  if (!raw) return null;
  if (raw.min && raw.max) {
    return {
      id: raw.id ?? null,
      min: { x: num(raw.min.x), y: num(raw.min.y), z: num(raw.min.z) },
      max: { x: num(raw.max.x), y: num(raw.max.y), z: num(raw.max.z) },
      delta: raw.delta
        ? { x: num(raw.delta.x), y: num(raw.delta.y), z: num(raw.delta.z) }
        : null,
    };
  }
  if (raw.aabb) {
    const box = toBox(raw.aabb);
    if (box) box.id = raw.id ?? box.id;
    return box;
  }
  if (typeof raw.x === 'number' && typeof raw.width === 'number') {
    const y = num(raw.y);
    const z = num(raw.z);
    return {
      id: raw.id ?? null,
      min: { x: num(raw.x), y, z },
      max: { x: num(raw.x) + num(raw.width), y: y + num(raw.height, 1), z: z + num(raw.depth) },
      delta: raw.delta
        ? { x: num(raw.delta.x), y: num(raw.delta.y), z: num(raw.delta.z) }
        : null,
    };
  }
  return null;
}

/** 收集关卡里所有实体碰撞盒（平台 / 障碍等） */
export function collectSolids(world = {}) {
  const solids = [];
  for (const key of ['platforms', 'solids', 'obstacles', 'boxes']) {
    const source = world[key];
    if (!source || typeof source[Symbol.iterator] !== 'function') continue;
    for (const raw of source) {
      const box = toBox(raw);
      if (!box) continue;
      if (box.id == null) box.id = `${key}-${solids.length}`;
      solids.push(box);
    }
  }
  return solids;
}

function overlaps(a, b) {
  return (
    a.min.x < b.max.x - EPS &&
    a.max.x > b.min.x + EPS &&
    a.min.y < b.max.y - EPS &&
    a.max.y > b.min.y + EPS &&
    a.min.z < b.max.z - EPS &&
    a.max.z > b.min.z + EPS
  );
}

function overlapsXZ(a, b) {
  return (
    a.min.x < b.max.x - EPS &&
    a.max.x > b.min.x + EPS &&
    a.min.z < b.max.z - EPS &&
    a.max.z > b.min.z + EPS
  );
}

function moveAxis(state, axis, delta, solids) {
  if (!delta) return;
  state.position[axis] += delta;
  const bounds = playerBounds(state);
  let limit = null;
  for (const box of solids) {
    if (!overlaps(bounds, box)) continue;
    if (delta > 0) {
      const candidate = box.min[axis] - PLAYER.radius;
      if (limit === null || candidate < limit) limit = candidate;
    } else {
      const candidate = box.max[axis] + PLAYER.radius;
      if (limit === null || candidate > limit) limit = candidate;
    }
  }
  if (limit !== null) {
    state.position[axis] = limit;
    state.velocity[axis] = 0;
  }
}

function findSupport(state, solids, groundY) {
  const y = state.position.y;
  const probe = PLAYER.groundProbe;
  const bounds = playerBounds(state);
  let support = null;
  for (const box of solids) {
    if (!overlapsXZ(bounds, box)) continue;
    const top = box.max.y;
    if (top <= y + 1e-4 && top >= y - probe) {
      if (!support || top > support.top) support = { top, id: box.id };
    }
  }
  const flat = groundY == null ? null : Number(groundY);
  if (flat != null && Number.isFinite(flat) && flat <= y + 1e-4 && flat >= y - probe) {
    if (!support || flat > support.top) support = { top: flat, id: 'ground' };
  }
  return support;
}

/** 把玩家重置到重生点（不触发重生回调，也不累加 respawns） */
export function resetPlayer(state, options = {}) {
  const source =
    options.respawnPoint ?? options.spawn ?? state.respawnPoint ?? { x: 0, y: 0, z: 0 };
  state.respawnPoint = { x: num(source.x), y: num(source.y), z: num(source.z) };
  state.position = { ...state.respawnPoint };
  state.velocity = { x: 0, y: 0, z: 0 };
  state.onGround = false;
  state.standingPlatformId = null;
  state.jumpHeld = false;
  state.lastEvent = null;
  return state;
}

/** 掉出边界后的重生：重置状态并调用 state / world 上的 onRespawn 回调 */
export function respawnPlayer(state, world = {}) {
  const from = { ...state.position };
  state.respawns += 1;
  resetPlayer(state, { respawnPoint: world.respawnPoint ?? state.respawnPoint });
  state.lastEvent = 'respawn';
  const info = {
    from,
    to: { ...state.position },
    respawns: state.respawns,
    reason: 'fell-out-of-bounds',
  };
  if (typeof state.onRespawn === 'function') state.onRespawn(state, info);
  if (typeof world.onRespawn === 'function') world.onRespawn(state, info);
  return state;
}

/**
 * 推进一帧物理。纯函数式副作用：就地修改并返回 state。
 * @param {object} state createPlayerState() 的结果
 * @param {object|string[]} input {forward, right, jump} 或按键/事件数组
 * @param {number} dt 秒
 * @param {object} ctx {platforms, obstacles, groundY, fallLimit, respawnPoint, onRespawn}
 */
export function stepPlayer(state, input, dt = 0, ctx = {}) {
  const world = ctx || {};
  const step = clamp(num(dt), 0, MAX_STEP);
  if (!(step > 0)) return state;

  const solids = collectSolids(world);
  const rawInput = normalizeInput(input);

  // 1) 站立于移动平台：先跟随平台本帧位移
  if (state.onGround && state.standingPlatformId != null) {
    const platform = solids.find((box) => box.id === state.standingPlatformId);
    if (platform && platform.delta) {
      state.position.x += platform.delta.x;
      state.position.y += platform.delta.y;
      state.position.z += platform.delta.z;
    }
  }

  // 2) 水平速度：加速度 / 摩擦 / 最大速度
  const yaw = num(state.yaw);
  const dirX = -Math.sin(yaw) * rawInput.forward + Math.cos(yaw) * rawInput.right;
  const dirZ = -Math.cos(yaw) * rawInput.forward - Math.sin(yaw) * rawInput.right;
  const wishLength = Math.hypot(dirX, dirZ);
  if (wishLength > 1e-4) {
    const nx = dirX / wishLength;
    const nz = dirZ / wishLength;
    const accel = (state.onGround ? PLAYER.acceleration : PLAYER.airAcceleration) * step;
    state.velocity.x = approach(state.velocity.x, nx * PLAYER.maxSpeed, accel);
    state.velocity.z = approach(state.velocity.z, nz * PLAYER.maxSpeed, accel);
  } else {
    const decel = (state.onGround ? PLAYER.friction : PLAYER.airFriction) * step;
    state.velocity.x = approach(state.velocity.x, 0, decel);
    state.velocity.z = approach(state.velocity.z, 0, decel);
  }
  const horizontal = Math.hypot(state.velocity.x, state.velocity.z);
  if (horizontal > PLAYER.maxSpeed) {
    const scale = PLAYER.maxSpeed / horizontal;
    state.velocity.x *= scale;
    state.velocity.z *= scale;
  }

  // 3) 跳跃（仅落地时可起跳；按住不会连跳）、重力与下落速度限制
  const jumpPressed = Boolean(rawInput.jump);
  const jumpStarted = jumpPressed && !state.jumpHeld && state.onGround;
  state.jumpHeld = jumpPressed;
  if (jumpStarted) {
    state.velocity.y = PLAYER.jumpSpeed;
    state.onGround = false;
    state.standingPlatformId = null;
  } else if (state.onGround && state.velocity.y <= 0) {
    state.velocity.y = 0;
  }
  if (!state.onGround) {
    state.velocity.y = Math.max(
      state.velocity.y + PLAYER.gravity * step,
      -PLAYER.maxFallSpeed,
    );
  }

  // 4) 水平位移 + AABB 阻挡
  moveAxis(state, 'x', state.velocity.x * step, solids);
  moveAxis(state, 'z', state.velocity.z * step, solids);

  // 5) 竖直位移 + 落地 / 撞头判定
  const previousY = state.position.y;
  state.position.y += state.velocity.y * step;

  let landed = null;
  let bumped = null;
  const bounds = playerBounds(state);
  for (const box of solids) {
    if (!overlaps(bounds, box)) continue;
    if (state.velocity.y <= 0 && previousY >= box.max.y - 1e-6) {
      if (!landed || box.max.y > landed.top) landed = { top: box.max.y, id: box.id };
    } else if (state.velocity.y > 0 && previousY + PLAYER.height <= box.min.y + 1e-6) {
      if (!bumped || box.min.y < bumped.bottom) bumped = { bottom: box.min.y };
    }
  }

  if (landed) {
    state.position.y = landed.top;
    state.velocity.y = 0;
    state.onGround = true;
    state.standingPlatformId = landed.id;
  } else if (bumped) {
    state.position.y = bumped.bottom - PLAYER.height;
    state.velocity.y = 0;
    state.onGround = false;
    state.standingPlatformId = null;
  } else if (state.velocity.y <= 0) {
    const support = findSupport(state, solids, world.groundY);
    if (support) {
      state.position.y = support.top;
      state.velocity.y = 0;
      state.onGround = true;
      state.standingPlatformId = support.id;
    } else {
      state.onGround = false;
      state.standingPlatformId = null;
    }
  } else {
    state.onGround = false;
    state.standingPlatformId = null;
  }

  // 6) 掉出边界 → 重生（重置到重生点并回调）
  const fallLimit = Number.isFinite(Number(world.fallLimit))
    ? Number(world.fallLimit)
    : PLAYER.fallLimit;
  if (state.position.y < fallLimit) {
    respawnPlayer(state, world);
  }

  return state;
}

/** 创建一个带独立世界配置的玩家控制器（供 main.js 使用） */
export function createPlayer(options = {}) {
  const state = options.state ?? createPlayerState(options);
  const world =
    options.world ??
    {
      platforms: options.platforms ?? [],
      obstacles: options.obstacles ?? [],
      groundY: options.groundY,
      fallLimit: options.fallLimit,
      respawnPoint: state.respawnPoint,
      onRespawn: options.onRespawn,
    };
  if (typeof options.onRespawn === 'function') state.onRespawn = options.onRespawn;

  return {
    state,
    world,
    setWorld(next) {
      this.world = next ?? {};
      return this;
    },
    update(dt, input) {
      stepPlayer(state, input, dt, this.world);
      return state;
    },
    reset(overrides) {
      resetPlayer(state, overrides ?? this.world);
      return state;
    },
    syncMesh(mesh, meshOptions) {
      return syncMesh(mesh, state, meshOptions);
    },
  };
}

/** 模块级默认玩家实例（main.js 可直接用 update / syncMesh 这两个导出） */
export const defaultPlayer = createPlayer();

/** main.js：每帧推进默认玩家 */
export function update(dt, input) {
  return defaultPlayer.update(dt, input);
}

/** main.js：读取默认玩家状态 */
export function getPlayerState() {
  return defaultPlayer.state;
}

/**
 * 把玩家状态同步到 three.js 网格（只要求 mesh 有 position/rotation，可用假对象测试）。
 * 默认把 mesh 原点放在角色中心（脚底 + meshYOffset）；options.feetOrigin = true 时与脚底对齐。
 */
export function syncMesh(mesh, state = defaultPlayer.state, options = {}) {
  if (!mesh) return mesh;
  const source = state ?? defaultPlayer.state;
  const y = options.feetOrigin
    ? source.position.y
    : source.position.y + (options.yOffset ?? PLAYER.meshYOffset);
  if (mesh.position) {
    if (typeof mesh.position.set === 'function') {
      mesh.position.set(source.position.x, y, source.position.z);
    } else {
      mesh.position.x = source.position.x;
      mesh.position.y = y;
      mesh.position.z = source.position.z;
    }
  }
  if (mesh.rotation && typeof mesh.rotation === 'object') {
    mesh.rotation.y = num(source.yaw);
  }
  return mesh;
}
