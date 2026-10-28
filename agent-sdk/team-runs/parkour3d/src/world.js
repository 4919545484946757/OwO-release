/**
 * src/world.js — 3D 跑酷关卡世界（关卡数据 + 纯逻辑）
 *
 * 设计要点：
 * - 模块内部 **不** import three.js CDN URL；THREE 由调用方（main.js）注入，因此无 DOM 也能在
 *   Node 下直接 import 测试（测试里用 fake THREE）。
 * - 世界只负责：搭建关卡网格、推进移动平台、把碰撞体积 / 可站立面以数据暴露给 player、
 *   给出「掉落重生」与「到达终点胜利」的判定结果；玩家本身的位移/输入/重置由 player.js 处理。
 *
 * 对外 API：
 *   WORLD_SPEC                        默认关卡定义（数据）
 *   buildLevel(def, THREE)            构建关卡（网格 + 碰撞数据），不推进时间
 *   createWorld(def, THREE)           构建世界对象（含 update / checkGoal / checkFall）
 *   buildWorld(THREE, def)            装配入口：返回 { group, platforms, obstacles, movers, goal, update(dt, elapsed) }
 *   updateWorld(world, dt, elapsed)   每帧推进移动平台（往返路径）
 *   checkGoal(world, position)        终点命中 => { win: true }
 *   checkFall(world, position)        y < killY => { reset: true, respawn: { x, y, z } }
 *   evaluatePlayer(world, position)   统一判定：{ win: true } | { reset: true, respawn } | null
 *   isBlocked(world, point)           该点是否被障碍（实心、不可站立）阻挡
 *   getCollisionData(world)           全部碰撞体（平台 / 移动平台 / 障碍）
 *   getStandableSurfaces(world)       可站立面（平台 + 移动平台）
 *   getNonStandableFaces(world)       不可站立面（障碍，六面皆不可站立）
 */

const AXES = {
  x: { x: 1, y: 0, z: 0 },
  y: { x: 0, y: 1, z: 0 },
  z: { x: 0, y: 0, z: 1 },
};

const COLORS = {
  platform: 0x4f8cff,
  moving: 0xffc857,
  obstacle: 0xff5b6e,
  goal: 0x33e08f,
};

/** 默认关卡：9 个平台（起点 → 台阶 → 终点）、3 个障碍、2 个移动平台、终点触发器、重生点。 */
export const WORLD_SPEC = {
  name: 'sky-run',
  killY: -6,
  spawn: { x: 0, y: 1.4, z: 7.5 },
  respawn: { x: 0, y: 1.4, z: 7.5 },
  goal: {
    id: 'goal',
    position: { x: 0, y: 5.2, z: -52 },
    size: { x: 4, y: 2.4, z: 3 },
  },
  platforms: [
    { id: 'start', position: { x: 0, y: 0, z: 6 }, size: { x: 6, y: 1, z: 6 } },
    { id: 'step-1', position: { x: 0, y: 0, z: -1 }, size: { x: 4, y: 1, z: 4 } },
    { id: 'step-2', position: { x: 2.5, y: 0.6, z: -7 }, size: { x: 4, y: 1, z: 4 } },
    { id: 'step-3', position: { x: -2.5, y: 1.2, z: -13 }, size: { x: 4, y: 1, z: 4 } },
    { id: 'step-4', position: { x: 0, y: 1.8, z: -19 }, size: { x: 5, y: 1, z: 5 } },
    { id: 'step-5', position: { x: 3, y: 2.4, z: -30 }, size: { x: 4, y: 1, z: 4 } },
    { id: 'step-6', position: { x: -3, y: 3, z: -37 }, size: { x: 4, y: 1, z: 4 } },
    { id: 'step-7', position: { x: 0, y: 3.6, z: -44 }, size: { x: 5, y: 1, z: 5 } },
    { id: 'finish', position: { x: 0, y: 4.2, z: -52 }, size: { x: 6, y: 1, z: 6 } },
  ],
  obstacles: [
    { id: 'blocker-1', position: { x: 0, y: 1.1, z: 4.2 }, size: { x: 2, y: 2, z: 1 } },
    { id: 'blocker-2', position: { x: 1.5, y: 2.9, z: -19 }, size: { x: 2, y: 2, z: 1 } },
    { id: 'blocker-3', position: { x: -1.5, y: 4.7, z: -44 }, size: { x: 2, y: 2, z: 1 } },
  ],
  movers: [
    {
      id: 'mover-1',
      position: { x: 1.5, y: 2, z: -24.75 },
      size: { x: 3, y: 0.6, z: 3 },
      axis: 'z',
      amplitude: 3,
      speed: 0.9,
      phase: 0,
    },
    {
      id: 'mover-2',
      position: { x: 0, y: 3.3, z: -41 },
      size: { x: 3, y: 0.6, z: 3 },
      axis: 'x',
      amplitude: 2.5,
      speed: 1.2,
      phase: 0.8,
    },
  ],
};

/* ------------------------------------------------------------------ */
/* 小工具                                                              */
/* ------------------------------------------------------------------ */

function num(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function vec3(source) {
  const p = source || {};
  return { x: num(p.x), y: num(p.y), z: num(p.z) };
}

function dim(value, fallback = 1) {
  const parsed = Math.abs(num(value, fallback));
  return parsed > 0 ? parsed : fallback;
}

function size3(source) {
  const s = source || {};
  return { x: dim(s.x), y: dim(s.y), z: dim(s.z) };
}

function boxFrom(position, size) {
  return {
    min: {
      x: position.x - size.x / 2,
      y: position.y - size.y / 2,
      z: position.z - size.z / 2,
    },
    max: {
      x: position.x + size.x / 2,
      y: position.y + size.y / 2,
      z: position.z + size.z / 2,
    },
  };
}

/** 原地刷新碰撞盒（保持 box 引用稳定，player 可以长期持有）。 */
function refreshBox(target) {
  const box = boxFrom(target.position, target.size);
  target.box.min.x = box.min.x;
  target.box.min.y = box.min.y;
  target.box.min.z = box.min.z;
  target.box.max.x = box.max.x;
  target.box.max.y = box.max.y;
  target.box.max.z = box.max.z;
  return target.box;
}

function pointInBox(box, point, padding = 0) {
  if (!box || !point) return false;
  const p = vec3(point);
  return (
    p.x >= box.min.x - padding &&
    p.x <= box.max.x + padding &&
    p.y >= box.min.y - padding &&
    p.y <= box.max.y + padding &&
    p.z >= box.min.z - padding &&
    p.z <= box.max.z + padding
  );
}

function setMeshPosition(mesh, position) {
  if (!mesh) return;
  if (mesh.position && typeof mesh.position.set === 'function') {
    mesh.position.set(position.x, position.y, position.z);
    return;
  }
  mesh.position = { x: position.x, y: position.y, z: position.z };
}

function createMesh(THREE, options) {
  if (!THREE) return null;
  const { id, type, size, position, color, group } = options;
  const geometry =
    typeof THREE.BoxGeometry === 'function' ? new THREE.BoxGeometry(size.x, size.y, size.z) : null;
  const material =
    typeof THREE.MeshStandardMaterial === 'function'
      ? new THREE.MeshStandardMaterial({ color, roughness: 0.82, metalness: 0.08 })
      : null;
  const mesh = new THREE.Mesh(geometry, material);
  mesh.name = id;
  mesh.userData = { id, type, collider: true };
  setMeshPosition(mesh, position);
  if ('castShadow' in mesh) mesh.castShadow = true;
  if ('receiveShadow' in mesh) mesh.receiveShadow = true;
  if (group && typeof group.add === 'function') group.add(mesh);
  return mesh;
}

function makeCollider(THREE, group, options) {
  const { id, type, position, size, color, standable, solid, moving, moverId } = options;
  const collider = {
    id,
    type,
    position,
    size,
    standable: !!standable,
    solid: solid !== false,
    moving: !!moving,
    moverId: moverId || null,
    box: boxFrom(position, size),
    mesh: null,
  };
  collider.mesh = createMesh(THREE, { id, type, size, position, color, group });
  return collider;
}

function computeBounds(colliders) {
  const min = { x: Infinity, y: Infinity, z: Infinity };
  const max = { x: -Infinity, y: -Infinity, z: -Infinity };
  for (const collider of colliders) {
    min.x = Math.min(min.x, collider.box.min.x);
    min.y = Math.min(min.y, collider.box.min.y);
    min.z = Math.min(min.z, collider.box.min.z);
    max.x = Math.max(max.x, collider.box.max.x);
    max.y = Math.max(max.y, collider.box.max.y);
    max.z = Math.max(max.z, collider.box.max.z);
  }
  if (!Number.isFinite(min.x)) {
    return { min: { x: 0, y: 0, z: 0 }, max: { x: 0, y: 0, z: 0 } };
  }
  return { min, max };
}

function normalizeSpec(def) {
  const input = def && typeof def === 'object' ? def : {};
  const spawn = vec3(input.spawn || WORLD_SPEC.spawn);
  return {
    name: input.name || WORLD_SPEC.name,
    killY: Number.isFinite(input.killY) ? Number(input.killY) : WORLD_SPEC.killY,
    spawn,
    respawn: vec3(input.respawn || input.spawn || WORLD_SPEC.respawn),
    goal: input.goal || WORLD_SPEC.goal,
    platforms: Array.isArray(input.platforms) ? input.platforms : WORLD_SPEC.platforms,
    obstacles: Array.isArray(input.obstacles) ? input.obstacles : WORLD_SPEC.obstacles,
    movers: Array.isArray(input.movers)
      ? input.movers
      : Array.isArray(input.movingPlatforms)
        ? input.movingPlatforms
        : WORLD_SPEC.movers,
  };
}

/* ------------------------------------------------------------------ */
/* 关卡构建                                                            */
/* ------------------------------------------------------------------ */

/**
 * 构建关卡（不推进时间）。
 * @param {object} [def] 关卡定义，默认 WORLD_SPEC
 * @param {object|null} [THREE] three.js 命名空间；为空时只生成数据（无网格）
 */
export function buildLevel(def = WORLD_SPEC, THREE = null) {
  const spec = normalizeSpec(def);
  const group = THREE && typeof THREE.Group === 'function' ? new THREE.Group() : null;
  if (group) group.name = spec.name;

  const platforms = [];
  const obstacles = [];
  const movers = [];
  const colliders = [];

  // 平台（可站立）
  for (const item of spec.platforms) {
    const id = String(item.id || `platform-${platforms.length}`);
    const collider = makeCollider(THREE, group, {
      id,
      type: 'platform',
      position: vec3(item.position),
      size: size3(item.size),
      color: Number.isFinite(item.color) ? item.color : COLORS.platform,
      standable: item.standable !== false,
      solid: true,
    });
    platforms.push(collider.mesh);
    colliders.push(collider);
  }

  // 移动平台（可站立，位置每帧更新）
  for (const item of spec.movers) {
    const axisName = AXES[item.axis] ? item.axis : 'x';
    const axis = AXES[axisName];
    const amplitude = num(item.amplitude, 2);
    const speed = num(item.speed, 1);
    const phase = num(item.phase, 0);
    const position = vec3(item.position);
    const offset0 = amplitude * Math.sin(phase);
    const base = {
      x: position.x - axis.x * offset0,
      y: position.y - axis.y * offset0,
      z: position.z - axis.z * offset0,
    };
    const mover = makeCollider(THREE, group, {
      id: String(item.id || `mover-${movers.length}`),
      type: 'moving-platform',
      position,
      size: size3(item.size),
      color: Number.isFinite(item.color) ? item.color : COLORS.moving,
      standable: true,
      solid: true,
      moving: true,
    });
    mover.moverId = mover.id;
    mover.base = base;
    mover.axis = axisName;
    mover.axisVector = axis;
    mover.amplitude = amplitude;
    mover.speed = speed;
    mover.phase = phase;
    mover.offset = offset0;
    mover.delta = { x: 0, y: 0, z: 0 };
    movers.push(mover);
    colliders.push(mover);
  }

  // 障碍（实心、不可站立）
  for (const item of spec.obstacles) {
    const collider = makeCollider(THREE, group, {
      id: String(item.id || `obstacle-${obstacles.length}`),
      type: 'obstacle',
      position: vec3(item.position),
      size: size3(item.size),
      color: Number.isFinite(item.color) ? item.color : COLORS.obstacle,
      standable: false,
      solid: true,
    });
    obstacles.push(collider.mesh);
    colliders.push(collider);
  }

  // 终点触发器
  const goalSpec = spec.goal || WORLD_SPEC.goal;
  const goalPosition = vec3(goalSpec.position);
  const goalSize = size3(goalSpec.size);
  const goal = {
    id: String(goalSpec.id || 'goal'),
    type: 'goal',
    position: goalPosition,
    size: goalSize,
    box: boxFrom(goalPosition, goalSize),
    mesh: null,
  };
  goal.mesh = createMesh(THREE, {
    id: goal.id,
    type: 'goal',
    size: goalSize,
    position: goalPosition,
    color: Number.isFinite(goalSpec.color) ? goalSpec.color : COLORS.goal,
    group,
  });
  if (goal.mesh && 'material' in goal.mesh && goal.mesh.material) {
    goal.mesh.material.transparent = true;
    goal.mesh.material.opacity = 0.65;
  }
  goal.contains = (point) => pointInBox(goal.box, point);

  return {
    spec,
    group,
    platforms,
    obstacles,
    movers,
    colliders,
    goal,
    spawn: { ...spec.spawn },
    respawn: { ...spec.respawn },
    killY: spec.killY,
    bounds: computeBounds(colliders),
  };
}

/* ------------------------------------------------------------------ */
/* 世界对象 + 每帧推进                                                  */
/* ------------------------------------------------------------------ */

/**
 * 构建可变世界对象（含时间状态与判定方法）。
 * @param {object} [def] 关卡定义
 * @param {object|null} [THREE] three.js 命名空间
 */
export function createWorld(def = WORLD_SPEC, THREE = null) {
  const level = buildLevel(def, THREE);

  const world = {
    ...level,
    time: 0,
    elapsed: 0,
    get movementTime() {
      return world.time;
    },
    update(dt, elapsed) {
      return updateWorld(world, dt, elapsed);
    },
    checkGoal(position) {
      return checkGoal(world, position);
    },
    checkFall(position) {
      return checkFall(world, position);
    },
    evaluatePlayer(position) {
      return evaluatePlayer(world, position);
    },
    isBlocked(point) {
      return isBlocked(world, point);
    },
    getCollisionData() {
      return getCollisionData(world);
    },
    getStandableSurfaces() {
      return getStandableSurfaces(world);
    },
    getNonStandableFaces() {
      return getNonStandableFaces(world);
    },
  };

  return world;
}

function isThreeLike(value) {
  if (!value) return false;
  if (typeof value === 'function') return true;
  if (typeof value !== 'object') return false;
  return typeof value.Group === 'function' || typeof value.Mesh === 'function';
}

function resolveWorldArgs(first, second) {
  if (isThreeLike(first)) return { THREE: first, def: second || WORLD_SPEC };
  if (isThreeLike(second)) return { THREE: second, def: first || WORLD_SPEC };
  return { THREE: null, def: first || second || WORLD_SPEC };
}

/**
 * 装配入口（main.js 使用）：buildWorld(THREE) / buildWorld(THREE, def)。
 * 也兼容 buildWorld(def, THREE)。
 * @returns {{group: any, platforms: any[], obstacles: any[], movers: any[], goal: object, update: Function}}
 */
export function buildWorld(first, second) {
  const { THREE, def } = resolveWorldArgs(first, second);
  return createWorld(def, THREE);
}

/**
 * 每帧推进移动平台：position = base + axis * amplitude * sin(time * speed + phase)。
 * 正弦轨迹天然往返；world.time 累加 dt，保证「按 dt 位移」。
 */
export function updateWorld(world, dt, elapsed) {
  if (!world) return world;
  const step = num(dt, 0);
  world.time = num(world.time, 0) + step;
  world.elapsed = Number.isFinite(elapsed) ? Number(elapsed) : num(world.elapsed, 0) + step;

  const movers = Array.isArray(world.movers) ? world.movers : [];
  for (const mover of movers) {
    const axis = mover.axisVector || AXES[mover.axis] || AXES.x;
    const previous = { x: mover.position.x, y: mover.position.y, z: mover.position.z };
    const offset = num(mover.amplitude, 0) * Math.sin(world.time * num(mover.speed, 0) + num(mover.phase, 0));
    mover.offset = offset;
    mover.position.x = mover.base.x + axis.x * offset;
    mover.position.y = mover.base.y + axis.y * offset;
    mover.position.z = mover.base.z + axis.z * offset;
    mover.delta.x = mover.position.x - previous.x;
    mover.delta.y = mover.position.y - previous.y;
    mover.delta.z = mover.position.z - previous.z;
    refreshBox(mover);
    setMeshPosition(mover.mesh, mover.position);
  }

  return world;
}

/* ------------------------------------------------------------------ */
/* 判定：终点胜利 / 掉落重生                                             */
/* ------------------------------------------------------------------ */

/** 终点区域命中 => { win: true }，否则 null。 */
export function checkGoal(world, position) {
  if (!world || !world.goal || !position) return null;
  if (pointInBox(world.goal.box, position)) return { win: true };
  return null;
}

/** 玩家 y 低于 killY => { reset: true, respawn: { x, y, z } }，否则 null。 */
export function checkFall(world, position) {
  if (!world || !position) return null;
  const killY = Number.isFinite(world.killY) ? Number(world.killY) : WORLD_SPEC.killY;
  if (num(position.y, 0) < killY) {
    return { reset: true, fell: true, respawn: { ...world.respawn } };
  }
  return null;
}

/** 统一判定：先判终点胜利，再判掉落重生，否则 null。 */
export function evaluatePlayer(world, position) {
  const win = checkGoal(world, position);
  if (win) return win;
  return checkFall(world, position);
}

/* ------------------------------------------------------------------ */
/* 对外暴露给 player 的碰撞数据                                          */
/* ------------------------------------------------------------------ */

/** 全部碰撞体：平台（standable）/ 移动平台（standable, moving）/ 障碍（solid, 不可站立）。 */
export function getCollisionData(world) {
  if (!world || !Array.isArray(world.colliders)) return [];
  return world.colliders;
}

/** 可站立面：顶面高度 top + 碰撞盒，供 player 做地面检测。 */
export function getStandableSurfaces(world) {
  return getCollisionData(world)
    .filter((collider) => collider.standable)
    .map((collider) => ({
      id: collider.id,
      type: collider.type,
      moving: !!collider.moving,
      box: collider.box,
      position: collider.position,
      size: collider.size,
      top: collider.box.max.y,
      mesh: collider.mesh,
      collider,
    }));
}

/** 不可站立面：障碍六个面都不能站，供 player 做阻挡/推挤判定。 */
export function getNonStandableFaces(world) {
  return getCollisionData(world)
    .filter((collider) => !collider.standable)
    .map((collider) => ({
      id: collider.id,
      type: collider.type,
      solid: !!collider.solid,
      faces: ['top', 'bottom', '+x', '-x', '+z', '-z'],
      box: collider.box,
      position: collider.position,
      size: collider.size,
      mesh: collider.mesh,
      collider,
    }));
}

/** 该点是否位于某个实心且不可站立的碰撞体（障碍）内部。 */
export function findBlockingCollider(world, point) {
  if (!point) return null;
  for (const collider of getCollisionData(world)) {
    if (!collider.solid || collider.standable) continue;
    if (pointInBox(collider.box, point)) return collider;
  }
  return null;
}

/** 障碍阻挡判定：true 表示该点被阻挡（不可穿过、不可站立）。 */
export function isBlocked(world, point) {
  return findBlockingCollider(world, point) !== null;
}

export default {
  WORLD_SPEC,
  buildLevel,
  createWorld,
  buildWorld,
  updateWorld,
  checkGoal,
  checkFall,
  evaluatePlayer,
  getCollisionData,
  getStandableSurfaces,
  getNonStandableFaces,
  findBlockingCollider,
  isBlocked,
};
