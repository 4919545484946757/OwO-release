/**
 * OwO Parkour 3D —— 关卡与物理世界模块
 *
 * 纯 ES module：
 *  - 不依赖 DOM、不依赖 WebGL、不依赖 three.js 的实现；
 *  - 浏览器里由 main.js 注入 THREE 生成可加入 scene 的网格组；
 *  - Node 测试里注入 fake THREE，或者直接使用纯数据世界。
 *
 * 导出的公开 API：
 *   createLevel()                     创建一份全新的纯数据关卡世界
 *   updateMovingPlatforms(world, dt)  推进移动平台（沿轴往返，端点折返）
 *   updateWorld(world, dt)            推进世界的额外逻辑（移动平台 + elapsed）
 *   resolvePlatformCollision(world, playerAABB) 平台/障碍的 AABB 碰撞求解
 *   isFallen(world, position)         是否低于掉落阈值
 *   respawnPlayer(world, position)    掉落后重置到出生点
 *   isAtGoal(world, position)         是否进入终点盒（victory）
 *   getVictoryState(world, position)  { victory }
 *   getSpawnPoint(world)              出生点副本
 *   buildWorldMeshes(THREE, world)    生成网格组；无 THREE 时返回 null
 *   buildWorld(THREE)                 { group, platforms, obstacles, goal, update(dt, elapsed) }
 */

const EPS = 1e-6;

/** 玩家碰撞盒尺寸（宽 / 高 / 深） */
export const PLAYER_SIZE = Object.freeze({ x: 0.8, y: 1.6, z: 0.8 });

/** 物理与玩法默认参数 */
export const DEFAULTS = Object.freeze({
  gravity: -24,
  moveSpeed: 6,
  jumpSpeed: 9.5,
  maxFallSpeed: -30,
  fallThresholdY: -12,
});

function vec(x = 0, y = 0, z = 0) {
  return { x, y, z };
}

function copyVec(source) {
  return { x: source.x, y: source.y, z: source.z };
}

function num(value, fallback = 0) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

/**
 * 平台 / 障碍统一的轴对齐包围盒（AABB）。
 * 接受 { position, size } 或已经带 min/max 的盒子。
 */
export function platformAABB(box) {
  if (box && box.min && box.max) {
    return { min: copyVec(box.min), max: copyVec(box.max) };
  }
  const position = (box && box.position) || vec();
  const size = (box && box.size) || vec(1, 1, 1);
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

/** 可行走顶面高度：平台顶部 y 坐标 */
export function getPlatformTopY(platform) {
  const position = (platform && platform.position) || vec();
  const size = (platform && platform.size) || vec(1, 1, 1);
  return position.y + size.y / 2;
}

/** 把玩家之类 { position, size } 规整成 AABB（不修改入参） */
export function toAABB(entity) {
  if (!entity) return { min: vec(), max: vec() };
  if (entity.min && entity.max) {
    return { min: copyVec(entity.min), max: copyVec(entity.max) };
  }
  const position = entity.position || vec();
  const size = entity.size || PLAYER_SIZE;
  return {
    min: { x: position.x - size.x / 2, y: position.y - size.y / 2, z: position.z - size.z / 2 },
    max: { x: position.x + size.x / 2, y: position.y + size.y / 2, z: position.z + size.z / 2 },
  };
}

function centerOf(box) {
  return {
    x: (box.min.x + box.max.x) / 2,
    y: (box.min.y + box.max.y) / 2,
    z: (box.min.z + box.max.z) / 2,
  };
}

/** 点是否在盒子（{ position, size } 或 AABB）内部 */
export function isInsideRegion(region, point) {
  if (!region || !point || typeof point !== 'object') return false;
  const box = platformAABB(region);
  const x = num(point.x, Number.NaN);
  const y = num(point.y, Number.NaN);
  const z = num(point.z, Number.NaN);
  if (Number.isNaN(x) || Number.isNaN(y) || Number.isNaN(z)) return false;
  return (
    x >= box.min.x &&
    x <= box.max.x &&
    y >= box.min.y &&
    y <= box.max.y &&
    z >= box.min.z &&
    z <= box.max.z
  );
}

/**
 * 创建一份全新的关卡世界（纯数据，可安全修改；每次调用返回独立副本）。
 * 关卡：起点平台 → 6 段跳台 → 移动浮桥 → 终点平台，共 8 个静态平台 + 1 个移动平台。
 */
export function createLevel() {
  const platforms = [
    { id: 'start', name: '起点平台', kind: 'static', position: vec(0, 0, 0), size: vec(6, 1, 6), color: 0x66bb6a },
    { id: 'step-1', name: '平台 1', kind: 'static', position: vec(0, 1.2, -8), size: vec(4, 1, 4), color: 0x66bb6a },
    { id: 'step-2', name: '平台 2', kind: 'static', position: vec(5, 2.2, -14), size: vec(4, 1, 4), color: 0x42a5f5 },
    { id: 'step-3', name: '平台 3', kind: 'static', position: vec(0, 4.4, -26), size: vec(4, 1, 4), color: 0x42a5f5 },
    { id: 'step-4', name: '平台 4', kind: 'static', position: vec(-6, 5.4, -31), size: vec(4, 1, 4), color: 0x7e57c2 },
    { id: 'step-5', name: '平台 5', kind: 'static', position: vec(-6, 6.6, -38), size: vec(5, 1, 5), color: 0x7e57c2 },
    { id: 'step-6', name: '平台 6', kind: 'static', position: vec(0, 7.6, -44), size: vec(4, 1, 4), color: 0xffa726 },
    { id: 'goal-platform', name: '终点平台', kind: 'static', position: vec(0, 8.6, -50), size: vec(6, 1, 6), color: 0xffd54f },
  ];

  const movingPlatforms = [
    {
      id: 'moving-bridge',
      name: '移动浮桥',
      kind: 'moving',
      position: vec(0, 3.6, -20),
      size: vec(3, 0.6, 3),
      color: 0x00bcd4,
      axis: 'x',
      speed: 2.4,
      travel: 3,
      origin: vec(0, 3.6, -20),
      direction: 1,
      offset: 0,
      elapsed: 0,
    },
  ];
  for (const platform of movingPlatforms) platforms.push(platform);

  const obstacles = [
    {
      id: 'obstacle-wall',
      name: '阻挡墙',
      kind: 'obstacle',
      blocking: true,
      position: vec(0, 2.4, -11),
      size: vec(1.2, 2, 3),
      color: 0xef5350,
    },
    {
      id: 'obstacle-pillar',
      name: '阻挡柱',
      kind: 'obstacle',
      blocking: true,
      position: vec(-6, 7.4, -34),
      size: vec(1.5, 2.5, 1.5),
      color: 0xef5350,
    },
  ];

  const goal = {
    id: 'goal',
    name: '终点',
    position: vec(0, 10.2, -50),
    size: vec(5, 3, 5),
    color: 0xffd54f,
  };

  const startPlatform = platforms[0];
  const spawn = {
    x: startPlatform.position.x,
    y: getPlatformTopY(startPlatform) + PLAYER_SIZE.y / 2,
    z: startPlatform.position.z,
  };

  return {
    name: 'OwO Parkour 3D',
    version: 1,
    gravity: DEFAULTS.gravity,
    moveSpeed: DEFAULTS.moveSpeed,
    jumpSpeed: DEFAULTS.jumpSpeed,
    maxFallSpeed: DEFAULTS.maxFallSpeed,
    playerSize: { x: PLAYER_SIZE.x, y: PLAYER_SIZE.y, z: PLAYER_SIZE.z },
    spawn,
    spawnPoint: { x: spawn.x, y: spawn.y, z: spawn.z },
    fallThresholdY: DEFAULTS.fallThresholdY,
    platforms,
    movingPlatforms,
    obstacles,
    goal,
    elapsed: 0,
  };
}

/**
 * 推进所有移动平台：位置按 speed 与 dt 沿 axis 改变，到达 ±travel 端点折返。
 * 返回传入的 world，方便链式调用。
 */
export function updateMovingPlatforms(world, dt) {
  const level = world || createLevel();
  const step = num(dt, 0);
  if (step <= 0) return level;

  for (const platform of level.movingPlatforms || []) {
    const axis = platform.axis || 'x';
    const speed = num(platform.speed, 2);
    const travel = Math.abs(num(platform.travel, 3));
    if (!platform.origin) platform.origin = copyVec(platform.position);

    let direction = platform.direction === -1 ? -1 : 1;
    let offset = num(platform.offset, platform.position[axis] - platform.origin[axis]) + direction * speed * step;

    // 用三角波反射把越界量折回 [-travel, travel]，实现端点折返
    let guard = 0;
    while ((offset > travel || offset < -travel) && guard < 1000) {
      if (offset > travel) {
        offset = 2 * travel - offset;
        direction = -1;
      } else if (offset < -travel) {
        offset = -2 * travel - offset;
        direction = 1;
      }
      guard += 1;
    }
    if (offset > travel) offset = travel;
    if (offset < -travel) offset = -travel;

    platform.offset = offset;
    platform.direction = direction;
    platform.position[axis] = platform.origin[axis] + offset;
    platform.elapsed = num(platform.elapsed, 0) + step;
  }

  return level;
}

/** 推进世界：移动平台 + 累计时间 */
export function updateWorld(world, dt) {
  const level = world || createLevel();
  updateMovingPlatforms(level, dt);
  level.elapsed = num(level.elapsed, 0) + num(dt, 0);
  return level;
}

function solidBoxesOf(world) {
  const solids = [];
  for (const platform of world.platforms || []) {
    solids.push({
      id: platform.id,
      kind: platform.kind === 'moving' ? 'moving' : 'platform',
      box: platformAABB(platform),
    });
  }
  for (const obstacle of world.obstacles || []) {
    solids.push({ id: obstacle.id, kind: 'obstacle', box: platformAABB(obstacle) });
  }
  return solids;
}

/**
 * 求解玩家 AABB 与平台 / 障碍的碰撞。
 * 做法：对每个重叠实体取最小穿透轴分离（y 轴向上分离即平台顶面站立，其它轴即阻挡）。
 * 不修改入参，返回：
 *   { collided, grounded, ceiling, wall, obstacle, hitObstacle, obstacleId, platformId, position, aabb }
 */
export function resolvePlatformCollision(world, playerAABB) {
  const level = world || createLevel();
  const current = toAABB(playerAABB);
  const result = {
    collided: false,
    grounded: false,
    ceiling: false,
    wall: false,
    obstacle: false,
    hitObstacle: false,
    obstacleId: null,
    platformId: null,
    position: centerOf(current),
    aabb: current,
  };

  for (const solid of solidBoxesOf(level)) {
    const depth = {
      x: Math.min(current.max.x, solid.box.max.x) - Math.max(current.min.x, solid.box.min.x),
      y: Math.min(current.max.y, solid.box.max.y) - Math.max(current.min.y, solid.box.min.y),
      z: Math.min(current.max.z, solid.box.max.z) - Math.max(current.min.z, solid.box.min.z),
    };
    if (depth.x <= EPS || depth.y <= EPS || depth.z <= EPS) continue;

    let axis = 'y';
    if (depth.x <= depth.y && depth.x <= depth.z) axis = 'x';
    else if (depth.z <= depth.y && depth.z <= depth.x) axis = 'z';

    const playerCenter = centerOf(current);
    const solidCenter = centerOf(solid.box);
    const positive = playerCenter[axis] >= solidCenter[axis];
    const delta = positive
      ? solid.box.max[axis] - current.min[axis]
      : solid.box.min[axis] - current.max[axis];
    current.min[axis] += delta;
    current.max[axis] += delta;

    result.collided = true;
    if (axis === 'y') {
      if (positive) {
        result.grounded = true;
        if (solid.kind === 'obstacle') {
          result.obstacle = true;
          result.hitObstacle = true;
          result.obstacleId = solid.id;
        } else {
          result.platformId = solid.id;
        }
      } else {
        result.ceiling = true;
      }
    } else {
      result.wall = true;
      if (solid.kind === 'obstacle') {
        result.obstacle = true;
        result.hitObstacle = true;
        result.obstacleId = solid.id;
      } else if (!result.platformId) {
        result.platformId = solid.id;
      }
    }
  }

  result.position = centerOf(current);
  result.aabb = current;
  return result;
}

/** 玩家 AABB 是否与任意障碍相交（障碍阻挡检测） */
export function isBlockedByObstacle(world, entityAABB) {
  const level = world || createLevel();
  const box = toAABB(entityAABB);
  return (level.obstacles || []).some((obstacle) => {
    const solid = platformAABB(obstacle);
    return (
      Math.min(box.max.x, solid.max.x) - Math.max(box.min.x, solid.min.x) > EPS &&
      Math.min(box.max.y, solid.max.y) - Math.max(box.min.y, solid.min.y) > EPS &&
      Math.min(box.max.z, solid.max.z) - Math.max(box.min.z, solid.min.z) > EPS
    );
  });
}

/** 玩家是否正好站在某个平台顶面上（容差 0.05） */
export function isGrounded(world, playerAABB) {
  const level = world || createLevel();
  const box = toAABB(playerAABB);
  for (const platform of level.platforms || []) {
    const solid = platformAABB(platform);
    const horizontal =
      box.max.x > solid.min.x + EPS &&
      box.min.x < solid.max.x - EPS &&
      box.max.z > solid.min.z + EPS &&
      box.min.z < solid.max.z - EPS;
    if (!horizontal) continue;
    if (Math.abs(box.min.y - solid.max.y) <= 0.05) return true;
  }
  return false;
}

/** 玩家是否掉出世界（y 低于掉落阈值） */
export function isFallen(world, position) {
  const level = world || createLevel();
  const threshold = num(level.fallThresholdY, DEFAULTS.fallThresholdY);
  if (!position) return false;
  const y = typeof position === 'number' ? position : num(position.y, Number.POSITIVE_INFINITY);
  return y < threshold;
}

/** 出生点副本（避免调用方改到关卡数据） */
export function getSpawnPoint(world) {
  const level = world || createLevel();
  const spawn = level.spawn || level.spawnPoint || vec();
  return { x: num(spawn.x), y: num(spawn.y), z: num(spawn.z) };
}

/**
 * 掉落重生：低于阈值则返回 { respawned: true, position: 出生点 }，
 * 否则原样返回当前位置。
 */
export function respawnPlayer(world, position) {
  const level = world || createLevel();
  if (isFallen(level, position)) {
    return { respawned: true, position: getSpawnPoint(level) };
  }
  const current =
    position && typeof position === 'object'
      ? { x: num(position.x), y: num(position.y), z: num(position.z) }
      : getSpawnPoint(level);
  return { respawned: false, position: current };
}

/** 终点盒 AABB */
export function getGoalBox(world) {
  const level = world || createLevel();
  return platformAABB(level.goal || { position: vec(), size: vec(1, 1, 1) });
}

/** 玩家是否进入终点盒（到达终点胜利） */
export function isAtGoal(world, position) {
  const level = world || createLevel();
  if (!position || typeof position !== 'object') return false;
  return isInsideRegion(level.goal, position);
}

/** 终点判定结果：{ victory: true|false } */
export function getVictoryState(world, position) {
  const victory = isAtGoal(world, position);
  return {
    victory,
    reachedGoal: victory,
    position:
      position && typeof position === 'object'
        ? { x: num(position.x), y: num(position.y), z: num(position.z) }
        : null,
  };
}

function createBoxMesh(THREE, item, options = {}) {
  const geometry = new THREE.BoxGeometry(item.size.x, item.size.y, item.size.z);
  const MaterialCtor = THREE.MeshStandardMaterial || THREE.MeshBasicMaterial;
  const opacity = options.opacity === undefined ? 1 : options.opacity;
  const material =
    typeof MaterialCtor === 'function'
      ? new MaterialCtor({
          color: item.color ?? options.color ?? 0xffffff,
          transparent: opacity < 1,
          opacity,
        })
      : null;
  const mesh = material ? new THREE.Mesh(geometry, material) : new THREE.Mesh(geometry);
  mesh.position.set(item.position.x, item.position.y, item.position.z);
  mesh.name = item.id;
  mesh.userData = { ...(mesh.userData || {}), kind: options.kind || 'platform', id: item.id };
  if (options.kind === 'platform' || options.kind === 'moving') {
    mesh.userData.platform = item;
  }
  return mesh;
}

/**
 * 生成可加入 scene 的世界网格组。
 * THREE 缺失（或不含 Group/Mesh/BoxGeometry）时返回 null，退化为纯数据世界。
 */
export function buildWorldMeshes(THREE, world) {
  const level = world || createLevel();
  if (
    !THREE ||
    typeof THREE.Group !== 'function' ||
    typeof THREE.Mesh !== 'function' ||
    typeof THREE.BoxGeometry !== 'function'
  ) {
    return null;
  }

  const group = new THREE.Group();
  group.name = 'parkour-world';
  group.userData = { kind: 'world', id: level.name };

  for (const platform of level.platforms || []) {
    group.add(
      createBoxMesh(THREE, platform, {
        kind: platform.kind === 'moving' ? 'moving' : 'platform',
        opacity: 1,
      }),
    );
  }
  for (const obstacle of level.obstacles || []) {
    group.add(createBoxMesh(THREE, obstacle, { kind: 'obstacle', opacity: 1 }));
  }
  if (level.goal) {
    group.add(createBoxMesh(THREE, level.goal, { kind: 'goal', opacity: 0.35 }));
  }

  return group;
}

function syncMeshes(group, world) {
  if (!group || !Array.isArray(group.children)) return;
  for (const child of group.children) {
    const platform = child && child.userData ? child.userData.platform : null;
    if (platform && child.position && typeof child.position.set === 'function') {
      child.position.set(platform.position.x, platform.position.y, platform.position.z);
    }
  }
}

/**
 * 便捷入口：创建世界 + 网格组。
 * 返回 { world, group, platforms, obstacles, movingPlatforms, goal, spawn, update(dt, elapsed) }。
 * 无 THREE 时 group 为 null，逻辑部分照常可用。
 */
export function buildWorld(THREE) {
  const world = createLevel();
  const group = buildWorldMeshes(THREE, world);

  const built = {
    world,
    group,
    platforms: world.platforms,
    obstacles: world.obstacles,
    movingPlatforms: world.movingPlatforms,
    goal: world.goal,
    spawn: world.spawn,
    update(dt, elapsed) {
      updateMovingPlatforms(world, dt);
      world.elapsed =
        typeof elapsed === 'number' && Number.isFinite(elapsed)
          ? elapsed
          : num(world.elapsed, 0) + num(dt, 0);
      syncMeshes(group, world);
      return world;
    },
    resolveCollision(playerAABB) {
      return resolvePlatformCollision(world, playerAABB);
    },
    isAtGoal(position) {
      return isAtGoal(world, position);
    },
    isFallen(position) {
      return isFallen(world, position);
    },
    getSpawnPoint() {
      return getSpawnPoint(world);
    },
  };

  return built;
}

/** 规范化关卡数据快照（只读参考；需要可变世界请调用 createLevel()） */
export const WORLD_SPEC = createLevel();
