/**
 * 纯 ESM 玩家控制器（parkour-3d）
 *
 * 设计约定：
 * - 不依赖任何渲染/浏览器 API，也不 import three.js；主循环由 main.js 每帧调用。
 * - state.position 表示玩家 AABB 的“脚底中心点”（feet center），y 即脚底高度。
 *   玩家 AABB：宽 PLAYER.width（x/z 同宽）、高 PLAYER.height。
 * - 输入可以是：
 *   a) 按键集合/数组，如 new Set(['KeyW','Space']) / ['ArrowUp'] / ['w'];
 *   b) 布尔动作对象，如 { forward: true, right: false, jump: true };
 *   c) 可选附带相机水平朝向 { yaw } / { cameraYaw }（弧度），默认 yaw=0 面向 +Z。
 * - world 接口宽容：可传数组，也可传 { platforms, obstacles, movingPlatforms, solids, colliders }
 *   对象；每个碰撞体支持 { position, size } 或 { min, max }，position 为 AABB 中心，size 为整尺寸。
 *   还可以传 { groundY } 表示地面高度、{ fallThresholdY } 表示掉落阈值。
 */

export const PLAYER = Object.freeze({
  spawn: Object.freeze({ x: 0, y: 0.25, z: 0 }),
  width: 0.7,
  height: 1.6,
  moveSpeed: 6,
  maxSpeed: 9,
  acceleration: 60,
  deceleration: 60,
  gravity: -24,
  jumpSpeed: 8,
  maxFallSpeed: -28,
  fallThresholdY: -12,
  stepHeight: 0.35,
  skin: 0.0001,
});

export const PLAYER_CONFIG = PLAYER;

const KEY_MAP = {
  forward: ['KeyW', 'ArrowUp', 'w', 'W', 'up', 'forward'],
  backward: ['KeyS', 'ArrowDown', 's', 'S', 'down', 'backward', 'back'],
  left: ['KeyA', 'ArrowLeft', 'a', 'A', 'left'],
  right: ['KeyD', 'ArrowRight', 'd', 'D', 'right'],
  jump: ['Space', ' ', 'space', 'Spacebar', 'jump'],
};

/** 返回“动作 -> 可识别按键”的映射（每次返回新副本）。 */
export function getPlayerKeyMap() {
  const map = {};
  for (const action of Object.keys(KEY_MAP)) map[action] = KEY_MAP[action].slice();
  return map;
}

function clampNumber(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function clampDt(dt) {
  const value = Number(dt);
  if (!Number.isFinite(value) || value <= 0) return 1 / 60;
  return Math.min(value, 0.1);
}

function toVec3(value, fallback = null) {
  if (value === undefined || value === null) return fallback;
  if (Array.isArray(value)) {
    return {
      x: Number(value[0]) || 0,
      y: Number(value[1]) || 0,
      z: Number(value[2]) || 0,
    };
  }
  if (typeof value === 'object') {
    if (Number.isFinite(value.x) || Number.isFinite(value.y) || Number.isFinite(value.z)) {
      return {
        x: Number.isFinite(value.x) ? Number(value.x) : 0,
        y: Number.isFinite(value.y) ? Number(value.y) : 0,
        z: Number.isFinite(value.z) ? Number(value.z) : 0,
      };
    }
    if (Number.isFinite(value.width) || Number.isFinite(value.height) || Number.isFinite(value.depth)) {
      return {
        x: Number.isFinite(value.width) ? Number(value.width) : 1,
        y: Number.isFinite(value.height) ? Number(value.height) : 1,
        z: Number.isFinite(value.depth) ? Number(value.depth) : 1,
      };
    }
    if (Number.isFinite(value.w) || Number.isFinite(value.h) || Number.isFinite(value.d)) {
      return {
        x: Number.isFinite(value.w) ? Number(value.w) : 1,
        y: Number.isFinite(value.h) ? Number(value.h) : 1,
        z: Number.isFinite(value.d) ? Number(value.d) : 1,
      };
    }
  }
  return fallback;
}

function approach(current, target, maxDelta) {
  const diff = target - current;
  if (Math.abs(diff) <= maxDelta) return target;
  return current + Math.sign(diff) * maxDelta;
}

function createVec3(x = 0, y = 0, z = 0) {
  return { x, y, z };
}

/** 创建玩家状态。spawn 可传 {x,y,z} 或 [x,y,z]，默认 PLAYER.spawn。 */
export function createPlayerState(spawn = PLAYER.spawn) {
  const origin = toVec3(spawn, toVec3(PLAYER.spawn, createVec3()));
  return {
    position: createVec3(origin.x, origin.y, origin.z),
    velocity: createVec3(0, 0, 0),
    spawn: createVec3(origin.x, origin.y, origin.z),
    width: PLAYER.width,
    height: PLAYER.height,
    moveSpeed: PLAYER.moveSpeed,
    maxSpeed: PLAYER.maxSpeed,
    acceleration: PLAYER.acceleration,
    deceleration: PLAYER.deceleration,
    gravity: PLAYER.gravity,
    jumpSpeed: PLAYER.jumpSpeed,
    maxFallSpeed: PLAYER.maxFallSpeed,
    fallThresholdY: PLAYER.fallThresholdY,
    stepHeight: PLAYER.stepHeight,
    yaw: 0,
    grounded: false,
    isFallen: false,
    jumps: 0,
    respawnCount: 0,
    respawned: false,
    lastInput: { forward: 0, right: 0, jump: false },
    lastDt: 0,
  };
}

function ensureState(state) {
  if (!state || typeof state !== 'object') return createPlayerState();
  if (!state.position) state.position = toVec3(state.spawn ?? PLAYER.spawn, createVec3());
  if (!state.velocity) state.velocity = createVec3();
  if (!state.spawn) state.spawn = createVec3(state.position.x, state.position.y, state.position.z);
  for (const key of [
    'width',
    'height',
    'moveSpeed',
    'maxSpeed',
    'acceleration',
    'deceleration',
    'gravity',
    'jumpSpeed',
    'maxFallSpeed',
    'fallThresholdY',
    'stepHeight',
  ]) {
    if (!Number.isFinite(state[key])) state[key] = PLAYER[key];
  }
  if (!Number.isFinite(state.yaw)) state.yaw = 0;
  if (typeof state.grounded !== 'boolean') state.grounded = false;
  if (typeof state.isFallen !== 'boolean') state.isFallen = false;
  if (!Number.isFinite(state.respawnCount)) state.respawnCount = 0;
  if (!Number.isFinite(state.jumps)) state.jumps = 0;
  return state;
}

function hasKey(keySet, action) {
  const keys = KEY_MAP[action];
  for (let i = 0; i < keys.length; i += 1) {
    if (keySet.has(String(keys[i]).toLowerCase())) return true;
  }
  return false;
}

function truthy(value) {
  return value === true || value === 1;
}

/**
 * 把各种输入形式归一化为 { forward, right, jump, yaw? }。
 * forward/right 取值 -1..1；jump 为布尔值。
 */
export function parsePlayerInput(input) {
  const result = { forward: 0, right: 0, jump: false };
  if (!input) return result;

  if (input instanceof Set || Array.isArray(input)) {
    const keySet = new Set(Array.from(input, (key) => String(key).toLowerCase()));
    const forward = hasKey(keySet, 'forward') ? 1 : 0;
    const backward = hasKey(keySet, 'backward') ? 1 : 0;
    const left = hasKey(keySet, 'left') ? 1 : 0;
    const right = hasKey(keySet, 'right') ? 1 : 0;
    result.forward = forward - backward;
    result.right = right - left;
    result.jump = hasKey(keySet, 'jump');
    return result;
  }

  if (typeof input !== 'object') return result;

  if (input.keys !== undefined) {
    const nested = parsePlayerInput(input.keys);
    result.forward += nested.forward;
    result.right += nested.right;
    result.jump = result.jump || nested.jump;
  }

  let forward = 0;
  let backward = 0;
  let left = 0;
  let right = 0;
  if (truthy(input.forward) || truthy(input.up) || truthy(input.w) || truthy(input.KeyW)) forward = 1;
  if (truthy(input.backward) || truthy(input.back) || truthy(input.down) || truthy(input.s) || truthy(input.KeyS)) {
    backward = 1;
  }
  if (truthy(input.left) || truthy(input.a) || truthy(input.KeyA)) left = 1;
  if (truthy(input.right) || truthy(input.d) || truthy(input.KeyD)) right = 1;
  result.forward += forward - backward;
  result.right += right - left;
  result.jump = result.jump || truthy(input.jump) || truthy(input.jumpPressed) || truthy(input.space);
  result.forward = clampNumber(result.forward, -1, 1);
  result.right = clampNumber(result.right, -1, 1);

  const yawCandidates = [input.yaw, input.cameraYaw, input.camera && input.camera.yaw];
  for (const yaw of yawCandidates) {
    if (Number.isFinite(yaw)) {
      result.yaw = yaw;
      break;
    }
  }
  return result;
}

/**
 * 应用输入：计算水平速度（含加速度、阻尼与速度上限），并在允许时起跳。
 * 不移动位置；位置积分与碰撞由 updatePlayer 完成。
 */
export function applyInput(state, keys, dt = 1 / 60) {
  const s = ensureState(state);
  const step = clampDt(dt);
  const input = parsePlayerInput(keys);

  if (Number.isFinite(input.yaw)) s.yaw = input.yaw;
  const yaw = Number.isFinite(s.yaw) ? s.yaw : 0;

  const forward = clampNumber(input.forward, -1, 1);
  const right = clampNumber(input.right, -1, 1);
  const magnitude = Math.hypot(forward, right);
  const maxSpeed = Number.isFinite(s.maxSpeed) ? s.maxSpeed : PLAYER.maxSpeed;

  if (magnitude > 1e-6) {
    // 归一化，保证斜向移动不会比直线更快
    const nf = forward / magnitude;
    const nr = right / magnitude;
    const speed = Number.isFinite(s.moveSpeed) ? s.moveSpeed : PLAYER.moveSpeed;
    const desiredX = (Math.sin(yaw) * nf + Math.cos(yaw) * nr) * speed;
    const desiredZ = (Math.cos(yaw) * nf - Math.sin(yaw) * nr) * speed;
    const accel = (Number.isFinite(s.acceleration) ? s.acceleration : PLAYER.acceleration) * step;
    s.velocity.x = approach(s.velocity.x, desiredX, accel);
    s.velocity.z = approach(s.velocity.z, desiredZ, accel);
  } else {
    const decel = (Number.isFinite(s.deceleration) ? s.deceleration : PLAYER.deceleration) * step;
    s.velocity.x = approach(s.velocity.x, 0, decel);
    s.velocity.z = approach(s.velocity.z, 0, decel);
  }

  const horizontalSpeed = Math.hypot(s.velocity.x, s.velocity.z);
  if (horizontalSpeed > maxSpeed) {
    const scale = maxSpeed / horizontalSpeed;
    s.velocity.x *= scale;
    s.velocity.z *= scale;
  }

  if (input.jump) jump(s);

  s.lastInput = { forward, right, jump: input.jump };
  return s;
}

/** 仅在 grounded 时起跳；滞空重复调用不会产生二段跳。 */
export function jump(state) {
  const s = ensureState(state);
  if (!s.grounded) return false;
  s.velocity.y = Number.isFinite(s.jumpSpeed) ? s.jumpSpeed : PLAYER.jumpSpeed;
  s.grounded = false;
  s.jumps = (Number.isFinite(s.jumps) ? s.jumps : 0) + 1;
  return true;
}

/** 重置到出生点（或指定点），清空速度、跳跃计数，重生计数 +1。 */
export function respawnPlayer(state, spawn) {
  const s = ensureState(state);
  const target = toVec3(spawn ?? s.spawn ?? PLAYER.spawn, toVec3(PLAYER.spawn, createVec3()));
  s.position.x = target.x;
  s.position.y = target.y;
  s.position.z = target.z;
  if (!s.spawn) s.spawn = createVec3(target.x, target.y, target.z);
  s.velocity.x = 0;
  s.velocity.y = 0;
  s.velocity.z = 0;
  s.grounded = false;
  s.isFallen = false;
  s.jumps = 0;
  s.respawnCount = (Number.isFinite(s.respawnCount) ? s.respawnCount : 0) + 1;
  return s;
}

function toBox(entry) {
  if (!entry || typeof entry !== 'object') return null;
  if (entry.solid === false || entry.collidable === false) return null;

  const min = toVec3(entry.min ?? entry.minimum, null);
  const max = toVec3(entry.max ?? entry.maximum, null);
  if (min && max) {
    return {
      minX: Math.min(min.x, max.x),
      maxX: Math.max(min.x, max.x),
      minY: Math.min(min.y, max.y),
      maxY: Math.max(min.y, max.y),
      minZ: Math.min(min.z, max.z),
      maxZ: Math.max(min.z, max.z),
    };
  }

  const position = toVec3(
    entry.position ?? entry.center ?? (entry.mesh && entry.mesh.position) ?? (entry.object && entry.object.position),
    null,
  );
  if (!position) return null;
  const size = toVec3(
    entry.size ??
      entry.dimensions ??
      (entry.userData && entry.userData.size) ??
      (entry.geometry && entry.geometry.parameters) ??
      (entry.scale && Number.isFinite(entry.scale.x) ? entry.scale : null),
    createVec3(1, 1, 1),
  );
  const halfX = Math.abs(size.x) / 2;
  const halfY = Math.abs(size.y) / 2;
  const halfZ = Math.abs(size.z) / 2;
  return {
    minX: position.x - halfX,
    maxX: position.x + halfX,
    minY: position.y - halfY,
    maxY: position.y + halfY,
    minZ: position.z - halfZ,
    maxZ: position.z + halfZ,
  };
}

/** 从世界对象/数组里收集 AABB 碰撞体（接口宽容）。 */
export function getWorldBoxes(world) {
  if (!world) return [];
  if (Array.isArray(world)) {
    return world.map(toBox).filter(Boolean);
  }
  if (typeof world !== 'object') return [];
  const boxes = [];
  const lists = ['platforms', 'obstacles', 'movingPlatforms', 'solids', 'colliders', 'boxes'];
  for (const key of lists) {
    const list = world[key];
    if (Array.isArray(list)) {
      for (const entry of list) {
        const box = toBox(entry);
        if (box) boxes.push(box);
      }
    }
  }
  if (world.world && world.world !== world) {
    for (const box of getWorldBoxes(world.world)) boxes.push(box);
  }
  return boxes;
}

function resolveGroundY(world) {
  if (!world || Array.isArray(world) || typeof world !== 'object') return -Infinity;
  const candidates = [world.groundY, world.groundHeight, world.floorY, world.spec && world.spec.groundY];
  for (const value of candidates) {
    if (Number.isFinite(value)) return value;
  }
  return -Infinity;
}

function resolveFallThreshold(world, state) {
  if (world && !Array.isArray(world) && typeof world === 'object') {
    const candidates = [
      world.fallThresholdY,
      world.fallY,
      world.spec && world.spec.fallThresholdY,
      world.spec && world.spec.fallY,
    ];
    for (const value of candidates) {
      if (Number.isFinite(value)) return value;
    }
  }
  return Number.isFinite(state.fallThresholdY) ? state.fallThresholdY : PLAYER.fallThresholdY;
}

function overlapsXZ(position, box, halfWidth, skin) {
  return (
    position.x + halfWidth > box.minX + skin &&
    position.x - halfWidth < box.maxX - skin &&
    position.z + halfWidth > box.minZ + skin &&
    position.z - halfWidth < box.maxZ - skin
  );
}

function blocksHorizontally(state, box) {
  const feet = state.position.y;
  const head = feet + state.height;
  return box.maxY > feet + state.stepHeight + PLAYER.skin && box.minY < head - PLAYER.skin;
}

function moveHorizontal(state, boxes, step) {
  const s = state;
  const halfWidth = s.width / 2;
  const skin = PLAYER.skin;

  // X 轴
  const dx = s.velocity.x * step;
  if (dx !== 0) {
    let nextX = s.position.x + dx;
    for (const box of boxes) {
      if (!blocksHorizontally(s, box)) continue;
      if (!overlapsXZ({ x: nextX, z: s.position.z }, box, halfWidth, skin)) continue;
      if (dx > 0) nextX = Math.min(nextX, box.minX - halfWidth - skin);
      else nextX = Math.max(nextX, box.maxX + halfWidth + skin);
      s.velocity.x = 0;
    }
    s.position.x = nextX;
  }

  // Z 轴
  const dz = s.velocity.z * step;
  if (dz !== 0) {
    let nextZ = s.position.z + dz;
    for (const box of boxes) {
      if (!blocksHorizontally(s, box)) continue;
      if (!overlapsXZ({ x: s.position.x, z: nextZ }, box, halfWidth, skin)) continue;
      if (dz > 0) nextZ = Math.min(nextZ, box.minZ - halfWidth - skin);
      else nextZ = Math.max(nextZ, box.maxZ + halfWidth + skin);
      s.velocity.z = 0;
    }
    s.position.z = nextZ;
  }
}

/**
 * 物理步进：重力 + 竖直积分 + 平台顶面着陆 / 天花板 + 水平 AABB 阻挡 + 掉落重生。
 * @param {object} state createPlayerState 的返回值
 * @param {object|Array} world 世界对象或碰撞体数组（可传 null）
 * @param {number} dt 帧时间（秒）
 * @param {*} [input] 可选：本帧输入，传入时会先执行 applyInput
 */
export function updatePlayer(state, world, dt = 1 / 60, input) {
  const s = ensureState(state);
  const step = clampDt(dt);
  if (input !== undefined) applyInput(s, input, step);

  const boxes = getWorldBoxes(world);
  const groundY = resolveGroundY(world);
  const fallThresholdY = resolveFallThreshold(world, s);

  const halfWidth = s.width / 2;
  const skin = PLAYER.skin;

  s.grounded = false;
  s.respawned = false;

  const prevY = s.position.y;
  const gravity = Number.isFinite(s.gravity) ? s.gravity : PLAYER.gravity;
  const maxFallSpeed = Number.isFinite(s.maxFallSpeed) ? s.maxFallSpeed : PLAYER.maxFallSpeed;
  s.velocity.y = Math.max(s.velocity.y + gravity * step, maxFallSpeed);
  s.position.y += s.velocity.y * step;

  if (s.velocity.y <= 0) {
    let landingY = -Infinity;
    for (const box of boxes) {
      if (!overlapsXZ(s.position, box, halfWidth, skin)) continue;
      const top = box.maxY;
      const canLand =
        prevY >= top - s.stepHeight - skin && s.position.y <= top + skin && top > landingY;
      if (canLand) landingY = top;
    }
    if (Number.isFinite(groundY) && s.position.y <= groundY && groundY > landingY) {
      landingY = groundY;
    }
    if (landingY > -Infinity) {
      s.position.y = landingY;
      s.velocity.y = 0;
      s.grounded = true;
      s.jumps = 0;
    }
  } else {
    const prevHead = prevY + s.height;
    const nextHead = s.position.y + s.height;
    for (const box of boxes) {
      if (!overlapsXZ(s.position, box, halfWidth, skin)) continue;
      if (prevHead <= box.minY + skin && nextHead > box.minY) {
        s.position.y = box.minY - s.height - skin;
        s.velocity.y = 0;
        break;
      }
    }
  }

  moveHorizontal(s, boxes, step);

  const fell = s.isFallen === true || (Number.isFinite(fallThresholdY) && s.position.y < fallThresholdY);
  if (fell) {
    respawnPlayer(s, s.spawn);
    s.respawned = true;
  }

  s.lastDt = step;
  return s;
}

/** 便捷组合：输入 + 物理一步（无需世界时传 null 即可）。 */
export function stepPlayer(state, input, dt = 1 / 60, world = null) {
  const s = ensureState(state);
  const step = clampDt(dt);
  applyInput(s, input, step);
  return updatePlayer(s, world, step);
}

/** 当前玩家 AABB。 */
export function getPlayerAABB(state) {
  const s = ensureState(state);
  const halfWidth = s.width / 2;
  return {
    minX: s.position.x - halfWidth,
    maxX: s.position.x + halfWidth,
    minY: s.position.y,
    maxY: s.position.y + s.height,
    minZ: s.position.z - halfWidth,
    maxZ: s.position.z + halfWidth,
  };
}

/** 是否满足掉落条件（脚下低于阈值或显式 isFallen）。 */
export function isFallen(state, world = null) {
  const s = ensureState(state);
  if (s.isFallen === true) return true;
  const threshold = resolveFallThreshold(world, s);
  return Number.isFinite(threshold) && s.position.y < threshold;
}
