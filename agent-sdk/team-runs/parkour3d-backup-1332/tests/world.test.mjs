// tests/world.test.mjs — src/world.js 的纯 JS 行为测试（无 DOM、无真实 three.js）
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import {
  WORLD_SPEC,
  buildWorld,
  isInsideRegion,
  isBelowFallThreshold,
  findNearestCheckpoint,
  getRespawnPosition,
  createRespawnState,
  handleFall,
  checkGoal,
} from '../src/world.js';

/* ------------------------------------------------------------------ */
/* 极简 fake three.js：只实现 world.js 声明使用的最小 API 面            */
/* ------------------------------------------------------------------ */
class FakeVector3 {
  constructor(x = 0, y = 0, z = 0) {
    this.x = x;
    this.y = y;
    this.z = z;
  }
  set(x, y, z) {
    this.x = x;
    this.y = y;
    this.z = z;
    return this;
  }
  copy(v) {
    this.x = v.x;
    this.y = v.y;
    this.z = v.z;
    return this;
  }
  clone() {
    return new FakeVector3(this.x, this.y, this.z);
  }
}

class FakeObject3D {
  constructor() {
    this.position = new FakeVector3();
    this.rotation = new FakeVector3();
    this.scale = new FakeVector3(1, 1, 1);
    this.children = [];
    this.userData = {};
    this.visible = true;
    this.name = '';
  }
  add(child) {
    this.children.push(child);
    return this;
  }
  remove(child) {
    const index = this.children.indexOf(child);
    if (index >= 0) this.children.splice(index, 1);
    return this;
  }
}

class FakeMesh extends FakeObject3D {
  constructor(geometry = null, material = null) {
    super();
    this.geometry = geometry;
    this.material = material;
  }
}

class FakeGroup extends FakeObject3D {}

class FakeBoxGeometry {
  constructor(width = 1, height = 1, depth = 1) {
    this.type = 'BoxGeometry';
    this.width = width;
    this.height = height;
    this.depth = depth;
  }
}

class FakeMeshStandardMaterial {
  constructor(params = {}) {
    this.type = 'MeshStandardMaterial';
    Object.assign(this, params);
  }
}

const FAKE_THREE = {
  Vector3: FakeVector3,
  Object3D: FakeObject3D,
  Mesh: FakeMesh,
  Group: FakeGroup,
  BoxGeometry: FakeBoxGeometry,
  MeshStandardMaterial: FakeMeshStandardMaterial,
};

const isSceneObject = (value) => value instanceof FakeMesh || value instanceof FakeGroup;

/* ------------------------------------------------------------------ */
/* 关卡数据                                                            */
/* ------------------------------------------------------------------ */
test('WORLD_SPEC 提供完整的 3D 跑酷关卡数据', () => {
  assert.ok(Array.isArray(WORLD_SPEC.platforms), 'platforms 必须是数组');
  assert.ok(WORLD_SPEC.platforms.length >= 8, '至少 8 段静态平台');
  assert.ok(Array.isArray(WORLD_SPEC.obstacles), 'obstacles 必须是数组');
  assert.ok(WORLD_SPEC.obstacles.length >= 2, '至少 2 个障碍');
  assert.ok(Array.isArray(WORLD_SPEC.movingPlatforms), 'movingPlatforms 必须是数组');
  assert.ok(WORLD_SPEC.movingPlatforms.length >= 1, '至少 1 个移动平台');
  assert.ok(Array.isArray(WORLD_SPEC.checkpoints), 'checkpoints 必须是数组');
  assert.ok(WORLD_SPEC.checkpoints.length >= 2, '至少 2 个重生检查点');
  assert.equal(typeof WORLD_SPEC.fallThresholdY, 'number', '需要数值型掉落阈值');
  assert.ok(WORLD_SPEC.spawn && typeof WORLD_SPEC.spawn.x === 'number');

  for (const platform of WORLD_SPEC.platforms) {
    assert.ok(platform.id, '平台需要 id');
    assert.ok(platform.position && platform.size, '平台需要 position/size');
    assert.ok(platform.size.x > 0 && platform.size.y > 0 && platform.size.z > 0);
  }
  for (const obstacle of WORLD_SPEC.obstacles) {
    assert.ok(obstacle.id, '障碍需要 id');
    assert.ok(obstacle.position && obstacle.size, '障碍需要 position/size');
  }
  for (const mover of WORLD_SPEC.movingPlatforms) {
    assert.ok(mover.id, '移动平台需要 id');
    assert.ok(mover.from && mover.to && mover.size, '移动平台需要 from/to/size');
    assert.ok(mover.speed > 0, '移动平台需要正的速度参数');
  }
  for (const checkpoint of WORLD_SPEC.checkpoints) {
    assert.ok(checkpoint.id, '检查点需要 id');
    assert.ok(checkpoint.position, '检查点需要 position');
  }
  assert.ok(WORLD_SPEC.goal && WORLD_SPEC.goal.position && WORLD_SPEC.goal.size, '需要终点区域');
});

/* ------------------------------------------------------------------ */
/* 场景构建                                                            */
/* ------------------------------------------------------------------ */
test('buildWorld(THREE) 返回可直接渲染的场景描述', () => {
  const world = buildWorld(FAKE_THREE);

  assert.ok(world.group instanceof FakeGroup, 'group 必须是 THREE.Group');
  assert.equal(typeof world.update, 'function', 'update 必须是函数');
  assert.equal(world.spec, WORLD_SPEC, 'spec 暴露关卡数据');

  const expectedCount = WORLD_SPEC.platforms.length + WORLD_SPEC.movingPlatforms.length;
  assert.ok(world.platforms.length >= expectedCount, 'platforms 需包含静态与移动平台');
  for (const mesh of world.platforms) {
    assert.ok(isSceneObject(mesh), '每个平台必须是 Mesh 或 Group');
    assert.ok(mesh.geometry instanceof FakeBoxGeometry, '平台需要有 BoxGeometry');
  }
  const staticMeshes = world.platforms.filter((mesh) => mesh.userData.moving !== true);
  assert.ok(staticMeshes.length >= 3, '至少 3 段静态平台');

  assert.ok(world.obstacles.length >= 2, '至少 2 个障碍');
  for (const mesh of world.obstacles) {
    assert.ok(isSceneObject(mesh), '每个障碍必须是 Mesh 或 Group');
  }

  assert.ok(world.goal && world.goal.mesh, '需要终点 mesh');
  assert.ok(isSceneObject(world.goal.mesh), '终点必须是 Mesh 或 Group');
  assert.ok(typeof world.goal.position.x === 'number' && typeof world.goal.size.x === 'number');

  assert.ok(world.group.children.includes(world.goal.mesh), '终点需加入 group');
  for (const mesh of world.platforms) {
    assert.ok(world.group.children.includes(mesh), '平台需加入 group');
  }
  for (const mesh of world.obstacles) {
    assert.ok(world.group.children.includes(mesh), '障碍需加入 group');
  }
});

test('移动平台位置随传入时间参数变化且沿路径往复运动', () => {
  const world = buildWorld(FAKE_THREE);
  assert.equal(world.movingPlatforms.length, WORLD_SPEC.movingPlatforms.length);

  const moverMesh = world.movingPlatforms[0];
  const moverSpec = WORLD_SPEC.movingPlatforms[0];

  world.update(0, 0);
  assert.equal(moverMesh.position.x, moverSpec.from.x);
  assert.equal(moverMesh.position.y, moverSpec.from.y);
  assert.equal(moverMesh.position.z, moverSpec.from.z);

  const atStart = moverMesh.position.x;
  world.update(0, 1);
  assert.notEqual(moverMesh.position.x, atStart, '位置必须随时间变化');

  // elapsed * speed = 1 → 到达路径另一端
  world.update(0, 1 / moverSpec.speed);
  assert.ok(Math.abs(moverMesh.position.x - moverSpec.to.x) < 1e-9, '应到达路径终点');
  assert.ok(Math.abs(moverMesh.position.y - moverSpec.to.y) < 1e-9);
  assert.ok(Math.abs(moverMesh.position.z - moverSpec.to.z) < 1e-9);

  const minX = Math.min(moverSpec.from.x, moverSpec.to.x);
  const maxX = Math.max(moverSpec.from.x, moverSpec.to.x);
  for (const t of [0, 0.25, 0.75, 1, 1.5, 2, 2.5, 3]) {
    world.update(0, t);
    assert.ok(Number.isFinite(moverMesh.position.x), '位置不能出现 NaN');
    assert.ok(
      moverMesh.position.x >= minX - 1e-9 && moverMesh.position.x <= maxX + 1e-9,
      '移动平台必须留在 from→to 路径上',
    );
  }

  // 第二个移动平台沿 y 轴运动
  const liftMesh = world.movingPlatforms[1];
  const liftSpec = WORLD_SPEC.movingPlatforms[1];
  world.update(0, 0);
  const liftY0 = liftMesh.position.y;
  world.update(0, 1 / liftSpec.speed);
  assert.notEqual(liftMesh.position.y, liftY0, '竖向移动平台应改变高度');

  // 只传 dt 时按累计时间推进
  world.reset();
  world.update(1);
  world.update(1);
  assert.ok(Math.abs(world.movingPlatforms[0].position.x - WORLD_SPEC.movingPlatforms[0].to.x) < 1e-9);
});

/* ------------------------------------------------------------------ */
/* 纯逻辑：掉落 / 重生                                                  */
/* ------------------------------------------------------------------ */
test('isInsideRegion 做轴对齐包围盒判定', () => {
  const center = { x: 0, y: 2, z: 10 };
  const size = { x: 4, y: 2, z: 4 };
  assert.equal(isInsideRegion({ x: 0, y: 2, z: 10 }, center, size), true);
  assert.equal(isInsideRegion({ x: 1.9, y: 2.9, z: 11.9 }, center, size), true);
  assert.equal(isInsideRegion({ x: 2.1, y: 2, z: 10 }, center, size), false);
  assert.equal(isInsideRegion({ x: 0, y: 3.5, z: 10 }, center, size), false);
});

test('玩家低于掉落阈值时返回最近检查点重生位置并累计重生次数', () => {
  const state = createRespawnState();
  assert.equal(state.respawnCount, 0);

  // 站在地面上：不触发掉落
  const safe = handleFall(state, { x: 0, y: 1, z: 0 });
  assert.equal(safe.fell, false);
  assert.equal(safe.event, null);
  assert.equal(safe.respawnCount, 0);
  assert.equal(isBelowFallThreshold({ x: 0, y: 1, z: 0 }), false);

  assert.equal(isBelowFallThreshold({ x: 0, y: WORLD_SPEC.fallThresholdY - 1, z: 0 }), true);

  // 掉到第一个检查点附近
  const first = handleFall(state, { x: 0.5, y: -20, z: 0.5 });
  assert.equal(first.fell, true);
  assert.equal(first.event, 'respawn');
  assert.equal(first.respawnCount, 1);
  assert.deepEqual(first.respawn, WORLD_SPEC.checkpoints[0].position);

  // 掉到后半段：应回到最近的检查点
  const second = handleFall(state, { x: 0, y: -30, z: 30 });
  assert.equal(second.fell, true);
  assert.equal(second.respawnCount, 2, '重生计数递增');
  assert.deepEqual(second.respawn, WORLD_SPEC.checkpoints.at(-1).position);
});

test('findNearestCheckpoint / getRespawnPosition 选择最近的安全点', () => {
  const nearMid = findNearestCheckpoint({ x: 0, y: -10, z: 21 });
  assert.ok(nearMid && nearMid.position);
  const nearestByHand = [...WORLD_SPEC.checkpoints].sort((a, b) => {
    const da = (a.position.x) ** 2 + (a.position.y + 10) ** 2 + (a.position.z - 21) ** 2;
    const db = (b.position.x) ** 2 + (b.position.y + 10) ** 2 + (b.position.z - 21) ** 2;
    return da - db;
  })[0];
  assert.equal(nearMid.id, nearestByHand.id);

  const respawn = getRespawnPosition({ x: 0, y: -10, z: 21 });
  assert.deepEqual(respawn, nearestByHand.position);
  assert.notEqual(respawn, nearestByHand.position, '应返回副本而不是共享引用');

  // 检查点为空时回退到出生点
  const fallback = findNearestCheckpoint({ x: 0, y: -10, z: 0 }, {
    spawn: WORLD_SPEC.spawn,
    fallThresholdY: WORLD_SPEC.fallThresholdY,
    checkpoints: [],
  });
  assert.equal(fallback.id, 'spawn');
  assert.deepEqual(fallback.position, WORLD_SPEC.spawn);
});

/* ------------------------------------------------------------------ */
/* 纯逻辑：终点胜利                                                     */
/* ------------------------------------------------------------------ */
test('进入终点区域返回 victory 事件，未进入返回非胜利结果', () => {
  const goal = WORLD_SPEC.goal;
  const inside = {
    x: goal.position.x,
    y: goal.position.y,
    z: goal.position.z,
  };

  const victory = checkGoal(inside);
  assert.equal(victory.victory, true);
  assert.equal(victory.event, 'victory');
  assert.equal(victory.goalId, goal.id);

  const outside = checkGoal({ x: goal.position.x, y: goal.position.y, z: goal.position.z - 20 });
  assert.equal(outside.victory, false);
  assert.equal(outside.event, null);

  const tooHigh = checkGoal({
    x: goal.position.x,
    y: goal.position.y + goal.size.y,
    z: goal.position.z,
  });
  assert.equal(tooHigh.victory, false);
});

/* ------------------------------------------------------------------ */
/* 模块纯净性                                                          */
/* ------------------------------------------------------------------ */
test('src/world.js 是纯 ES module 且不引用浏览器全局', () => {
  const sourcePath = fileURLToPath(new URL('../src/world.js', import.meta.url));
  const source = readFileSync(sourcePath, 'utf8');
  assert.match(source, /export\s+function\s+buildWorld/, '必须导出 buildWorld');
  assert.match(source, /export\s+const\s+WORLD_SPEC/, '必须导出 WORLD_SPEC');
  for (const banned of ['window', 'document', 'navigator', 'localStorage']) {
    assert.equal(source.includes(banned), false, `world.js 不应引用浏览器全局：${banned}`);
  }
});
