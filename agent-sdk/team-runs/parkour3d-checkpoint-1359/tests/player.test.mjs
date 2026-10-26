import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  PLAYER,
  createPlayerState,
  applyInput,
  updatePlayer,
  stepPlayer,
  jump,
  respawnPlayer,
  getPlayerKeyMap,
  parsePlayerInput,
  getWorldBoxes,
  getPlayerAABB,
  isFallen,
} from '../src/player.js';

const DT = 1 / 60;
const EPS = 1e-6;
const GROUND_WORLD = { groundY: 0, platforms: [], obstacles: [] };

function run(state, world, seconds, input = null, dt = DT) {
  const frames = Math.max(1, Math.round(seconds / dt));
  for (let i = 0; i < frames; i += 1) {
    applyInput(state, input ?? [], dt);
    updatePlayer(state, world, dt);
  }
  return state;
}

function runPhysics(state, world, seconds, dt = DT) {
  const frames = Math.max(1, Math.round(seconds / dt));
  for (let i = 0; i < frames; i += 1) updatePlayer(state, world, dt);
  return state;
}

test('模块为纯 ESM：import 不触碰浏览器 API，仅导出控制器函数', () => {
  assert.equal(typeof globalThis.window, 'undefined');
  assert.equal(typeof globalThis.document, 'undefined');
  const source = readFileSync(new URL('../src/player.js', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /\b(window|document|requestAnimationFrame)\s*[.(]/);
  for (const name of ['createPlayerState', 'applyInput', 'updatePlayer', 'jump', 'respawnPlayer', 'getPlayerKeyMap']) {
    assert.equal(typeof globalThis[name], 'undefined');
  }
  assert.equal(typeof createPlayerState, 'function');
  assert.equal(typeof applyInput, 'function');
  assert.equal(typeof updatePlayer, 'function');
  assert.equal(typeof jump, 'function');
  assert.equal(typeof respawnPlayer, 'function');
  assert.equal(typeof getPlayerKeyMap, 'function');
});

test('按键映射包含 WASD、方向键与空格', () => {
  const map = getPlayerKeyMap();
  assert.ok(map.forward.includes('KeyW'));
  assert.ok(map.forward.includes('ArrowUp'));
  assert.ok(map.backward.includes('KeyS'));
  assert.ok(map.backward.includes('ArrowDown'));
  assert.ok(map.left.includes('KeyA'));
  assert.ok(map.left.includes('ArrowLeft'));
  assert.ok(map.right.includes('KeyD'));
  assert.ok(map.right.includes('ArrowRight'));
  assert.ok(map.jump.includes('Space'));

  assert.deepEqual(parsePlayerInput(new Set(['KeyW', 'Space'])), {
    forward: 1,
    right: 0,
    jump: true,
  });
  assert.deepEqual(parsePlayerInput({ forward: true, right: true, jump: false }), {
    forward: 1,
    right: 1,
    jump: false,
  });
  assert.deepEqual(parsePlayerInput(['ArrowDown', 'ArrowLeft']), {
    forward: -1,
    right: -1,
    jump: false,
  });
});

test('WASD 与方向键沿预期轴移动，位置变化方向正确', () => {
  const cases = [
    ['KeyW', 'z', 1],
    ['ArrowUp', 'z', 1],
    ['KeyS', 'z', -1],
    ['ArrowDown', 'z', -1],
    ['KeyD', 'x', 1],
    ['ArrowRight', 'x', 1],
    ['KeyA', 'x', -1],
    ['ArrowLeft', 'x', -1],
    ['w', 'z', 1],
    ['d', 'x', 1],
  ];
  for (const [key, axis, sign] of cases) {
    const state = createPlayerState({ x: 0, y: 0, z: 0 });
    updatePlayer(state, GROUND_WORLD, DT);
    const start = state.position[axis];
    run(state, GROUND_WORLD, 0.5, [key]);
    const delta = state.position[axis] - start;
    assert.ok(
      delta * sign > 0.3,
      `按键 ${key} 应让 ${axis} 轴按 ${sign} 方向移动，实际 delta=${delta}`,
    );
    assert.ok(state.velocity[axis] * sign > 0);
    assert.ok(Math.abs(state.velocity[axis]) <= PLAYER.maxSpeed + EPS);
  }
});

test('无输入时速度归零/阻尼，位置不再漂移', () => {
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, GROUND_WORLD, DT);
  run(state, GROUND_WORLD, 0.5, ['KeyW']);
  assert.ok(Math.hypot(state.velocity.x, state.velocity.z) > 1);

  run(state, GROUND_WORLD, 0.5, null);
  assert.equal(state.velocity.x, 0);
  assert.equal(state.velocity.z, 0);

  const resting = { ...state.position };
  run(state, GROUND_WORLD, 0.5, null);
  assert.equal(state.position.x, resting.x);
  assert.equal(state.position.z, resting.z);
});

test('斜向移动归一化，水平速度不超过上限', () => {
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, GROUND_WORLD, DT);
  run(state, GROUND_WORLD, 1, new Set(['KeyW', 'KeyD']));
  const speed = Math.hypot(state.velocity.x, state.velocity.z);
  assert.ok(speed <= PLAYER.maxSpeed + EPS);
  assert.ok(Math.abs(speed - PLAYER.moveSpeed) < 0.01, `斜向速度应归一化到 ${PLAYER.moveSpeed}，实际 ${speed}`);

  // 人为注入超大速度后仍被钳制
  const fast = createPlayerState({ x: 0, y: 0, z: 0 });
  fast.velocity.x = 500;
  fast.velocity.z = -500;
  applyInput(fast, [], DT);
  assert.ok(Math.hypot(fast.velocity.x, fast.velocity.z) <= PLAYER.maxSpeed + EPS);
});

test('空格跳跃：起跳后 y 上升且 grounded=false，空中不可二段跳，落地后 grounded=true', () => {
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, GROUND_WORLD, DT);
  assert.equal(state.grounded, true);

  applyInput(state, new Set(['Space']), DT);
  assert.ok(state.velocity.y > 0, '起跳瞬间竖直速度应大于 0');
  assert.equal(state.grounded, false);
  const firstJumpVelocity = state.velocity.y;

  updatePlayer(state, GROUND_WORLD, DT);
  assert.ok(state.position.y > 0, '起跳后 y 应上升');
  const airborneVelocity = state.velocity.y;

  // 空中再次按空格：不得重置竖直速度（无二段跳）
  applyInput(state, { jump: true }, DT);
  assert.equal(state.velocity.y, airborneVelocity);
  assert.ok(state.velocity.y < firstJumpVelocity);

  // 落地
  run(state, GROUND_WORLD, 2, null);
  assert.equal(state.grounded, true);
  assert.ok(Math.abs(state.position.y - 0) < 1e-9);
  assert.equal(state.velocity.y, 0);
  assert.equal(state.jumps, 0);

  // 落地后再次起跳应成功
  assert.equal(jump(state), true);
  assert.ok(state.velocity.y > 0);
});

test('重力下落并稳定停在平台顶面，不穿透平台', () => {
  const platform = {
    id: 'plate',
    position: { x: 0, y: 2, z: 0 },
    size: { x: 6, y: 0.5, z: 6 },
  };
  const topY = platform.position.y + platform.size.y / 2;
  const world = { platforms: [platform] };
  const state = createPlayerState({ x: 0, y: 10, z: 0 });
  assert.equal(state.grounded, false);

  let minY = Infinity;
  for (let i = 0; i < 240; i += 1) {
    updatePlayer(state, world, DT);
    minY = Math.min(minY, state.position.y);
  }
  assert.equal(state.grounded, true);
  assert.ok(Math.abs(state.position.y - topY) < 1e-9, `应停在平台顶面 ${topY}，实际 ${state.position.y}`);
  assert.equal(state.velocity.y, 0);
  assert.ok(minY >= topY - 1e-6, `不应穿透平台，minY=${minY} topY=${topY}`);
});

test('可接收平台数组，且重力按 dt 积分、下落速度有上限', () => {
  const world = [{ position: { x: 0, y: 0, z: 0 }, size: { x: 10, y: 1, z: 10 } }];
  assert.equal(getWorldBoxes(world).length, 1);
  const topY = 0.5;
  const state = createPlayerState({ x: 0, y: 20, z: 0 });
  state.velocity.y = 0;

  updatePlayer(state, world, DT);
  assert.ok(state.velocity.y < 0, '受重力作用应获得向下速度');
  assert.ok(Math.abs(state.velocity.y - PLAYER.gravity * DT) < 1e-9);

  // 从很高处坠落时达到最大下落速度，而不是无限加速
  const falling = createPlayerState({ x: 0, y: 200, z: 0 });
  updatePlayer(falling, null, DT);
  runPhysics(falling, null, 3);
  assert.ok(falling.velocity.y >= PLAYER.maxFallSpeed - 1e-9);
  assert.ok(Math.abs(falling.velocity.y - PLAYER.maxFallSpeed) < 1e-9);
  assert.ok(falling.position.y < 200);

  const landing = createPlayerState({ x: 0, y: 20, z: 0 });
  let minY = Infinity;
  for (let i = 0; i < 300; i += 1) {
    updatePlayer(landing, world, DT);
    minY = Math.min(minY, landing.position.y);
  }
  assert.equal(landing.grounded, true);
  assert.ok(Math.abs(landing.position.y - topY) < 1e-9);
  assert.ok(minY >= topY - 1e-6);
});

test('水平撞向障碍被 AABB 阻挡，不穿透', () => {
  const obstacle = {
    id: 'crate',
    position: { x: 2, y: 2, z: 0 },
    size: { x: 1, y: 4, z: 4 },
  };
  const world = { groundY: 0, obstacles: [obstacle] };
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, world, DT);
  run(state, world, 2, ['KeyD']);

  const obstacleMinX = obstacle.position.x - obstacle.size.x / 2;
  const expectedMaxX = obstacleMinX - PLAYER.width / 2;
  assert.ok(state.position.x > 0.3, '玩家应向右移动');
  assert.ok(
    state.position.x <= expectedMaxX + 1e-3,
    `应被障碍挡在 x<=${expectedMaxX}，实际 ${state.position.x}`,
  );
  assert.equal(state.velocity.x, 0);

  const box = getPlayerAABB(state);
  assert.ok(box.maxX <= obstacleMinX + 1e-3, '玩家 AABB 不应与障碍重叠');

  // 反向移动也应被从另一侧挡住
  const back = createPlayerState({ x: 4, y: 0, z: 0 });
  updatePlayer(back, world, DT);
  run(back, world, 2, ['KeyA']);
  const obstacleMaxX = obstacle.position.x + obstacle.size.x / 2;
  assert.ok(back.position.x >= obstacleMaxX + PLAYER.width / 2 - 1e-3);
});

test('低矮平台可跨上，但高层障碍会阻挡水平移动', () => {
  const world = {
    groundY: 0,
    platforms: [{ position: { x: 1.2, y: 0.15, z: 0 }, size: { x: 1.4, y: 0.3, z: 2 } }],
    obstacles: [{ position: { x: 3, y: 1, z: 0 }, size: { x: 0.5, y: 2, z: 2 } }],
  };
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, world, DT);
  run(state, world, 1.5, ['KeyD']);
  assert.ok(state.position.x > 1.4, `应能越过低矮平台，实际 x=${state.position.x}`);
  assert.ok(state.position.x <= 2.75 - PLAYER.width / 2 + 1e-3);
});

test('掉落到阈值以下自动重生：回到出生点、速度清零、重生计数递增', () => {
  const world = { groundY: -1000, fallThresholdY: -12 };
  const spawn = { x: 1, y: 5, z: 2 };
  const state = createPlayerState(spawn);
  state.position.x = 3;
  state.position.y = -20;
  state.position.z = -7;
  state.velocity.x = 4;
  state.velocity.y = -10;
  state.velocity.z = 3;

  updatePlayer(state, world, DT);
  assert.equal(state.position.x, spawn.x);
  assert.equal(state.position.y, spawn.y);
  assert.equal(state.position.z, spawn.z);
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });
  assert.equal(state.respawnCount, 1);
  assert.equal(state.grounded, false);
  assert.equal(state.respawned, true);

  // isFallen 标记同样触发重生
  state.isFallen = true;
  updatePlayer(state, world, DT);
  assert.equal(state.respawnCount, 2);
  assert.equal(state.isFallen, false);
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });

  // 直接调用 respawnPlayer 也会清空速度与跳跃计数
  state.velocity.x = 9;
  state.jumps = 3;
  respawnPlayer(state, { x: 0, y: 1.2, z: 0 });
  assert.deepEqual(state.position, { x: 0, y: 1.2, z: 0 });
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });
  assert.equal(state.jumps, 0);
  assert.equal(state.respawnCount, 3);
  assert.equal(isFallen(state, world), false);
});

test('stepPlayer 组合输入与物理：向前移动并起跳', () => {
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, GROUND_WORLD, DT);
  assert.equal(state.grounded, true);

  stepPlayer(state, { forward: true, jump: true }, DT, GROUND_WORLD);
  assert.ok(state.velocity.z > 0);
  assert.ok(state.velocity.y > 0);
  assert.equal(state.grounded, false);

  const startZ = state.position.z;
  for (let i = 0; i < 30; i += 1) stepPlayer(state, { forward: true }, DT, GROUND_WORLD);
  assert.ok(state.position.z > startZ);
});

test('支持按相机水平朝向移动（yaw 旋转）', () => {
  const state = createPlayerState({ x: 0, y: 0, z: 0 });
  updatePlayer(state, GROUND_WORLD, DT);
  run(state, GROUND_WORLD, 0.5, { forward: true, yaw: Math.PI / 2 });
  assert.ok(state.position.x > 0.3, `yaw=90° 时前进应沿 +X 移动，实际 x=${state.position.x}`);
  assert.ok(Math.abs(state.position.z) < 0.05);
});
