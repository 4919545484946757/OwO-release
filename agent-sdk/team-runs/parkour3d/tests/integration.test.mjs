// tests/integration.test.mjs
// 集成测试：把 src/world.js + src/player.js + src/ui.js 通过 src/main.js 串起来跑。
// 纯 Node：不访问 DOM、不请求 CDN，three.js 用 fake 替身注入。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { WORLD_SPEC, getCollisionData, evaluatePlayer } from '../src/world.js';
import { PLAYER } from '../src/player.js';
import { CONTROL_HINT, HUD_IDS, formatHud, formatTime, statusLabel } from '../src/ui.js';
import {
  THREE_CDN_URL,
  MAX_DT,
  MAX_RESPAWNS,
  WIN_SCORE,
  FALL_PENALTY,
  loadThree,
  createGame,
  stepGame,
  restartGame,
  resolveWin,
  resetWorldMotion,
  syncGame,
  snapshot,
  updateCamera,
  createPlayerMesh,
  mount,
  boot,
  levelToSolids,
} from '../src/main.js';

/* ------------------------------------------------------------------ */
/* fake three.js                                                       */
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
}

class FakeObject3D {
  constructor() {
    this.children = [];
    this.position = new FakeVector3();
    this.rotation = new FakeVector3();
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

class FakeGroup extends FakeObject3D {}

class FakeBoxGeometry {
  constructor(width = 0, height = 0, depth = 0) {
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

const fakeThree = {
  Vector3: FakeVector3,
  Object3D: FakeObject3D,
  Group: FakeGroup,
  Mesh: FakeMesh,
  BoxGeometry: FakeBoxGeometry,
  MeshStandardMaterial: FakeMaterial,
};

/* ------------------------------------------------------------------ */
/* fake document（HUD 元素）                                            */
/* ------------------------------------------------------------------ */

const STEP = 1 / 60;

function createFakeDocument() {
  const elements = new Map();
  for (const id of Object.values(HUD_IDS)) {
    elements.set(id, {
      id,
      textContent: '',
      classList: {
        set: new Set(),
        toggle(name, on) {
          if (on) this.set.add(name);
          else this.set.delete(name);
        },
        contains(name) {
          return this.set.has(name);
        },
      },
    });
  }
  return {
    elements,
    getElementById(id) {
      return elements.get(id) || null;
    },
  };
}

function frames(game, count, input = null) {
  for (let i = 0; i < count; i += 1) stepGame(game, STEP, input);
  return game;
}

/** 起局并让角色落到起点平台上（start 平台顶面 y = 0.5） */
function groundedGame(options = {}) {
  const game = createGame({
    THREE: options.THREE ?? null,
    documentRef: options.documentRef ?? null,
  });
  frames(game, 40);
  return game;
}

/* ------------------------------------------------------------------ */
/* 1. 入口与 three.js 加载约定                                          */
/* ------------------------------------------------------------------ */

test('integration: main.js 导出集成层 API 且不依赖 DOM / CDN', () => {
  assert.equal(typeof createGame, 'function');
  assert.equal(typeof stepGame, 'function');
  assert.equal(typeof restartGame, 'function');
  assert.equal(typeof mount, 'function');
  assert.equal(typeof boot, 'function');
  assert.equal(typeof loadThree, 'function');
  assert.equal(typeof updateCamera, 'function');
  assert.equal(typeof snapshot, 'function');
  assert.equal(typeof THREE_CDN_URL, 'string');
  assert.match(THREE_CDN_URL, /^https:\/\/unpkg\.com\/three@0\.160\.0\/build\/three\.module\.js$/);
  // Node 下 import 本模块不应触发浏览器自启动
  assert.equal(typeof globalThis.__PARKOUR__, 'undefined');
});

test('integration: index.html 通过 importmap 加载 three.js 并挂载 main.js', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  assert.match(html, /type="importmap"/);
  assert.ok(html.includes(THREE_CDN_URL), 'importmap 的 three 地址应与 main.js 的 THREE_CDN_URL 一致');
  assert.match(html, /"three":\s*"https:\/\/unpkg\.com\/three@0\.160\.0\/build\/three\.module\.js"/);
  assert.match(html, /<script type="module" src="\.\/src\/main\.js"><\/script>/);
  // HUD 需要的元素 id 必须全部存在，否则 ui.createHud 拿到 null
  for (const id of Object.values(HUD_IDS)) {
    assert.ok(html.includes(`id="${id}"`), `index.html 缺少 HUD 元素 #${id}`);
  }
});

/* ------------------------------------------------------------------ */
/* 2. 关卡数据被真正使用（world → main → player）                        */
/* ------------------------------------------------------------------ */

test('integration: createGame 使用 WORLD_SPEC 的关卡数据构建世界与玩家', () => {
  const game = createGame();
  const colliderCount =
    WORLD_SPEC.platforms.length + WORLD_SPEC.movers.length + WORLD_SPEC.obstacles.length;
  assert.equal(game.world.colliders.length, colliderCount);
  assert.deepEqual(game.world.spawn, WORLD_SPEC.spawn);
  assert.deepEqual(game.world.respawn, WORLD_SPEC.respawn);
  assert.equal(game.world.killY, WORLD_SPEC.killY);
  assert.deepEqual(game.world.goal.position, WORLD_SPEC.goal.position);

  // 玩家从关卡 spawn 出生
  assert.deepEqual(game.state.position, WORLD_SPEC.spawn);
  assert.deepEqual(game.state.respawnPoint, WORLD_SPEC.respawn);
  assert.equal(game.hud.status, 'ready');

  // player 上下文来自 world 的碰撞体（含移动平台 id，用于「站在移动平台上跟随」）
  const ids = game.ctx.platforms.map((solid) => solid.id);
  assert.equal(game.ctx.platforms.length, colliderCount);
  assert.ok(ids.includes('start'));
  assert.ok(ids.includes('mover-1'));
  assert.ok(ids.includes('blocker-1'));
  assert.equal(game.ctx.fallLimit, WORLD_SPEC.killY - 3);
  assert.deepEqual(game.ctx.respawnPoint, WORLD_SPEC.respawn);
  // levelToSolids 与 world.getCollisionData 一一对应
  assert.deepEqual(
    levelToSolids(game.world).map((solid) => solid.id),
    getCollisionData(game.world).map((collider) => collider.id),
  );
});

test('integration: 移动平台随 dt 推进，且 delta 传给 player 碰撞数据', () => {
  const game = createGame();
  const mover = game.world.movers[0];
  const before = { ...mover.position };
  assert.equal(mover.delta.z, 0);

  stepGame(game, STEP);
  assert.ok(game.world.time > 0);
  const after = { ...game.world.movers[0].position };
  assert.ok(
    Math.abs(after.z - before.z) > 1e-6 || Math.abs(after.x - before.x) > 1e-6,
    '移动平台位置应随帧推进',
  );
  const solid = game.ctx.platforms.find((item) => item.id === mover.id);
  assert.ok(solid, '移动平台应出现在 player 碰撞数据里');
  assert.ok(solid.delta, '移动平台应带 delta，供 player 跟随');
});

/* ------------------------------------------------------------------ */
/* 3. 重力 / 落地 / 行走 / 跳跃                                         */
/* ------------------------------------------------------------------ */

test('integration: 重力让角色落到起点平台顶面并保持站立', () => {
  const game = createGame();
  assert.ok(game.state.position.y > 1, '出生点在空中');

  frames(game, 40);
  const start = WORLD_SPEC.platforms[0];
  const topY = start.position.y + start.size.y / 2;
  assert.ok(Math.abs(game.state.position.y - topY) < 1e-6, `应站在平台顶面 ${topY}`);
  assert.equal(game.state.onGround, true);
  assert.equal(game.state.standingPlatformId, 'start');
  assert.equal(game.hud.status, 'running', '第一帧推进后从 ready 进入 running');
  assert.ok(game.hud.time > 0);
});

test('integration: 前进键推动角色位移，并被关卡障碍阻挡', () => {
  const game = groundedGame();
  const startZ = game.state.position.z;
  frames(game, 60, { forward: 1 });

  assert.ok(game.state.position.z < startZ - 1, '角色应向 -Z 前进');
  // blocker-1: position (0,1.1,4.2) size (2,2,1) → 前面 z = 4.7，加玩家半径 0.35
  const blocker = WORLD_SPEC.obstacles[0];
  const blockerFrontZ = blocker.position.z + blocker.size.z / 2 + PLAYER.radius;
  assert.ok(
    game.state.position.z >= blockerFrontZ - 1e-6,
    `障碍应阻挡角色（z=${game.state.position.z} < ${blockerFrontZ}）`,
  );
  assert.ok(game.state.position.z < blockerFrontZ + 0.5, '角色应贴着障碍而不是卡在起点');
});

test('integration: 空格在落地时起跳，空中不能二段跳', () => {
  const game = groundedGame();
  const groundY = game.state.position.y;

  stepGame(game, STEP, { jump: true });
  assert.equal(game.state.onGround, false);
  assert.ok(game.state.velocity.y > 0, '起跳后竖直速度为正');
  assert.ok(game.state.position.y > groundY, '起跳后高度增加');

  const vy = game.state.velocity.y;
  stepGame(game, STEP, { jump: true }); // 空中继续按跳跃
  assert.ok(game.state.velocity.y < vy, '空中按住跳跃不应再次加速上升');

  frames(game, 120, null); // 落回平台
  assert.equal(game.state.onGround, true);
  assert.ok(Math.abs(game.state.velocity.y) < 1e-9);
});

/* ------------------------------------------------------------------ */
/* 4. 掉落重生 / 胜利 / 判负                                            */
/* ------------------------------------------------------------------ */

test('integration: y < killY 触发掉落重生，重置到关卡 respawn 并扣分', () => {
  const game = groundedGame();
  const before = { ...game.state.position };
  game.state.position.y = WORLD_SPEC.killY - 1; // -7：低于 killY，但高于 player 兜底 fallLimit(-9)
  stepGame(game, STEP);

  assert.equal(game.state.respawns, 1);
  assert.deepEqual(game.state.position, WORLD_SPEC.respawn);
  assert.equal(game.state.lastEvent, 'respawn');
  assert.equal(game.hud.status, 'running');
  assert.equal(game.hud.score, 0, `${FALL_PENALTY} 分惩罚后下限仍为 0`);
  const event = game.events[game.events.length - 1];
  assert.equal(event.type, 'respawn');
  assert.deepEqual(event.from.y, WORLD_SPEC.killY - 1);
  assert.notDeepEqual(before, game.state.position);
});

test('integration: 抵达终点判定胜利、结算得分并冻结物理', () => {
  const game = groundedGame();
  game.state.position = { ...WORLD_SPEC.goal.position };
  stepGame(game, STEP);

  assert.equal(game.hud.status, 'won');
  assert.equal(statusLabel(game.hud.status), 'CLEARED');
  assert.ok(game.hud.score > 0 && game.hud.score <= WIN_SCORE, '通关得分应在 (0, WIN_SCORE]');
  assert.equal(game.hud.bestScore, game.hud.score);
  assert.match(snapshot(game).overlay, /通关/);
  assert.match(snapshot(game).overlay, /用时/);
  // world 判定与 main 结论一致
  assert.deepEqual(evaluatePlayer(game.world, WORLD_SPEC.goal.position), { win: true });

  const frozen = { ...game.state.position };
  const frozenTime = game.hud.time;
  frames(game, 30, { forward: 1 });
  assert.deepEqual(game.state.position, frozen, '胜利后物理应冻结');
  assert.equal(game.hud.time, frozenTime, '胜利后计时应停止');
});

test('integration: 重生次数达到 MAX_RESPAWNS 判负，overlay 提示重试且物理冻结', () => {
  const game = groundedGame();
  game.state.respawns = MAX_RESPAWNS - 1;
  game.state.position.y = WORLD_SPEC.killY - 1;
  stepGame(game, STEP);

  assert.equal(game.state.respawns, MAX_RESPAWNS);
  assert.equal(game.hud.status, 'lost');
  assert.match(snapshot(game).overlay, /掉落了/);

  const frozenTime = game.hud.time;
  const frozenPos = { ...game.state.position };
  frames(game, 30);
  assert.equal(game.hud.time, frozenTime);
  assert.deepEqual(game.state.position, frozenPos);
});

/* ------------------------------------------------------------------ */
/* 5. HUD 一致性（ui 状态 → DOM）                                       */
/* ------------------------------------------------------------------ */

test('integration: HUD 元素与 ui 状态保持一致（时间/分数/状态/overlay）', () => {
  const doc = createFakeDocument();
  const game = groundedGame({ documentRef: doc });
  frames(game, 30, { forward: 1 });

  const timeEl = doc.getElementById(HUD_IDS.time);
  const scoreEl = doc.getElementById(HUD_IDS.score);
  const statusEl = doc.getElementById(HUD_IDS.status);
  const controlsEl = doc.getElementById(HUD_IDS.controls);
  const overlayEl = doc.getElementById(HUD_IDS.overlay);
  const overlayTextEl = doc.getElementById(HUD_IDS.overlayText);

  assert.equal(timeEl.textContent, formatTime(game.hud.time));
  assert.equal(scoreEl.textContent, String(Math.trunc(game.hud.score)));
  assert.equal(statusEl.textContent, 'RUNNING');
  assert.equal(controlsEl.textContent, CONTROL_HINT);
  assert.equal(overlayEl.classList.contains('is-visible'), false, 'running 时隐藏 overlay');
  assert.equal(overlayTextEl.textContent, '');

  game.state.position = { ...WORLD_SPEC.goal.position };
  stepGame(game, STEP);
  assert.equal(statusEl.textContent, 'CLEARED');
  assert.equal(overlayEl.classList.contains('is-visible'), true, '胜利时显示 overlay');
  assert.match(overlayTextEl.textContent, /通关/);
  assert.equal(timeEl.textContent, formatTime(game.hud.time));
  assert.ok(formatHud(game.hud).includes(CONTROL_HINT), 'HUD 文案应保留操作提示');
});

test('integration: restart 复用 ui.onRestart，重置计时/分数/局号/玩家/移动平台', () => {
  const doc = createFakeDocument();
  const game = createGame({ documentRef: doc });
  const initialMovers = game.world.movers.map((mover) => ({ ...mover.position }));

  frames(game, 30);
  game.state.position = { ...WORLD_SPEC.goal.position };
  stepGame(game, STEP);
  assert.equal(game.hud.status, 'won');
  assert.ok(game.hud.time > 0);

  restartGame(game);
  assert.equal(game.hud.status, 'running');
  assert.equal(game.hud.time, 0);
  assert.equal(game.hud.score, 0);
  assert.equal(game.hud.runId, 2);
  assert.equal(game.state.respawns, 0);
  assert.deepEqual(game.state.position, WORLD_SPEC.respawn);
  assert.equal(game.world.time, 0);
  game.world.movers.forEach((mover, index) => {
    assert.ok(Math.abs(mover.position.x - initialMovers[index].x) < 1e-9);
    assert.ok(Math.abs(mover.position.y - initialMovers[index].y) < 1e-9);
    assert.ok(Math.abs(mover.position.z - initialMovers[index].z) < 1e-9);
    assert.equal(mover.delta.x, 0);
    assert.equal(mover.delta.z, 0);
  });
  assert.equal(doc.getElementById(HUD_IDS.overlay).classList.contains('is-visible'), false);
  assert.equal(doc.getElementById(HUD_IDS.time).textContent, '00:00.00');

  // 重开之后能继续正常游玩（计时恢复、可移动）
  frames(game, 30, { forward: 1 });
  assert.ok(game.hud.time > 0);
  assert.ok(game.state.position.z < WORLD_SPEC.respawn.z);
});

/* ------------------------------------------------------------------ */
/* 6. three.js 网格 / 相机 / dt 安全                                    */
/* ------------------------------------------------------------------ */

test('integration: fake THREE 下生成玩家网格并随玩家状态同步', () => {
  const game = createGame({ THREE: fakeThree });
  assert.ok(game.playerMesh, '应创建角色网格');
  assert.equal(game.playerMesh.name, 'player');
  assert.ok(game.world.group.children.length > 0, '关卡网格应挂到 group');

  frames(game, 40);
  assert.ok(
    Math.abs(game.playerMesh.position.y - (game.state.position.y + PLAYER.meshYOffset)) < 1e-9,
  );

  const camera = {
    position: new FakeVector3(),
    lookAt() {
      this.lookedAt = true;
    },
  };
  updateCamera(camera, game.state);
  assert.ok(camera.position.z > game.state.position.z, '相机应在角色身后（+Z）');
  assert.equal(camera.lookedAt, true);

  const standalone = createPlayerMesh(fakeThree);
  assert.ok(standalone instanceof FakeMesh);
  assert.equal(createPlayerMesh(null), null);
});

test('integration: dt 被安全夹取，非法 dt 不推进', () => {
  const game = createGame();
  const time = game.hud.time;
  stepGame(game, Number.NaN);
  stepGame(game, -1);
  assert.equal(game.hud.time, time);

  stepGame(game, 5);
  assert.ok(game.hud.time <= MAX_DT + 1e-9, `单帧时长应被夹到 ${MAX_DT}s`);
  assert.ok(game.elapsed <= MAX_DT + 1e-9);
});

test('integration: syncGame / resolveWin / resetWorldMotion 可直接复用', () => {
  const doc = createFakeDocument();
  const game = createGame({ documentRef: doc });
  game.hud.time = 12.345;
  syncGame(game);
  assert.equal(doc.getElementById(HUD_IDS.time).textContent, formatTime(12.345));

  resolveWin(game);
  assert.equal(game.hud.status, 'won');

  game.world.update(1.5);
  assert.ok(game.world.time > 0);
  resetWorldMotion(game);
  assert.equal(game.world.time, 0);
});
