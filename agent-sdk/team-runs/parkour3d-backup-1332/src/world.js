/**
 * src/world.js — 3D 跑酷关卡模块（浏览器原生 ES module）。
 *
 * 设计要点：
 *  - 只依赖调用方传入的 three.js 命名空间（CDN 场景下由 index.html 以 importmap 提供），
 *    模块本身不访问任何宿主环境全局对象，因此在 Node 中可直接用假 THREE 做单元测试。
 *  - 数据与逻辑分离：WORLD_SPEC 是纯数据，纯函数负责掉落判定 / 检查点重生 / 终点胜利判定，
 *    buildWorld(THREE) 只负责把数据变成可渲染的 three.js 对象并驱动移动平台。
 *
 * main.js 用法：
 *   import { buildWorld, handleFall, createRespawnState, checkGoal } from './world.js';
 *   const world = buildWorld(THREE);
 *   scene.add(world.group);
 *   // 每帧：
 *   world.update(dt, elapsed);
 *   const fall = handleFall(respawnState, playerPosition);
 *   const goal = checkGoal(playerPosition);
 */

/** 关卡数据（纯 JSON 风格，便于测试与外部读取） */
export const WORLD_SPEC = {
  spawn: { x: 0, y: 1.2, z: 0 },
  fallThresholdY: -4,
  /** 静态平台：起点 → 间隙 → 桥 → 检查点 → 窄梁 → 抬升段 → 终点平台 */
  platforms: [
    { id: 'start-plate', position: { x: 0, y: 0, z: 0 }, size: { x: 6, y: 0.5, z: 6 }, color: 0x8ecae6 },
    { id: 'gap-plate', position: { x: 0, y: 0.3, z: 8 }, size: { x: 4, y: 0.5, z: 3 }, color: 0x90be6d },
    { id: 'turn-plate', position: { x: 1.5, y: 0.6, z: 12 }, size: { x: 3, y: 0.5, z: 3 }, color: 0x90be6d },
    { id: 'bridge-plate', position: { x: 0, y: 0.8, z: 17 }, size: { x: 3.5, y: 0.5, z: 5 }, color: 0xf9c74f },
    { id: 'checkpoint-plate', position: { x: 0, y: 1.2, z: 21 }, size: { x: 5, y: 0.5, z: 5 }, color: 0x8ecae6 },
    { id: 'narrow-beam', position: { x: -1, y: 1.6, z: 27 }, size: { x: 1.5, y: 0.5, z: 6 }, color: 0xf9c74f },
    { id: 'step-plate', position: { x: 0, y: 2.0, z: 33 }, size: { x: 3, y: 0.5, z: 3 }, color: 0x90be6d },
    { id: 'step-high-plate', position: { x: 0, y: 2.4, z: 39 }, size: { x: 3, y: 0.5, z: 3 }, color: 0x90be6d },
    { id: 'goal-plate', position: { x: 0, y: 3.0, z: 46 }, size: { x: 6, y: 0.5, z: 6 }, color: 0x8ecae6 },
  ],
  /** 静态障碍物：需要绕行或跳过 */
  obstacles: [
    { id: 'crate-a', type: 'box', position: { x: 1.4, y: 1.3, z: 3 }, size: { x: 1.4, y: 1.2, z: 1.4 }, color: 0xef476f },
    { id: 'wall-b', type: 'wall', position: { x: -1.4, y: 1.4, z: 12 }, size: { x: 0.6, y: 1.6, z: 3 }, color: 0xf78c6b },
    { id: 'pillar-c', type: 'pillar', position: { x: 1.0, y: 1.9, z: 24 }, size: { x: 0.8, y: 2.2, z: 0.8 }, color: 0xffd166 },
  ],
  /** 移动平台：在 from → to 之间按速度参数做三角波往复运动 */
  movingPlatforms: [
    {
      id: 'moving-bridge-1',
      from: { x: 0, y: 1.4, z: 30.5 },
      to: { x: 4, y: 1.4, z: 30.5 },
      size: { x: 3, y: 0.4, z: 3 },
      speed: 0.5,
      color: 0x4cc9f0,
    },
    {
      id: 'moving-lift-2',
      from: { x: -2, y: 1.2, z: 36 },
      to: { x: -2, y: 2.6, z: 36 },
      size: { x: 2.4, y: 0.4, z: 2.4 },
      speed: 0.4,
      color: 0x4895ef,
    },
  ],
  /** 终点触发区域（进入即胜利） */
  goal: {
    id: 'goal-finish',
    position: { x: 0, y: 3.0, z: 46 },
    size: { x: 6, y: 3, z: 6 },
    color: 0x38b000,
  },
  /** 安全检查点：重生位置取离掉落点最近的一个 */
  checkpoints: [
    { id: 'checkpoint-start', position: { x: 0, y: 1.2, z: 0 } },
    { id: 'checkpoint-mid', position: { x: 0, y: 1.5, z: 21 } },
    { id: 'checkpoint-upper', position: { x: -1, y: 1.9, z: 27 } },
  ],
};

/* ------------------------------------------------------------------ */
/* 通用小工具                                                          */
/* ------------------------------------------------------------------ */

function toVec3(value) {
  return {
    x: Number.isFinite(value?.x) ? value.x : 0,
    y: Number.isFinite(value?.y) ? value.y : 0,
    z: Number.isFinite(value?.z) ? value.z : 0,
  };
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

/** 三角波：t=0 → 0，t=1 → 1，t=2 → 0，用于把线性时间映射成往复运动 */
function pingPong(t) {
  if (!Number.isFinite(t)) return 0;
  const cycle = Math.abs(t) % 2;
  return cycle <= 1 ? cycle : 2 - cycle;
}

function distanceSq(a, b) {
  const dx = a.x - b.x;
  const dy = a.y - b.y;
  const dz = a.z - b.z;
  return dx * dx + dy * dy + dz * dz;
}

/* ------------------------------------------------------------------ */
/* 纯逻辑                                                              */
/* ------------------------------------------------------------------ */

/** 点是否落在以 center 为中心、size 为边长的轴对齐包围盒内（边界算命中） */
export function isInsideRegion(position, center, size) {
  const p = toVec3(position);
  const c = toVec3(center);
  const s = toVec3(size);
  const eps = 1e-9;
  return (
    Math.abs(p.x - c.x) <= s.x / 2 + eps &&
    Math.abs(p.y - c.y) <= s.y / 2 + eps &&
    Math.abs(p.z - c.z) <= s.z / 2 + eps
  );
}

/** 是否低于掉落阈值（掉出关卡） */
export function isBelowFallThreshold(position, spec = WORLD_SPEC) {
  const p = toVec3(position);
  const threshold = Number.isFinite(spec?.fallThresholdY) ? spec.fallThresholdY : WORLD_SPEC.fallThresholdY;
  return p.y < threshold;
}

/** 找到离当前位置最近的安全检查点；检查点为空时回退到出生点 */
export function findNearestCheckpoint(position, spec = WORLD_SPEC) {
  const p = toVec3(position);
  const checkpoints = Array.isArray(spec?.checkpoints) ? spec.checkpoints : [];
  let best = null;
  let bestDistance = Infinity;
  for (const checkpoint of checkpoints) {
    if (!checkpoint || !checkpoint.position) continue;
    const distance = distanceSq(p, checkpoint.position);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = checkpoint;
    }
  }
  if (best) {
    return { id: best.id ?? 'checkpoint', position: toVec3(best.position) };
  }
  const spawn = toVec3(spec?.spawn ?? WORLD_SPEC.spawn);
  return { id: 'spawn', position: spawn };
}

/** 取最近安全检查点的重生坐标（始终返回副本） */
export function getRespawnPosition(position, spec = WORLD_SPEC) {
  const checkpoint = findNearestCheckpoint(position, spec);
  return { ...checkpoint.position };
}

/** 创建一个重生计数状态对象，供 main.js 每关/每次重开时复用 */
export function createRespawnState(spec = WORLD_SPEC) {
  const checkpoint = findNearestCheckpoint(toVec3(spec?.spawn ?? WORLD_SPEC.spawn), spec);
  return {
    respawnCount: 0,
    lastCheckpointId: checkpoint.id,
    spawn: toVec3(spec?.spawn ?? WORLD_SPEC.spawn),
  };
}

/**
 * 掉落重生判定。
 * 未低于阈值 → { fell: false, event: null }；
 * 低于阈值 → 重生次数 +1，返回最近检查点作为重生位置与 'respawn' 事件。
 */
export function handleFall(state, position, spec = WORLD_SPEC) {
  const respawnState = state ?? createRespawnState(spec);
  if (!isBelowFallThreshold(position, spec)) {
    return {
      fell: false,
      event: null,
      respawn: null,
      respawnCount: respawnState.respawnCount,
      checkpointId: respawnState.lastCheckpointId,
    };
  }
  const checkpoint = findNearestCheckpoint(position, spec);
  respawnState.respawnCount += 1;
  respawnState.lastCheckpointId = checkpoint.id;
  return {
    fell: true,
    event: 'respawn',
    respawn: { ...checkpoint.position },
    respawnCount: respawnState.respawnCount,
    checkpointId: checkpoint.id,
  };
}

/** 终点触发判定：进入终点区域返回 victory 事件 */
export function checkGoal(position, spec = WORLD_SPEC) {
  const goal = spec?.goal ?? WORLD_SPEC.goal;
  const victory = isInsideRegion(position, goal.position, goal.size);
  return victory
    ? { victory: true, event: 'victory', goalId: goal.id ?? 'goal' }
    : { victory: false, event: null, goalId: goal.id ?? 'goal' };
}

/* ------------------------------------------------------------------ */
/* 场景构建                                                            */
/* ------------------------------------------------------------------ */

function createBoxMesh(THREE, config, defaultColor, kind) {
  const size = toVec3(config.size ?? { x: 1, y: 1, z: 1 });
  const position = toVec3(config.position);
  const geometry = new THREE.BoxGeometry(size.x, size.y, size.z);
  const material = new THREE.MeshStandardMaterial({
    color: Number.isFinite(config.color) ? config.color : defaultColor,
    roughness: 0.65,
    metalness: 0.1,
  });
  const mesh = new THREE.Mesh(geometry, material);
  mesh.name = config.id ?? kind;
  mesh.position.set(position.x, position.y, position.z);
  mesh.receiveShadow = true;
  mesh.castShadow = true;
  mesh.userData = {
    kind,
    id: config.id ?? kind,
    size,
    moving: kind === 'moving-platform',
  };
  return mesh;
}

/**
 * 根据 WORLD_SPEC 构建场景描述。
 * @param {object} THREE three.js 命名空间（需包含 Group/Mesh/BoxGeometry/MeshStandardMaterial）
 * @returns {{group: object, platforms: object[], obstacles: object[], movingPlatforms: object[], goal: object, spec: object, update: Function, reset: Function}}
 */
export function buildWorld(THREE, spec = WORLD_SPEC) {
  if (!THREE || typeof THREE.Group !== 'function' || typeof THREE.Mesh !== 'function') {
    throw new TypeError('buildWorld(THREE) 需要传入包含 Group/Mesh 的 three.js 命名空间');
  }

  const group = new THREE.Group();
  group.name = 'parkour-world';

  const platforms = [];
  for (const platform of spec.platforms ?? []) {
    const mesh = createBoxMesh(THREE, platform, 0x8ecae6, 'platform');
    mesh.userData.platformId = platform.id;
    group.add(mesh);
    platforms.push(mesh);
  }

  const obstacles = [];
  for (const obstacle of spec.obstacles ?? []) {
    const mesh = createBoxMesh(THREE, obstacle, 0xef476f, 'obstacle');
    group.add(mesh);
    obstacles.push(mesh);
  }

  const movingPlatforms = [];
  const movers = [];
  for (const mover of spec.movingPlatforms ?? []) {
    const mesh = createBoxMesh(
      THREE,
      { id: mover.id, position: mover.from, size: mover.size, color: mover.color },
      0x4cc9f0,
      'moving-platform',
    );
    mesh.userData.platformId = mover.id;
    mesh.userData.speed = mover.speed;
    mesh.userData.from = toVec3(mover.from);
    mesh.userData.to = toVec3(mover.to);
    group.add(mesh);
    platforms.push(mesh);
    movingPlatforms.push(mesh);
    movers.push({ mesh, from: toVec3(mover.from), to: toVec3(mover.to), speed: mover.speed ?? 0.5 });
  }

  const goalConfig = spec.goal ?? WORLD_SPEC.goal;
  const goalMesh = createBoxMesh(
    THREE,
    { id: goalConfig.id ?? 'goal', position: goalConfig.position, size: goalConfig.size, color: goalConfig.color },
    0x38b000,
    'goal',
  );
  group.add(goalMesh);

  const goal = {
    id: goalConfig.id ?? 'goal',
    mesh: goalMesh,
    position: toVec3(goalConfig.position),
    size: toVec3(goalConfig.size),
  };

  let elapsedTime = 0;

  /** 每帧调用：推进移动平台。elapsed 可选；缺省时用 dt 累加。 */
  function update(dt = 0, elapsed) {
    const safeDt = Number.isFinite(dt) ? dt : 0;
    elapsedTime = Number.isFinite(elapsed) ? elapsed : elapsedTime + safeDt;
    for (const mover of movers) {
      const travel = pingPong(elapsedTime * mover.speed);
      mover.mesh.position.set(
        lerp(mover.from.x, mover.to.x, travel),
        lerp(mover.from.y, mover.to.y, travel),
        lerp(mover.from.z, mover.to.z, travel),
      );
    }
    goalMesh.rotation.y = elapsedTime * 0.6;
    return { elapsed: elapsedTime };
  }

  /** 重开关卡：把时间与移动平台复位到初始位置 */
  function reset() {
    elapsedTime = 0;
    for (const mover of movers) {
      mover.mesh.position.set(mover.from.x, mover.from.y, mover.from.z);
    }
    goalMesh.rotation.y = 0;
    return { elapsed: 0 };
  }

  reset();

  return { group, platforms, obstacles, movingPlatforms, goal, spec, update, reset };
}
