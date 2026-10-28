// tests/world.test.mjs
// 先写测试、再实现 src/world.js（node:test + node:assert，纯 JS / fake THREE，不依赖 DOM）。
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  WORLD_SPEC,
  buildWorld,
  buildLevel,
  createWorld,
  updateWorld,
  checkGoal,
  checkFall,
  evaluatePlayer,
  isBlocked,
  getCollisionData,
  getStandableSurfaces,
  getNonStandableFaces,
} from '../src/world.js';

/* ------------------------------------------------------------------ */
/* fake three.js（浏览器里用 CDN 的真实 THREE，Node 测试里用这些替身）        */
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
}

class FakeObject3D {
  constructor() {
    this.children = [];
    this.position = new FakeVector3();
    this.userData = {};
    this.name = '';
  }

  add(child) {
    this.children.push(child);
    return this;
  }
}

class FakeMesh extends FakeObject3D {
  constructor(geometry, material) {
    super();
    this.isMesh = true;
    this.geometry = geometry || null;
    this.material = material || null;
  }
}

class FakeGroup extends FakeObject3D {
  constructor() {
    super();
    this.isGroup = true;
  }
}

class FakeBoxGeometry {
  constructor(width = 1, height = 1, depth = 1) {
    this.width = width;
    this.height = height;
    this.depth = depth;
  }
}

class FakeMaterial {
  constructor(params = {}) {
    Object.assign(this, params);
  }
}

function makeFakeTHREE() {
  return {
    THREE: {
      Group: FakeGroup,
      Mesh: FakeMesh,
      BoxGeometry: FakeBoxGeometry,
      MeshStandardMaterial: FakeMaterial,
      Vector3: FakeVector3,
    },
    FakeMesh,
    FakeGroup,
    FakeBoxGeometry,
  };
}

function close(actual, expected, message) {
  assert.ok(
    Math.abs(actual - expected) < 1e-9,
    `${message || '数值不匹配'}：actual=${actual} expected=${expected}`,
  );
}

/* ------------------------------------------------------------------ */
/* 1. 关卡定义                                                          */
/* ------------------------------------------------------------------ */

test('WORLD_SPEC 至少包含 8 个平台、2 个障碍、1 个移动平台、终点与重生点', () => {
  assert.ok(Array.isArray(WORLD_SPEC.platforms), 'platforms 是数组');
  assert.ok(WORLD_SPEC.platforms.length >= 8, '至少 8 个平台');
  assert.ok(Array.isArray(WORLD_SPEC.obstacles) && WORLD_SPEC.obstacles.length >= 2, '至少 2 个障碍');
  assert.ok(Array.isArray(WORLD_SPEC.movers) && WORLD_SPEC.movers.length >= 1, '至少 1 个移动平台');
  assert.ok(WORLD_SPEC.goal && WORLD_SPEC.goal.position && WORLD_SPEC.goal.size, '终点触发器');
  assert.ok(WORLD_SPEC.respawn && Number.isFinite(WORLD_SPEC.respawn.y), '重生点');
  assert.ok(Number.isFinite(WORLD_SPEC.killY) && WORLD_SPEC.killY < 0, 'killY');

  const ids = new Set(WORLD_SPEC.platforms.map((p) => p.id));
  assert.equal(ids.size, WORLD_SPEC.platforms.length, '平台 id 唯一');
});

/* ------------------------------------------------------------------ */
/* 2. 平台存在性 / 可站立数据                                            */
/* ------------------------------------------------------------------ */

test('buildWorld 返回 { group, platforms, goal, update }，平台可站立数据可读', () => {
  const { THREE, FakeMesh, FakeGroup, FakeBoxGeometry } = makeFakeTHREE();
  const world = buildWorld(THREE);

  assert.ok(world.group instanceof FakeGroup, 'group 是 THREE.Group');
  assert.equal(world.platforms.length, WORLD_SPEC.platforms.length);
  assert.equal(
    world.group.children.length,
    WORLD_SPEC.platforms.length + WORLD_SPEC.obstacles.length + WORLD_SPEC.movers.length + 1,
    'group 包含平台 + 障碍 + 移动平台 + 终点',
  );

  for (const mesh of world.platforms) {
    assert.ok(mesh instanceof FakeMesh, '每个平台都是 THREE.Mesh');
    assert.equal(mesh.userData.collider, true, '平台网格带碰撞标记');
    assert.ok(mesh.geometry instanceof FakeBoxGeometry, '平台有几何体');
    assert.ok(mesh.material, '平台有材质');
  }

  assert.equal(typeof world.update, 'function', 'world.update(dt, elapsed)');
  assert.ok(world.goal && world.goal.box, '终点带触发体积');

  const start = world.colliders.find((c) => c.id === 'start');
  assert.ok(start, '起始平台碰撞体存在');
  assert.equal(start.type, 'platform');
  assert.equal(start.standable, true, '平台可站立');
  assert.deepEqual(start.box, {
    min: { x: -3, y: -0.5, z: 3 },
    max: { x: 3, y: 0.5, z: 9 },
  });

  const surfaces = getStandableSurfaces(world);
  assert.equal(
    surfaces.length,
    WORLD_SPEC.platforms.length + WORLD_SPEC.movers.length,
    '可站立面 = 平台 + 移动平台',
  );
  const startSurface = surfaces.find((s) => s.id === 'start');
  assert.ok(startSurface);
  close(startSurface.top, 0.5, '起始平台顶面高度');

  assert.equal(
    getCollisionData(world).length,
    WORLD_SPEC.platforms.length + WORLD_SPEC.obstacles.length + WORLD_SPEC.movers.length,
    '全部碰撞体',
  );
});

/* ------------------------------------------------------------------ */
/* 3. 障碍阻挡                                                          */
/* ------------------------------------------------------------------ */

test('障碍阻挡判定为真，且障碍面不可站立', () => {
  const { THREE } = makeFakeTHREE();
  const world = buildWorld(THREE);

  const obstacles = getCollisionData(world).filter((c) => c.type === 'obstacle');
  assert.ok(obstacles.length >= 2, '至少 2 个障碍碰撞体');

  const nonStandable = getNonStandableFaces(world);
  assert.equal(nonStandable.length, obstacles.length, '障碍全部是“不可站立面”');

  for (const obstacle of obstacles) {
    assert.equal(obstacle.standable, false);
    assert.equal(obstacle.solid, true);
    assert.equal(
      isBlocked(world, { x: obstacle.position.x, y: obstacle.position.y, z: obstacle.position.z }),
      true,
      `${obstacle.id} 阻挡判定为真`,
    );
    assert.equal(
      isBlocked(world, { x: obstacle.position.x, y: obstacle.box.max.y - 0.001, z: obstacle.position.z }),
      true,
      `${obstacle.id} 顶部不可站立`,
    );
    const faces = nonStandable.find((f) => f.id === obstacle.id);
    assert.ok(faces && faces.faces.length >= 6, '障碍暴露 6 个不可站立面');
  }

  assert.equal(isBlocked(world, { x: 100, y: 100, z: 100 }), false, '空中不算阻挡');
  assert.equal(isBlocked(world, { x: 0, y: 0, z: 6 }), false, '平台可站立不算阻挡');
});

/* ------------------------------------------------------------------ */
/* 4. 移动平台：按 dt 位移 + 往返                                        */
/* ------------------------------------------------------------------ */

test('移动平台按 dt 位移并可往返，网格与碰撞盒同步更新', () => {
  const { THREE } = makeFakeTHREE();
  const def = {
    ...WORLD_SPEC,
    movers: [
      {
        id: 'test-mover',
        position: { x: 0, y: 2, z: 0 },
        size: { x: 2, y: 0.5, z: 2 },
        axis: 'x',
        amplitude: 2,
        speed: 1,
        phase: 0,
      },
    ],
  };
  const world = buildWorld(THREE, def);
  const mover = world.movers[0];

  assert.equal(world.movers.length, 1);
  assert.equal(mover.type, 'moving-platform');
  assert.equal(mover.standable, true, '移动平台可站立');
  close(mover.position.x, 0, '初始位置');

  updateWorld(world, Math.PI / 2);
  close(mover.position.x, 2, '第一个 dt 后的位移');
  close(mover.delta.x, 2, '位移增量');
  close(mover.mesh.position.x, mover.position.x, '网格位置同步');
  close(mover.box.max.x, mover.position.x + 1, '碰撞盒同步');

  updateWorld(world, Math.PI / 2);
  close(mover.position.x, 0, '返程经过中点');

  updateWorld(world, Math.PI / 2);
  close(mover.position.x, -2, '反向极值');

  updateWorld(world, Math.PI / 2);
  close(mover.position.x, 0, '一个完整往返回到起点');

  const t = world.time;
  world.update(Math.PI / 2, 5);
  close(world.time, t + Math.PI / 2, 'world.time 累加 dt');
  close(mover.position.x, 2, 'world.update(dt) 同样推进移动平台');
  close(world.elapsed, 5, 'elapsed 透传');
});

test('默认关卡中的移动平台按周期往返', () => {
  const { THREE } = makeFakeTHREE();
  const world = buildWorld(THREE);
  const mover = world.movers[0];
  const base = { ...mover.base };
  const period = (2 * Math.PI) / mover.speed;

  close(mover.position.z, base.z, 't=0 位于基准点');
  updateWorld(world, period / 4);
  close(mover.position.z, base.z + mover.amplitude, '四分之一周期到达极值');
  close(mover.mesh.position.z, mover.position.z, '网格跟随');
  updateWorld(world, period);
  close(mover.position.z, base.z + mover.amplitude, '整数周期后重复');
  updateWorld(world, period / 4);
  close(mover.position.z, base.z, '半周期后回到基准点');
});

/* ------------------------------------------------------------------ */
/* 5. 掉落重生                                                          */
/* ------------------------------------------------------------------ */

test('玩家 y 低于 killY 时返回重生点与重置信号', () => {
  const { THREE } = makeFakeTHREE();
  const world = buildWorld(THREE);

  assert.equal(checkFall(world, { x: 0, y: world.killY + 1, z: 0 }), null, '高于 killY 不触发');

  const result = checkFall(world, { x: 0, y: world.killY - 0.5, z: 10 });
  assert.ok(result, '低于 killY 触发掉落');
  assert.equal(result.reset, true, '重置信号');
  assert.deepEqual(result.respawn, world.respawn, '返回重生点');

  const evaluated = evaluatePlayer(world, { x: 5, y: -50, z: 5 });
  assert.ok(evaluated && evaluated.reset === true, 'evaluatePlayer 同样返回重置');
  close(evaluated.respawn.x, world.respawn.x, '重生点 x');
  assert.equal(evaluated.win, undefined, '掉落不是胜利');
});

/* ------------------------------------------------------------------ */
/* 6. 终点胜利                                                          */
/* ------------------------------------------------------------------ */

test('进入终点区域返回 { win: true }', () => {
  const { THREE } = makeFakeTHREE();
  const world = buildWorld(THREE);
  const goal = world.goal.position;

  assert.deepEqual(checkGoal(world, { x: goal.x, y: goal.y, z: goal.z }), { win: true }, '终点中心命中');
  assert.deepEqual(
    checkGoal(world, { x: goal.x + 1, y: goal.y, z: goal.z - 1 }),
    { win: true },
    '终点区域内命中',
  );
  assert.equal(checkGoal(world, { x: goal.x, y: goal.y, z: goal.z + 1000 }), null, '终点区域外不胜利');

  const evaluated = evaluatePlayer(world, { x: goal.x, y: goal.y - 0.5, z: goal.z });
  assert.ok(evaluated && evaluated.win === true, 'evaluatePlayer 返回 win');

  assert.equal(evaluatePlayer(world, { x: 0, y: 1.4, z: 7.5 }), null, '起点既不胜利也不重生');
});

/* ------------------------------------------------------------------ */
/* 7. 可被 Node 直接 import，且不依赖 DOM                                */
/* ------------------------------------------------------------------ */

test('模块是纯 ES module，不依赖 DOM，可脱离 THREE 单独构建关卡数据', () => {
  assert.equal(typeof globalThis.document, 'undefined', '测试环境无 DOM');
  assert.equal(typeof globalThis.window, 'undefined', '测试环境无 window');

  const level = buildLevel(WORLD_SPEC, null);
  assert.equal(level.platforms.length, WORLD_SPEC.platforms.length);
  assert.equal(level.group, null, '没有 THREE 时不建 scene graph');
  assert.equal(
    level.colliders.length,
    WORLD_SPEC.platforms.length + WORLD_SPEC.obstacles.length + WORLD_SPEC.movers.length,
  );

  const world = createWorld(WORLD_SPEC, null);
  assert.equal(typeof world.update, 'function');
  assert.equal(typeof world.checkGoal, 'function');
  assert.equal(typeof world.checkFall, 'function');
  updateWorld(world, 0.016);
  assert.ok(world.time > 0);
});
