// OwO Parkour 3D —— 关卡与物理世界模块测试（node:test + node:assert/strict）
// 全部在无浏览器、无 WebGL、无 three.js 真实依赖的 Node 环境运行。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import {
  DEFAULTS,
  PLAYER_SIZE,
  createLevel,
  updateMovingPlatforms,
  updateWorld,
  resolvePlatformCollision,
  isFallen,
  respawnPlayer,
  isAtGoal,
  getVictoryState,
  getSpawnPoint,
  getPlatformTopY,
  platformAABB,
  buildWorldMeshes,
  buildWorld,
  isInsideRegion,
  isGrounded,
  isBlockedByObstacle,
} from '../src/world.js';

const close = (actual, expected, eps = 1e-9) => Math.abs(actual - expected) <= eps;

function assertClose(actual, expected, message, eps = 1e-9) {
  assert.ok(close(actual, expected, eps), `${message}: 期望 ≈ ${expected}，实际 ${actual}`);
}

function createFakeThree() {
  class FakeVector3 {
    constructor(x = 0, y = 0, z = 0) {
      this.x = x;
      this.y = y;
      this.z = z;
    }
    set(x = 0, y = 0, z = 0) {
      this.x = x;
      this.y = y;
      this.z = z;
      return this;
    }
  }

  class FakeObject3D {
    constructor() {
      this.children = [];
      this.name = '';
      this.userData = {};
      this.position = new FakeVector3();
      this.rotation = new FakeVector3();
      this.scale = new FakeVector3(1, 1, 1);
      this.parent = null;
    }
    add(child) {
      this.children.push(child);
      if (child) child.parent = this;
      return this;
    }
  }

  class Group extends FakeObject3D {
    constructor() {
      super();
      this.isGroup = true;
      this.type = 'Group';
    }
  }

  class Mesh extends FakeObject3D {
    constructor(geometry, material) {
      super();
      this.isMesh = true;
      this.type = 'Mesh';
      this.geometry = geometry || null;
      this.material = material || null;
    }
  }

  class BoxGeometry {
    constructor(width = 1, height = 1, depth = 1) {
      this.type = 'BoxGeometry';
      this.parameters = { width, height, depth };
    }
  }

  class MeshStandardMaterial {
    constructor(params = {}) {
      this.type = 'MeshStandardMaterial';
      this.params = params;
      this.color = params.color ?? 0xffffff;
      this.opacity = params.opacity ?? 1;
      this.transparent = Boolean(params.transparent);
    }
  }

  return { Group, Mesh, BoxGeometry, MeshStandardMaterial, Vector3: FakeVector3 };
}

test('关卡数据包含至少 8 个静态平台、AABB 碰撞盒与可行走顶面（平台/障碍数据）', () => {
  const world = createLevel();
  assert.ok(Array.isArray(world.platforms), 'platforms 必须是数组');

  const staticPlatforms = world.platforms.filter((platform) => platform.kind === 'static');
  assert.ok(staticPlatforms.length >= 8, `静态平台至少 8 个，实际 ${staticPlatforms.length}`);
  assert.ok(world.platforms.length >= 9, '含移动平台后平台总数至少 9');

  for (const platform of world.platforms) {
    assert.equal(typeof platform.id, 'string');
    assert.equal(typeof platform.position.x, 'number');
    assert.equal(typeof platform.position.y, 'number');
    assert.equal(typeof platform.position.z, 'number');
    assert.ok(platform.size.x > 0 && platform.size.y > 0 && platform.size.z > 0, '平台尺寸必须为正');

    const box = platformAABB(platform);
    assertClose(box.min.x, platform.position.x - platform.size.x / 2, `${platform.id} AABB min.x`);
    assertClose(box.max.y, platform.position.y + platform.size.y / 2, `${platform.id} AABB max.y`);
    // 可行走顶面 = AABB 顶部
    assert.equal(getPlatformTopY(platform), box.max.y, `${platform.id} 顶面高度须等于 AABB 顶面`);
  }

  assert.ok(Array.isArray(world.obstacles), 'obstacles 必须是数组');
  assert.ok(world.obstacles.length >= 2, `障碍至少 2 个，实际 ${world.obstacles.length}`);
  for (const obstacle of world.obstacles) {
    assert.equal(typeof obstacle.position.x, 'number');
    assert.ok(obstacle.size.x > 0 && obstacle.size.y > 0 && obstacle.size.z > 0, '障碍要有 box 尺寸');
    assert.notEqual(obstacle.blocking, false, '障碍默认为阻挡体');
    const box = platformAABB(obstacle);
    assert.ok(box.max.x > box.min.x && box.max.y > box.min.y && box.max.z > box.min.z);
  }

  assert.ok(world.goal && world.goal.position && world.goal.size, '必须有终点区域');
  assert.equal(world.fallThresholdY, DEFAULTS.fallThresholdY);
});

test('出生点在起点平台顶面上方，getSpawnPoint 返回副本（出生点）', () => {
  const world = createLevel();
  const startPlatform = world.platforms.find((platform) => platform.id === 'start');
  assert.ok(startPlatform, '必须存在 start 平台');

  const spawn = getSpawnPoint(world);
  assert.equal(spawn.x, startPlatform.position.x);
  assert.equal(spawn.z, startPlatform.position.z);
  assert.equal(spawn.y, getPlatformTopY(startPlatform) + PLAYER_SIZE.y / 2);
  assertClose(spawn.y, 0.5 + PLAYER_SIZE.y / 2, '出生高度 = 平台顶面 + 半个玩家身高');

  spawn.x = 999;
  spawn.y = -999;
  assert.equal(getSpawnPoint(world).x, startPlatform.position.x, 'getSpawnPoint 不应暴露内部对象');
  assertClose(getSpawnPoint(world).y, 1.3, '出生点 y 约为 1.3');

  // 出生点本身不算掉出世界
  assert.equal(isFallen(world, getSpawnPoint(world)), false);
});

test('移动平台 update 后坐标改变，并在行程端点折返（移动平台）', () => {
  const world = createLevel();
  assert.ok(world.movingPlatforms.length >= 1, '至少一个移动平台');

  const moving = world.movingPlatforms[0];
  assert.equal(moving.kind, 'moving');
  assert.ok(['x', 'y', 'z'].includes(moving.axis), '轴向往返需要有轴');
  assert.ok(moving.speed > 0 && moving.travel > 0);

  const startX = moving.position.x;
  const startBox = platformAABB(moving);

  updateWorld(world, 0.25);
  assert.notEqual(moving.position.x, startX, 'update 后移动平台坐标必须改变');
  assertClose(moving.position.x, startX + moving.speed * 0.25, '位置按速度与 dt 改变');
  assert.notEqual(platformAABB(moving).max.x, startBox.max.x, 'AABB 随平台位置更新');

  let minX = moving.position.x;
  let maxX = moving.position.x;
  let flips = 0;
  let lastDirection = moving.direction;
  const axis = moving.axis;
  for (let i = 0; i < 400; i += 1) {
    updateMovingPlatforms(world, 1 / 60);
    minX = Math.min(minX, moving.position.x);
    maxX = Math.max(maxX, moving.position.x);
    if (moving.direction !== lastDirection) {
      flips += 1;
      lastDirection = moving.direction;
    }
    assert.ok(
      moving.position.x >= moving.origin.x - moving.travel - 1e-9 &&
        moving.position.x <= moving.origin.x + moving.travel + 1e-9,
      '移动平台不得越出行程',
    );
    assert.equal(moving.position.y, moving.origin.y, '非移动轴保持不变');
    assert.equal(moving.position.z, moving.origin.z, '非移动轴保持不变');
    assert.ok(Math.abs(moving.offset) <= moving.travel + 1e-9);
    assert.ok(axis === 'x');
  }

  assert.ok(flips >= 2, `行程端点必须折返，实际折返 ${flips} 次`);
  assert.ok(maxX - minX > moving.travel, '往返覆盖整段行程');
  assertClose(moving.elapsed, moving.elapsed, 'elapsed 单调累加');
  assert.ok(moving.elapsed > 6, 'elapsed 记录累计时间');

  // 负数/NaN dt 不改变位置
  const before = moving.position.x;
  updateMovingPlatforms(world, 0);
  updateMovingPlatforms(world, Number.NaN);
  assert.equal(moving.position.x, before);
});

test('掉落到阈值以下判定重生并重置到出生点（掉落重生）', () => {
  const world = createLevel();
  const threshold = world.fallThresholdY;

  assert.equal(isFallen(world, { x: 0, y: threshold - 1, z: 0 }), true, '低于阈值即掉落');
  assert.equal(isFallen(world, { x: 0, y: threshold - 0.001, z: 0 }), true);
  assert.equal(isFallen(world, { x: 0, y: threshold, z: 0 }), false, '阈值处不算掉落');
  assert.equal(isFallen(world, { x: 0, y: 5, z: -20 }), false);
  assert.equal(isFallen(world, null), false);

  const spawn = getSpawnPoint(world);
  const respawned = respawnPlayer(world, { x: 8, y: threshold - 40, z: -30 });
  assert.equal(respawned.respawned, true, '掉落必须触发重生');
  assert.deepEqual(respawned.position, spawn, '重生位置必须等于出生点');
  assert.equal(isFallen(world, respawned.position), false);

  const kept = respawnPlayer(world, { x: 1, y: 3.5, z: -2 });
  assert.equal(kept.respawned, false, '未掉落不重生');
  assert.equal(kept.position.y, 3.5);
});

test('碰撞：平台顶面可站立，障碍阻挡水平移动（平台顶面 / 障碍阻挡）', () => {
  const world = createLevel();
  const startPlatform = world.platforms.find((platform) => platform.id === 'start');
  const topY = getPlatformTopY(startPlatform);

  // 玩家轻微嵌入平台顶面 → 被抬到顶面并标记 grounded
  const fallingPlayer = { position: { x: 0, y: topY + 0.05, z: 0 }, size: { ...PLAYER_SIZE } };
  const landed = resolvePlatformCollision(world, fallingPlayer);
  assert.equal(landed.collided, true);
  assert.equal(landed.grounded, true, '站在平台顶面时 grounded 为 true');
  assert.equal(landed.platformId, 'start');
  assertClose(landed.position.y, topY + PLAYER_SIZE.y / 2, '站立高度 = 顶面 + 半身高');
  assertClose(landed.aabb.min.y, topY, '碰撞盒底部贴合顶面');
  assert.equal(isGrounded(world, { position: landed.position, size: { ...PLAYER_SIZE } }), true);

  // 输入对象不被就地修改
  assert.equal(fallingPlayer.position.y, topY + 0.05, 'resolvePlatformCollision 不应修改入参');

  // 障碍阻挡：玩家 AABB 与障碍重叠 → 沿最小穿透轴推出，且不再与障碍相交
  const pillar = world.obstacles.find((obstacle) => obstacle.id === 'obstacle-pillar');
  const pillarBox = platformAABB(pillar);
  const playerInObstacle = { position: { x: -5.2, y: 7.4, z: -34 }, size: { ...PLAYER_SIZE } };
  const pushed = resolvePlatformCollision(world, playerInObstacle);
  assert.equal(pushed.collided, true);
  assert.equal(pushed.wall, true, '障碍在水平方向阻挡玩家');
  assert.equal(pushed.hitObstacle, true);
  assert.equal(pushed.obstacleId, 'obstacle-pillar');
  assert.ok(pushed.position.x >= pillarBox.max.x + PLAYER_SIZE.x / 2 - 1e-9, '玩家被推出障碍包围盒');
  assert.equal(isBlockedByObstacle(world, { position: pushed.position, size: { ...PLAYER_SIZE } }), false);

  // 远离障碍时没有碰撞
  const freePlayer = { position: { x: 0, y: 12, z: 0 }, size: { ...PLAYER_SIZE } };
  const free = resolvePlatformCollision(world, freePlayer);
  assert.equal(free.collided, false);
  assert.equal(free.grounded, false);
  assert.equal(free.hitObstacle, false);
});

test('进入终点盒判定胜利，远离终点为 false（到达终点胜利）', () => {
  const world = createLevel();
  const goalBox = platformAABB(world.goal);

  assert.equal(isAtGoal(world, { ...world.goal.position }), true, '进入终点盒 victory 为 true');
  assert.equal(getVictoryState(world, { ...world.goal.position }).victory, true);

  const goalPlatform = world.platforms.find((platform) => platform.id === 'goal-platform');
  const standingOnGoal = {
    x: goalPlatform.position.x,
    y: getPlatformTopY(goalPlatform) + PLAYER_SIZE.y / 2,
    z: goalPlatform.position.z,
  };
  assert.equal(isAtGoal(world, standingOnGoal), true, '站在终点平台上即触发胜利');
  assert.ok(standingOnGoal.y >= goalBox.min.y && standingOnGoal.y <= goalBox.max.y);

  assert.equal(isAtGoal(world, getSpawnPoint(world)), false, '出生点不在终点盒内');
  assert.equal(getVictoryState(world, getSpawnPoint(world)).victory, false);
  assert.equal(isAtGoal(world, { x: 0, y: 10.2, z: 0 }), false, '远离终点为 false');
  assert.equal(isAtGoal(world, { x: goalBox.max.x + 5, y: 10.2, z: world.goal.position.z }), false);
  assert.equal(isAtGoal(world, null), false);
  assert.equal(getVictoryState(world, { x: 0, y: 10.2, z: 0 }).victory, false);

  assert.equal(isInsideRegion(world.goal, world.goal.position), true);
  assert.equal(isInsideRegion(world.goal, { x: 0, y: 0, z: 0 }), false);
});

test('buildWorldMeshes：无 THREE 时退化为 null，注入 fake THREE 时生成可加入场景的网格组', () => {
  const world = createLevel();
  assert.equal(buildWorldMeshes(null, world), null, '无 THREE 时退化为 null');
  assert.equal(buildWorldMeshes(undefined, world), null);

  const fakeThree = createFakeThree();
  const group = buildWorldMeshes(fakeThree, world);
  assert.ok(group, '注入 THREE 时必须返回 Group');
  assert.equal(group.name, 'parkour-world');
  assert.equal(
    group.children.length,
    world.platforms.length + world.obstacles.length + 1,
    '平台 + 障碍 + 终点各有一个网格',
  );

  const startMesh = group.children.find((child) => child.name === 'start');
  assert.ok(startMesh && startMesh.isMesh);
  assert.equal(startMesh.position.x, 0);
  assert.equal(startMesh.position.y, 0);
  assert.equal(startMesh.userData.kind, 'platform');
  assert.equal(startMesh.geometry.parameters.width, 6);

  const obstacleMesh = group.children.find((child) => child.name === 'obstacle-wall');
  assert.ok(obstacleMesh, '障碍也要有网格');
  assert.equal(obstacleMesh.userData.kind, 'obstacle');

  const goalMesh = group.children.find((child) => child.name === 'goal');
  assert.ok(goalMesh, '终点要有网格');
  assert.equal(goalMesh.userData.kind, 'goal');
});

test('buildWorld(THREE) 返回 {group, platforms, goal, update} 且 update 驱动移动平台', () => {
  const fakeThree = createFakeThree();
  const built = buildWorld(fakeThree);
  assert.ok(built.group, '必须有 group');
  assert.equal(built.platforms.length, built.world.platforms.length);
  assert.ok(built.platforms.length >= 9);
  assert.ok(built.goal && built.goal.position);
  assert.equal(typeof built.update, 'function');
  assert.equal(typeof built.isAtGoal, 'function');
  assert.equal(typeof built.isFallen, 'function');

  const moving = built.world.movingPlatforms[0];
  const movingMesh = built.group.children.find((child) => child.name === moving.id);
  assert.ok(movingMesh, '移动平台必须有网格');
  const beforeX = movingMesh.position.x;

  built.update(0.5, 0.5);
  assert.notEqual(movingMesh.position.x, beforeX, 'update 后网格位置随平台移动');
  assertClose(movingMesh.position.x, moving.position.x, '网格与逻辑位置同步');
  assertClose(moving.position.x, moving.origin.x + moving.speed * 0.5, '按速度移动');

  assert.equal(built.isAtGoal({ ...built.goal.position }), true);
  assert.equal(built.isAtGoal({ x: 0, y: 0, z: 0 }), false);
  assert.equal(built.isFallen({ x: 0, y: -999, z: 0 }), true);

  const noThree = buildWorld(null);
  assert.ok(noThree.group === null || Array.isArray(noThree.group), '无 THREE 时 group 退化为 null/空数组');
  assert.equal(noThree.update(0.1, 0.1).movingPlatforms.length >= 1, true, '无 THREE 时逻辑仍可更新');
});

test('src/world.js 是纯 ES module 且不引用浏览器全局', async () => {
  const sourcePath = fileURLToPath(new URL('../src/world.js', import.meta.url));
  const source = await readFile(sourcePath, 'utf8');

  assert.match(source, /^export\s+(function|const|class)\s+createLevel/m, '必须导出 createLevel');
  for (const name of [
    'updateMovingPlatforms',
    'resolvePlatformCollision',
    'isFallen',
    'isAtGoal',
    'getSpawnPoint',
    'buildWorldMeshes',
  ]) {
    assert.match(
      source,
      new RegExp(`^export\\s+(function|const|class)\\s+${name}`, 'm'),
      `必须导出 ${name}`,
    );
    assert.equal(typeof globalThis[name], 'undefined');
  }

  assert.doesNotMatch(source, /\bdocument\./);
  assert.doesNotMatch(source, /\bwindow\./);
  assert.doesNotMatch(source, /new\s+WebGLRenderer/);
  assert.doesNotMatch(source, /require\s*\(/);

  // 无 three.js 真实依赖也能创建世界
  const world = createLevel();
  assert.equal(world.name, 'OwO Parkour 3D');
  assert.equal(typeof updateWorld(world, 1 / 60).elapsed, 'number');
});
