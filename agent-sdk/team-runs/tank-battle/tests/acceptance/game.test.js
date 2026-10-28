const test = require('node:test');
const assert = require('node:assert/strict');
const C = require('../../src/game-core.js');

function started(seed = 7) {
  const g = C.createGame(seed);
  C.start(g);
  return g;
}

test('start creates a playing arena with three lives and a wave', () => {
  const g = started();
  assert.equal(g.status, 'playing');
  assert.equal(g.player.lives, 3);
  assert.equal(g.player.hp, 3);
  assert.ok(g.enemies.length >= 3);
  assert.ok(g.walls.length > 40);
});

test('WASD moves the player and walls block movement', () => {
  const g = started();
  const before = { x: g.player.x, y: g.player.y };
  C.update(g, 0.5, { keys: { d: true } });
  assert.ok(g.player.x > before.x);
  g.player.x = 2;
  const x = g.player.x;
  C.update(g, 0.2, { keys: { a: true } });
  assert.equal(g.player.x, x, 'left border wall must block');
});

test('fire creates a friendly bullet and cooldown prevents instant spam', () => {
  const g = started();
  C.update(g, 0.016, { fire: true, aim: 0 });
  const shots = g.shots;
  assert.ok(shots >= 1);
  assert.ok(g.player.cooldown > 0);
  C.update(g, 0.05, { fire: true, aim: 0 });
  assert.equal(g.shots, shots, 'cooldown must block a second shot');
  for (let i = 0; i < 8; i++) C.update(g, 0.05, { fire: true, aim: 0 });
  assert.equal(g.shots, shots + 1);
});

test('a bullet can destroy a destructible brick wall', () => {
  const g = started();
  g.enemies = [];
  g.player.x = 200; g.player.y = 200; g.player.angle = 0;
  g.walls = [{ x: 260, y: 180, type: 'brick' }];
  C.fire(g, 'player', g.player.x, g.player.y, 0, 520, true);
  for (let i = 0; i < 5; i++) C.update(g, 0.05, { aim: 0 });
  assert.equal(g.walls.length, 0);
});

test('pause stops updates and restart resets score, wave and run counters', () => {
  const g = started();
  g.score = 500; g.wave = 2; g.player.hp = 1; g.kills = 4; g.shots = 9; g.time = 20;
  C.togglePause(g);
  const t = g.time;
  C.update(g, 1, { keys: { d: true } });
  assert.equal(g.time, t);
  C.restart(g);
  assert.equal(g.status, 'playing');
  assert.equal(g.score, 0);
  assert.equal(g.wave, 1);
  assert.equal(g.player.hp, 3);
  assert.equal(g.kills, 0);
  assert.equal(g.shots, 0);
  assert.equal(g.time, 0);
});

test('clearing a wave advances and score increases', () => {
  const g = started();
  g.enemies = [];
  const wave = g.wave;
  C.update(g, 0.016, {});
  assert.equal(g.wave, wave + 1);
  assert.equal(g.score, 200);
  assert.ok(g.enemies.length >= 4);
});

test('clearing wave 3 reaches victory and adds the final bonus', () => {
  const g = started();
  g.wave = 3;
  g.enemies = [];
  C.update(g, 0.016, {});
  assert.equal(g.status, 'won');
  assert.equal(g.score, 500);
});

test('three deaths end the run instead of allowing a fourth', () => {
  const g = started();
  g.enemies = [];
  for (let life = 3; life >= 1; life--) {
    g.player.flash = 0;
    g.player.hp = 0;
    g.bullets.push({ x: g.player.x, y: g.player.y, vx: 0, vy: 0, r: 3, life: 1, friendly: false, owner: 'enemy' });
    C.update(g, 0.016, {});
    assert.equal(g.player.lives, life - 1);
  }
  assert.equal(g.status, 'lost');
});

test('snapshot is serializable and reflects HUD fields', () => {
  const g = started();
  const s = C.snapshot(g);
  assert.deepEqual(Object.keys(s).sort(), ['enemies','hp','kills','lives','paused','score','status','time','wave']);
  assert.equal(JSON.parse(JSON.stringify(s)).wave, 1);
});
