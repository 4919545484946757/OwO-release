const test = require('node:test');
const assert = require('node:assert/strict');
const C = require('../../src/game-core.js');

// Coverage gap guard (tester): none of the 7 baseline tests drives the game for
// many frames with mixed input, so a NaN/Infinity, crash or unbounded-growth bug
// in the simulation loop could ship while every baseline assertion still passes.
// Scope note: this asserts loop robustness and state integrity, NOT game balance.
// The real-time playtest (playtest/report.md) shows an unskilled player losing in
// about 38s, so surviving a fixed 20000 frames is not a fair acceptance bar and
// must not be encoded as a product requirement here.
test('long mixed-input run keeps the simulation finite and bounded', () => {
  const g = C.createGame(11);
  C.start(g);
  let frames = 0;
  let crash = null;
  let deaths = 0;
  for (let i = 0; i < 20000; i++) {
    try {
      C.update(g, 0.016, { keys: { w: i % 7 < 3, d: i % 5 < 2, s: i % 13 === 0, a: i % 17 === 0 }, aim: i % 3, fire: i % 9 === 0 });
    } catch (err) {
      crash = 'frame ' + i + ': ' + (err && err.stack ? err.stack.split('\n').slice(0, 3).join(' | ') : err);
      break;
    }
    frames++;
    // Arcade games end by design: after 'lost' update() is a no-op, so exercise
    // the real browser flow (game over -> restart) instead of forcing survival.
    if (g.status === 'lost') {
      deaths++;
      C.restart(g);
      assert.equal(g.status, 'playing', 'restart after game over must be playable');
    }
  }
  const s = C.snapshot(g);
  assert.equal(crash, null, 'simulation loop must not throw');
  assert.equal(frames, 20000, 'simulation loop must process 20000 frames');
  assert.equal(g.status, 'playing', 'game must be playable after every restart');
  assert.ok(deaths >= 1, 'mixed/blind input is expected to die at least once (sanity of the loop guard)');
  assert.ok(Number.isFinite(s.time) && s.time > 0, 'game time must stay finite');
  for (const k of ['score', 'wave', 'lives', 'hp', 'kills', 'enemies']) {
    assert.ok(Number.isFinite(s[k]), k + ' must stay finite');
  }
  assert.ok(s.wave >= 1 && s.wave <= 3, 'wave must stay inside 1..3');
  assert.ok(s.hp >= 0 && s.hp <= 3, 'hp must stay inside 0..3');
  for (const e of g.enemies) {
    assert.ok(Number.isFinite(e.x) && Number.isFinite(e.y), 'enemy position must stay finite');
    assert.ok(e.x >= 0 && e.x <= C.W && e.y >= 0 && e.y <= C.H, 'enemy must stay inside the arena');
  }
  for (const b of g.bullets) {
    assert.ok(Number.isFinite(b.x) && Number.isFinite(b.y), 'bullet position must stay finite');
  }
  for (const w of g.walls) {
    assert.ok(Number.isFinite(w.x) && Number.isFinite(w.y), 'wall position must stay finite');
  }
  assert.ok(g.bullets.length < 500, 'bullets must not accumulate: ' + g.bullets.length);
  assert.ok(g.particles.length < 2000, 'particles must not accumulate: ' + g.particles.length);
  assert.ok(JSON.parse(JSON.stringify(s)).time === s.time, 'snapshot must stay serializable');
});
