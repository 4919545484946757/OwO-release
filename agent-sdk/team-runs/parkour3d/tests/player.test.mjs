// tests/player.test.mjs —— 跑酷玩家控制器的纯逻辑测试（无 DOM / 无 THREE）
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  PLAYER,
  KEY_BINDINGS,
  resolveAction,
  normalizeInput,
  createInputState,
  applyActions,
  inputFromKeys,
  createPlayerState,
  stepPlayer,
  playerBounds,
  resetPlayer,
  createPlayer,
  update,
  getPlayerState,
  syncMesh,
} from '../src/player.js';

const DT = 1 / 60;

// 一块 10x10 的地面，顶面 y = 0
const GROUND = {
  id: 'ground',
  min: { x: -5, y: -0.5, z: -5 },
  max: { x: 5, y: 0, z: 5 },
};

function makeWorld(extra = {}) {
  return {
    platforms: [GROUND],
    fallLimit: -15,
    respawnPoint: { x: 0, y: 0, z: 0 },
    ...extra,
  };
}

function run(state, input, frames, ctx, dt = DT) {
  for (let i = 0; i < frames; i += 1) stepPlayer(state, input, dt, ctx);
  return state;
}

test('PLAYER 配置齐全，player 状态可在无 DOM 环境构造', () => {
  assert.ok(PLAYER.maxSpeed > 0);
  assert.ok(PLAYER.acceleration > 0);
  assert.ok(PLAYER.friction > 0);
  assert.ok(PLAYER.gravity < 0);
  assert.ok(PLAYER.jumpSpeed > 0);
  assert.ok(PLAYER.maxFallSpeed > 0);
  assert.ok(PLAYER.radius > 0);
  assert.ok(PLAYER.height > 0);

  const state = createPlayerState({ respawnPoint: { x: 1, y: 2, z: 3 } });
  assert.deepEqual(state.position, { x: 1, y: 2, z: 3 });
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });
  assert.deepEqual(state.respawnPoint, { x: 1, y: 2, z: 3 });
  assert.equal(state.onGround, false);
  assert.equal(state.standingPlatformId, null);
  assert.equal(state.respawns, 0);

  const bounds = playerBounds(state);
  assert.ok(Math.abs(bounds.max.y - bounds.min.y - PLAYER.height) < 1e-9);
  assert.ok(Math.abs(bounds.max.x - bounds.min.x - PLAYER.radius * 2) < 1e-9);
});

test('WASD / 方向键 / 空格 的事件名到动作映射', () => {
  assert.equal(KEY_BINDINGS.w, 'forward');
  assert.equal(KEY_BINDINGS.arrowup, 'forward');
  assert.equal(KEY_BINDINGS.space, 'jump');

  assert.equal(resolveAction('w'), 'forward');
  assert.equal(resolveAction('W'), 'forward');
  assert.equal(resolveAction('s'), 'back');
  assert.equal(resolveAction('a'), 'left');
  assert.equal(resolveAction('d'), 'right');
  assert.equal(resolveAction('ArrowUp'), 'forward');
  assert.equal(resolveAction('ArrowDown'), 'back');
  assert.equal(resolveAction('ArrowLeft'), 'left');
  assert.equal(resolveAction('ArrowRight'), 'right');
  assert.equal(resolveAction(' '), 'jump');
  assert.equal(resolveAction('Space'), 'jump');
  assert.equal(resolveAction({ key: ' ' }), 'jump');
  assert.equal(resolveAction({ code: 'Space' }), 'jump');
  assert.equal(resolveAction({ code: 'KeyW' }), 'forward');
  assert.equal(resolveAction('Escape'), null);
  assert.equal(resolveAction(null), null);
});

test('输入归一化支持按键集合、Set 与布尔对象', () => {
  assert.deepEqual(normalizeInput(undefined), { forward: 0, right: 0, jump: false });
  assert.deepEqual(normalizeInput(['w', 'ArrowLeft']), { forward: 1, right: -1, jump: false });
  assert.deepEqual(normalizeInput(['w', 's']), { forward: 0, right: 0, jump: false });
  assert.deepEqual(normalizeInput(new Set(['d', ' '])), { forward: 0, right: 1, jump: true });
  assert.deepEqual(normalizeInput({ forward: true, right: false, jump: true }), {
    forward: 1,
    right: 0,
    jump: true,
  });
  assert.deepEqual(inputFromKeys(['ArrowUp', 'a']), { forward: 1, right: -1, jump: false });
  assert.deepEqual(inputFromKeys(['space']), { forward: 0, right: 0, jump: true });

  const input = createInputState();
  assert.deepEqual(input, { forward: 0, right: 0, jump: false });
  applyActions(input, ['a', 'space']);
  assert.deepEqual(input, { forward: 0, right: -1, jump: true });
});

test('WASD 移动产生水平位移（D→+X，W→-Z）', () => {
  const ctx = makeWorld();

  const right = run(createPlayerState(), { forward: 0, right: 1, jump: false }, 30, ctx);
  assert.ok(right.position.x > 0.5, `按 D 应向 +X 位移，实际 x=${right.position.x}`);
  assert.ok(Math.abs(right.position.z) < 1e-6, '按 D 不应有 z 位移');
  assert.ok(Math.abs(right.velocity.x) <= PLAYER.maxSpeed + 1e-6);
  assert.equal(right.onGround, true);

  const forward = run(createPlayerState(), { forward: 1, right: 0, jump: false }, 30, ctx);
  assert.ok(forward.position.z < -0.5, `按 W 应向 -Z 位移，实际 z=${forward.position.z}`);
  assert.ok(Math.abs(forward.position.x) < 1e-6, '按 W 不应有 x 位移');

  const back = run(createPlayerState(), { forward: -1, right: 0, jump: false }, 30, ctx);
  assert.ok(back.position.z > 0.5, '按 S 应向 +Z 位移');

  const left = run(createPlayerState(), { forward: 0, right: -1, jump: false }, 30, ctx);
  assert.ok(left.position.x < -0.5, '按 A 应向 -X 位移');
});

test('方向键与 WASD 走位一致', () => {
  const ctx = makeWorld();
  const viaKeys = run(createPlayerState(), inputFromKeys(['ArrowRight']), 30, ctx);
  const viaWasd = run(createPlayerState(), inputFromKeys(['d']), 30, ctx);
  assert.ok(Math.abs(viaKeys.position.x - viaWasd.position.x) < 1e-9);
  assert.ok(viaKeys.position.x > 0.5);

  const upKeys = run(createPlayerState(), inputFromKeys(['ArrowUp']), 30, ctx);
  const wKeys = run(createPlayerState(), inputFromKeys(['w']), 30, ctx);
  assert.ok(Math.abs(upKeys.position.z - wKeys.position.z) < 1e-9);
  assert.ok(upKeys.position.z < -0.5);
});

test('加速度受最大速度限制，松开按键后摩擦减速停止', () => {
  const ctx = makeWorld();
  const state = run(createPlayerState(), { forward: 0, right: 1, jump: false }, 60, ctx);
  assert.ok(state.velocity.x > 1, `应已加速，实际 vx=${state.velocity.x}`);
  assert.ok(Math.abs(state.velocity.x) <= PLAYER.maxSpeed + 1e-6);

  const speed = state.velocity.x;
  run(state, {}, 120, ctx);
  assert.ok(Math.abs(state.velocity.x) < 1e-6, `摩擦应使水平速度归零，实际 vx=${state.velocity.x}`);
  assert.ok(speed > 0);
});

test('斜向移动速度不超过 PLAYER.maxSpeed', () => {
  const ctx = makeWorld();
  const state = run(createPlayerState(), { forward: 1, right: 1, jump: false }, 120, ctx);
  const speed = Math.hypot(state.velocity.x, state.velocity.z);
  assert.ok(speed <= PLAYER.maxSpeed + 1e-6, `水平速度应被限制，实际 ${speed}`);
  assert.ok(speed > PLAYER.maxSpeed * 0.9);
});

test('空格起跳：获得向上初速度，受重力回落并落地', () => {
  const ctx = makeWorld();
  const state = createPlayerState();
  stepPlayer(state, {}, DT, ctx);
  assert.equal(state.onGround, true);
  assert.equal(state.position.y, 0);

  stepPlayer(state, { jump: true }, DT, ctx);
  assert.ok(state.velocity.y > 0, `起跳速度应向上，实际 vy=${state.velocity.y}`);
  assert.equal(state.onGround, false);

  let apex = state.position.y;
  for (let i = 0; i < 300 && !state.onGround; i += 1) {
    stepPlayer(state, { jump: true }, DT, ctx);
    apex = Math.max(apex, state.position.y);
  }
  assert.ok(apex > 1, `跳跃应达到一定高度，实际 apex=${apex}`);
  assert.equal(state.onGround, true, '应重新落地');
  assert.ok(Math.abs(state.position.y) < 1e-6, `应落回地面高度，实际 y=${state.position.y}`);
  assert.equal(state.velocity.y, 0);
});

test('空中不可二次起跳，落地并松开后可再次起跳', () => {
  const ctx = makeWorld();
  const state = createPlayerState();
  stepPlayer(state, {}, DT, ctx);
  assert.equal(state.onGround, true);

  stepPlayer(state, { jump: true }, DT, ctx);
  const vyAfterJump = state.velocity.y;
  assert.ok(vyAfterJump > 0);

  // 空中松开再按空格：不应获得新的向上速度
  stepPlayer(state, { jump: false }, DT, ctx);
  stepPlayer(state, { jump: true }, DT, ctx);
  assert.ok(state.velocity.y < vyAfterJump, `空中二次起跳被禁止，实际 vy=${state.velocity.y}`);
  assert.equal(state.onGround, false);

  // 一直按住空格直到落地
  for (let i = 0; i < 300 && !state.onGround; i += 1) {
    stepPlayer(state, { jump: true }, DT, ctx);
  }
  assert.equal(state.onGround, true);

  // 按住不放：落地后不会自动起跳
  stepPlayer(state, { jump: true }, DT, ctx);
  assert.equal(state.onGround, true);
  assert.equal(state.velocity.y, 0);

  // 松开再按：可以再次起跳
  stepPlayer(state, { jump: false }, DT, ctx);
  stepPlayer(state, { jump: true }, DT, ctx);
  assert.ok(state.velocity.y > 0, '松开后重新按空格应再次起跳');
  assert.equal(state.onGround, false);
});

test('重力持续加速并将下落速度限制在 maxFallSpeed', () => {
  const ctx = { platforms: [], fallLimit: -1000 };
  const state = createPlayerState();
  stepPlayer(state, {}, DT, ctx);
  assert.ok(state.velocity.y < 0, '自由下落时 vy 为负');
  assert.ok(Math.abs(state.velocity.y - PLAYER.gravity * DT) < 1e-6);

  for (let i = 0; i < 600; i += 1) stepPlayer(state, {}, DT, ctx);
  assert.ok(
    Math.abs(state.velocity.y + PLAYER.maxFallSpeed) < 1e-6,
    `下落速度应被限制，实际 vy=${state.velocity.y}`,
  );
});

test('AABB 水平碰撞阻挡走进墙体', () => {
  const wall = { id: 'wall', min: { x: 1, y: 0, z: -2 }, max: { x: 1.5, y: 3, z: 2 } };
  const ctx = makeWorld({ platforms: [GROUND, wall] });
  const state = run(createPlayerState(), { forward: 0, right: 1, jump: false }, 120, ctx);
  assert.ok(
    state.position.x <= 1 - PLAYER.radius + 1e-6,
    `不应穿过墙体，实际 x=${state.position.x}`,
  );
  assert.ok(state.position.x > 0.5);
  assert.equal(state.velocity.x, 0);
});

test('从上方下落会停在平台顶面并判定站立', () => {
  const mid = { id: 'mid', min: { x: -2, y: 1.5, z: -2 }, max: { x: 2, y: 2, z: 2 } };
  const ctx = { platforms: [mid], fallLimit: -15 };
  const state = createPlayerState({ spawn: { x: 0, y: 5, z: 0 } });
  for (let i = 0; i < 300 && !state.onGround; i += 1) stepPlayer(state, {}, DT, ctx);

  assert.equal(state.onGround, true);
  assert.ok(Math.abs(state.position.y - 2) < 1e-6, `应站在平台顶面，实际 y=${state.position.y}`);
  assert.equal(state.standingPlatformId, 'mid');
  assert.equal(state.velocity.y, 0);
});

test('站立于移动平台时随平台位移', () => {
  const mover = {
    id: 'mover',
    min: { x: -2, y: 0, z: -2 },
    max: { x: 2, y: 0.2, z: 2 },
    delta: { x: 0, y: 0, z: 0 },
  };
  const ctx = { platforms: [mover], fallLimit: -15 };
  const state = createPlayerState({ spawn: { x: 0, y: 0.2, z: 0 } });
  stepPlayer(state, {}, DT, ctx);
  assert.equal(state.onGround, true);
  assert.equal(state.standingPlatformId, 'mover');

  const startX = state.position.x;
  const startZ = state.position.z;
  for (let i = 0; i < 10; i += 1) {
    mover.min.x += 0.05;
    mover.max.x += 0.05;
    mover.delta = { x: 0.05, y: 0, z: 0 };
    stepPlayer(state, {}, DT, ctx);
  }

  assert.ok(
    state.position.x - startX > 0.4,
    `应跟随移动平台位移，实际位移 ${state.position.x - startX}`,
  );
  assert.ok(Math.abs(state.position.z - startZ) < 1e-9);
  assert.ok(Math.abs(state.position.y - 0.2) < 1e-6);
  assert.equal(state.onGround, true);
});

test('掉出边界时调用重生回调并重置到重生点', () => {
  const calls = [];
  const respawnPoint = { x: 2, y: 1, z: -3 };
  const ctx = {
    platforms: [],
    fallLimit: -5,
    respawnPoint,
    onRespawn: (state, info) => calls.push({ position: { ...state.position }, info }),
  };
  const state = createPlayerState({ respawnPoint });

  for (let i = 0; i < 300 && calls.length === 0; i += 1) stepPlayer(state, {}, DT, ctx);

  assert.equal(calls.length, 1, '掉出边界应触发一次重生回调');
  assert.equal(state.respawns, 1);
  assert.deepEqual(state.position, respawnPoint);
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });
  assert.equal(state.onGround, false);
  assert.ok(calls[0].info.from.y < -5, '回调应带上掉落位置信息');

  resetPlayer(state);
  assert.deepEqual(state.position, respawnPoint);
  assert.deepEqual(state.velocity, { x: 0, y: 0, z: 0 });
});

test('createPlayer 的 update / syncMesh 可供 main.js 使用且不依赖 DOM', () => {
  const player = createPlayer({ world: makeWorld() });
  const state = player.update(DT, { right: 1 });
  assert.equal(state, player.state);

  for (let i = 0; i < 30; i += 1) player.update(DT, inputFromKeys(['d', ' ']));
  assert.ok(player.state.position.x > 0.5);

  const mesh = {
    position: {
      x: 0,
      y: 0,
      z: 0,
      set(x, y, z) {
        this.x = x;
        this.y = y;
        this.z = z;
      },
    },
    rotation: { y: 0 },
  };
  syncMesh(mesh, player.state);
  assert.equal(mesh.position.x, player.state.position.x);
  assert.equal(mesh.position.z, player.state.position.z);
  assert.ok(mesh.position.y > player.state.position.y, 'mesh 原点应位于角色中心');

  player.reset();
  assert.deepEqual(player.state.position, { x: 0, y: 0, z: 0 });

  // 模块级默认实例（main.js 可直接 import { update } 使用）
  assert.equal(typeof update, 'function');
  const shared = getPlayerState();
  assert.ok(shared && typeof shared.position.x === 'number');
  assert.equal(update(DT, createInputState()), shared);
});
