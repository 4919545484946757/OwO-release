/* Deterministic Tank Arena core. Browser: window.TankCore; Node: module.exports. */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.TankCore = factory();
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const W = 960, H = 640, CELL = 40, COLS = W / CELL, ROWS = H / CELL;
  const TAU = Math.PI * 2;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  function rng(seed) {
    let a = (seed >>> 0) || 0x9e3779b9;
    return function () {
      a |= 0; a = (a + 0x6d2b79f5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function makeWalls(rand) {
    const walls = [];
    for (let y = 0; y < ROWS; y++) for (let x = 0; x < COLS; x++) {
      if (x === 0 || y === 0 || x === COLS - 1 || y === ROWS - 1) {
        walls.push({ x: x * CELL, y: y * CELL, type: 'steel' });
      }
    }
    for (let y = 2; y < ROWS - 2; y += 2) for (let x = 2; x < COLS - 2; x += 2) {
      if (rand() < 0.52) walls.push({ x: x * CELL, y: y * CELL, type: 'brick' });
    }
    // Keep player's lower-left spawn and the central lane readable.
    for (const wall of walls) {
      if (wall.type === 'brick' && (wall.x < 200 && wall.y > 400)) {
        wall.x = -999; wall.y = -999;
      }
    }
    return walls.filter(w => w.x >= 0);
  }

  function rectHit(x, y, r, wall) {
    const nx = clamp(x, wall.x, wall.x + CELL);
    const ny = clamp(y, wall.y, wall.y + CELL);
    const dx = x - nx, dy = y - ny;
    return dx * dx + dy * dy < r * r;
  }

  function blocked(game, x, y, r) {
    if (x - r < 0 || y - r < 0 || x + r > W || y + r > H) return true;
    return game.walls.some(w => rectHit(x, y, r, w));
  }

  // Visual-only feedback buffer (hit sparks / kill rings / muzzle flashes).
  // Gameplay never reads it and it is capped, so simulation results, difficulty
  // and memory stay exactly as before.
  const MAX_FX = 120;
  function fx(game, kind, x, y, extra) {
    const list = game.fx || (game.fx = []);
    const e = { kind, x, y, max: 0.2, life: 0.2 };
    if (extra) Object.assign(e, extra);
    list.push(e);
    if (list.length > MAX_FX) list.splice(0, list.length - MAX_FX);
  }

  function updateFx(game, dt) {
    const list = game.fx;
    if (!list || !list.length) return;
    for (const e of list) e.life -= dt;
    game.fx = list.filter(e => e.life > 0);
  }

  function makePlayer() {
    return { x: 90, y: H - 90, angle: -Math.PI / 2, hp: 3, lives: 3, cooldown: 0, flash: 0 };
  }

  function spawnEnemy(game, index) {
    const rand = game.rand;
    const spots = [
      [W - 100, 100], [W / 2, 100], [W - 220, H / 2], [220, 160],
    ];
    const spot = spots[index % spots.length];
    const jitter = (rand() - 0.5) * 40;
    return {
      id: index + 1,
      x: clamp(spot[0] + jitter, 50, W - 50),
      y: clamp(spot[1] + jitter, 50, H - 50),
      angle: Math.PI / 2,
      hp: game.wave >= 3 ? 3 : 2,
      cooldown: 0.5 + rand(),
      speed: 52 + game.wave * 12,
      turn: rand() * TAU,
      charge: 0,
      hitFlash: 0,
    };
  }

  function startWave(game) {
    game.enemies = [];
    // Keep the first wave approachable; the old 3+wave curve made a 60s
    // playtest feel like a wall before the player learned the controls.
    const count = 2 + game.wave;
    for (let i = 0; i < count; i++) game.enemies.push(spawnEnemy(game, i));
    game.waveTimer = 0;
  }

  function createGame(seed) {
    const game = {
      rand: rng(seed || 20261026), walls: [], player: makePlayer(),
      enemies: [], bullets: [], particles: [], fx: [], score: 0, wave: 1,
      status: 'menu', paused: false, time: 0, waveTimer: 0,
      message: 'TANK ARENA', shots: 0, kills: 0,
    };
    game.walls = makeWalls(game.rand);
    return game;
  }

  function restart(game) {
    const seed = Math.floor(game.rand() * 1e9);
    const fresh = createGame(seed);
    Object.keys(game).forEach(k => { delete game[k]; });
    Object.assign(game, fresh);
    start(game);
  }

  function start(game) {
    game.status = 'playing';
    game.paused = false;
    game.time = 0;
    game.waveTimer = 0;
    game.score = 0;
    game.wave = 1;
    game.shots = 0;
    game.kills = 0;
    game.message = '';
    game.player = makePlayer();
    game.bullets = [];
    game.particles = [];
    game.fx = [];
    startWave(game);
  }

  function togglePause(game) {
    if (game.status === 'playing') game.paused = !game.paused;
  }

  function fire(game, owner, x, y, angle, speed, friendly) {
    game.bullets.push({
      x: x + Math.cos(angle) * 22,
      y: y + Math.sin(angle) * 22,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed,
      r: 3.5, life: 2.6, friendly: !!friendly, owner,
    });
    game.shots++;
  }

  function moveTank(game, tank, dx, dy) {
    const r = 15;
    if (!blocked(game, tank.x + dx, tank.y, r)) tank.x += dx;
    if (!blocked(game, tank.x, tank.y + dy, r)) tank.y += dy;
  }

  function updatePlayer(game, dt, input) {
    const p = game.player;
    p.cooldown = Math.max(0, p.cooldown - dt);
    p.flash = Math.max(0, p.flash - dt);
    const keys = input.keys || {};
    let ax = 0, ay = 0;
    if (keys.w || keys.arrowup) ay -= 1;
    if (keys.s || keys.arrowdown) ay += 1;
    if (keys.a || keys.arrowleft) ax -= 1;
    if (keys.d || keys.arrowright) ax += 1;
    const len = Math.hypot(ax, ay);
    if (len) { ax /= len; ay /= len; }
    const speed = 185 * dt;
    moveTank(game, p, ax * speed, ay * speed);
    if (Number.isFinite(input.aim)) p.angle = input.aim;
    if ((input.fire || keys[' ']) && p.cooldown <= 0) {
      p.cooldown = 0.28;
      fire(game, 'player', p.x, p.y, p.angle, 520, true);
      fx(game, 'muzzle', p.x + Math.cos(p.angle) * 26, p.y + Math.sin(p.angle) * 26, { max: 0.10, color: '#fef08a' });
    }
  }

  function updateEnemy(game, e, dt) {
    e.cooldown = Math.max(0, e.cooldown - dt);
    if (e.hitFlash > 0) e.hitFlash = Math.max(0, e.hitFlash - dt);
    const p = game.player;
    const dx = p.x - e.x, dy = p.y - e.y;
    const dist = Math.hypot(dx, dy) || 1;
    const desired = Math.atan2(dy, dx);
    // "Charge" telegraph: rises in the last 0.3s before an enemy shot so the
    // renderer can glow the barrel and the HUD can warn the player.
    e.charge = e.cooldown < 0.3 ? clamp(1 - e.cooldown / 0.3, 0, 1) : 0;
    let delta = Math.atan2(Math.sin(desired - e.angle), Math.cos(desired - e.angle));
    e.angle += clamp(delta, -1.8 * dt, 1.8 * dt);
    const speed = e.speed * dt;
    // Move toward player but strafe around obstacles.
    const nx = Math.cos(e.angle) * speed, ny = Math.sin(e.angle) * speed;
    const beforeX = e.x, beforeY = e.y;
    moveTank(game, e, nx, ny);
    if (Math.abs(e.x - beforeX) + Math.abs(e.y - beforeY) < speed * 0.35) {
      e.turn += dt * 2.4;
      const sx = Math.cos(e.turn) * speed * 1.4, sy = Math.sin(e.turn) * speed * 1.4;
      moveTank(game, e, sx, sy);
    }
    if (dist < 560 && e.cooldown <= 0) {
      const shotAngle = Math.atan2(dy, dx);
      e.cooldown = Math.max(0.7, 1.9 - game.wave * 0.16) + game.rand() * 0.4;
      fire(game, e.id, e.x, e.y, shotAngle, 260, false);
      fx(game, 'muzzle', e.x + Math.cos(shotAngle) * 26, e.y + Math.sin(shotAngle) * 26, { max: 0.16, color: '#fb7185' });
    }
  }

  function updateBullets(game, dt) {
    for (const b of game.bullets) {
      b.x += b.vx * dt;
      b.y += b.vy * dt;
      b.life -= dt;
      if (b.life <= 0 || b.x < 0 || b.y < 0 || b.x > W || b.y > H) b.dead = true;
      for (const wall of game.walls) {
        if (!wall.dead && rectHit(b.x, b.y, b.r, wall)) {
          if (wall.type === 'brick') wall.dead = true;
          b.dead = true;
          fx(game, 'hit', b.x, b.y, {
            max: wall.type === 'brick' ? 0.22 : 0.14,
            color: wall.type === 'brick' ? '#ff9d5c' : '#9adfff',
          });
          for (let i = 0; i < 5; i++) game.particles.push({
            x: b.x, y: b.y, vx: (game.rand() - .5) * 160,
            vy: (game.rand() - .5) * 160, life: .25, color: wall.type === 'brick' ? '#ff9d5c' : '#9adfff',
          });
          break;
        }
      }
      if (b.dead) continue;
      if (b.friendly) {
        for (const e of game.enemies) {
          if (!e.dead && Math.hypot(b.x - e.x, b.y - e.y) < 18) {
            b.dead = true; e.hp -= 1;
            e.hitFlash = 0.16;
            if (e.hp <= 0) {
              e.dead = true; game.kills++; game.score += 100;
              fx(game, 'ring', e.x, e.y, { max: 0.40, color: '#fef08a' });
              for (let i = 0; i < 18; i++) game.particles.push({
                x: e.x, y: e.y, vx: (game.rand() - .5) * 320,
                vy: (game.rand() - .5) * 320, life: .55, color: i % 2 ? '#ffd166' : '#ff5c8a',
              });
            } else {
              fx(game, 'hit', b.x, b.y, { max: 0.18, color: '#fef08a' });
            }
            break;
          }
        }
      } else if (Math.hypot(b.x - game.player.x, b.y - game.player.y) < 17) {
        b.dead = true;
        fx(game, 'hit', b.x, b.y, { max: 0.20, color: '#fb7185' });
        if (game.player.flash <= 0) {
          game.player.hp -= 1;
          game.player.flash = 1.2;
          fx(game, 'ring', game.player.x, game.player.y, { max: 0.35, color: '#fb7185' });
          if (game.player.hp <= 0) game.player.lives -= 1;
          if (game.player.lives <= 0) game.status = 'lost';
          else if (game.player.hp <= 0) {
            game.player.hp = 3;
            game.player.x = 90; game.player.y = H - 90;
            game.player.flash = 2.0;
            fx(game, 'ring', game.player.x, game.player.y, { max: 0.5, color: '#5eead4' });
          }
        }
      }
    }
    game.bullets = game.bullets.filter(b => !b.dead);
    game.walls = game.walls.filter(w => !w.dead);
  }

  function updateParticles(game, dt) {
    for (const p of game.particles) {
      p.x += p.vx * dt; p.y += p.vy * dt; p.life -= dt;
    }
    game.particles = game.particles.filter(p => p.life > 0);
  }

  function update(game, dt, input) {
    if (game.status !== 'playing' || game.paused) return;
    dt = clamp(dt, 0, 0.05);
    game.time += dt;
    game.waveTimer += dt;
    updatePlayer(game, dt, input || {});
    for (const e of game.enemies) if (!e.dead) updateEnemy(game, e, dt);
    updateBullets(game, dt);
    updateParticles(game, dt);
    updateFx(game, dt);
    game.enemies = game.enemies.filter(e => !e.dead);
    if (game.enemies.length === 0 && game.status === 'playing') {
      if (game.wave >= 3) {
        game.status = 'won';
        game.score += 500;
      } else {
        game.wave++;
        game.score += 200;
        startWave(game);
      }
    }
  }

  function snapshot(game) {
    return {
      status: game.status, paused: game.paused, score: game.score, wave: game.wave,
      lives: game.player.lives, hp: game.player.hp, kills: game.kills,
      enemies: game.enemies.length, time: game.time,
    };
  }

  return {
    W, H, CELL, COLS, ROWS, TAU, createGame, start, restart, togglePause, update, snapshot,
    blocked, moveTank, fire,
  };
});
