(function () {
  'use strict';
  const C = window.TankCore;
  const canvas = document.getElementById('game');
  const ctx = canvas.getContext('2d');
  const clockEl = document.getElementById('clock');
  const game = C.createGame(20261026);
  const keys = Object.create(null);
  let aim = -Math.PI / 2, firing = false, last = performance.now(), audio = null;
  let fpsFrames = 0, fpsAt = performance.now();
  let bestScore = 0, bestRecorded = false;
  // Screen shake + floating score text: pure presentation, never touches simulation.
  let shake = 0;
  const pops = [];
  try { const saved = Number(localStorage.getItem('tank-arena-best')); if (Number.isFinite(saved)) bestScore = saved; } catch (_) { /* storage optional */ }

  function tone(freq, dur, type, gain) {
    try {
      audio = audio || new (window.AudioContext || window.webkitAudioContext)();
      const osc = audio.createOscillator(), g = audio.createGain();
      osc.type = type || 'square'; osc.frequency.value = freq;
      g.gain.value = gain == null ? .035 : gain;
      osc.connect(g); g.connect(audio.destination);
      osc.start(); g.gain.exponentialRampToValueAtTime(.0001, audio.currentTime + dur);
      osc.stop(audio.currentTime + dur);
    } catch (_) { /* audio is optional */ }
  }

  function resize() {
    const ratio = canvas.width / canvas.height;
    const w = Math.min(window.innerWidth - 28, window.innerHeight * ratio - 28);
    canvas.style.width = Math.max(420, w) + 'px';
    canvas.style.height = Math.max(280, w / ratio) + 'px';
  }
  window.addEventListener('resize', resize);

  window.addEventListener('keydown', (e) => {
    keys[e.key.toLowerCase()] = true;
    if ([' ', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright'].includes(e.key.toLowerCase())) e.preventDefault();
    if (e.key === 'Enter' && (game.status === 'menu' || game.status === 'won' || game.status === 'lost')) { C.start(game); bestRecorded = false; }
    if ((e.key === 'p' || e.key === 'P') && game.status === 'playing') C.togglePause(game);
    if ((e.key === 'r' || e.key === 'R') && game.status !== 'menu') { C.restart(game); bestRecorded = false; }
  });
  window.addEventListener('keyup', (e) => { keys[e.key.toLowerCase()] = false; });
  // Do not simulate a huge catch-up frame when the tab returns from background.
  document.addEventListener('visibilitychange', () => {
    if (document.hidden && game.status === 'playing') game.paused = true;
  });
  canvas.addEventListener('mousemove', (e) => {
    const r = canvas.getBoundingClientRect();
    const x = (e.clientX - r.left) * canvas.width / r.width;
    const y = (e.clientY - r.top) * canvas.height / r.height;
    aim = Math.atan2(y - game.player.y, x - game.player.x);
  });
  canvas.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    if (game.status === 'menu' || game.status === 'won' || game.status === 'lost') { C.start(game); bestRecorded = false; return; }
    if (game.status === 'playing') firing = true;
  });
  window.addEventListener('mouseup', () => { firing = false; });
  canvas.addEventListener('click', () => {
    if (game.status === 'menu') { C.start(game); bestRecorded = false; }
  });

  // ---- readability helpers -------------------------------------------------
  function chip(x, y, w, h, border) {
    ctx.fillStyle = 'rgba(5,8,20,.82)';
    ctx.fillRect(x, y, w, h);
    ctx.strokeStyle = border || 'rgba(94,234,212,.45)';
    ctx.lineWidth = 1;
    ctx.strokeRect(x + .5, y + .5, w - 1, h - 1);
  }

  function healthBar(x, y, w, hp, max, color) {
    const h = 5, ratio = Math.max(0, Math.min(1, hp / max));
    ctx.fillStyle = 'rgba(2,6,18,.9)';
    ctx.fillRect(x, y, w, h);
    ctx.fillStyle = color;
    ctx.fillRect(x, y, w * ratio, h);
    ctx.strokeStyle = 'rgba(226,232,240,.28)';
    ctx.lineWidth = 1;
    ctx.strokeRect(x - .5, y - .5, w + 1, h + 1);
  }

  function arrowTo(x, y, dist, angle) {
    if (dist < 300) return;
    const ax = x + Math.cos(angle) * (dist * .5), ay = y + Math.sin(angle) * (dist * .5);
    ctx.save();
    ctx.translate(ax, ay); ctx.rotate(angle);
    ctx.globalAlpha = Math.min(.85, (dist - 300) / 260);
    ctx.fillStyle = '#fca5a5';
    ctx.beginPath(); ctx.moveTo(9, 0); ctx.lineTo(-6, 6); ctx.lineTo(-6, -6); ctx.closePath(); ctx.fill();
    ctx.restore();
    ctx.globalAlpha = 1;
  }
  // -------------------------------------------------------------------------

  function drawWall(w) {
    if (w.type === 'brick') {
      ctx.fillStyle = '#5f2f20'; ctx.fillRect(w.x, w.y, C.CELL, C.CELL);
      ctx.fillStyle = '#c07450';
      ctx.fillRect(w.x + 3, w.y + 3, C.CELL - 6, C.CELL / 2 - 4);
      ctx.fillRect(w.x + 3, w.y + C.CELL / 2 + 1, C.CELL - 6, C.CELL / 2 - 4);
      ctx.strokeStyle = 'rgba(255,196,140,.3)'; ctx.lineWidth = 1;
      ctx.strokeRect(w.x + .5, w.y + .5, C.CELL - 1, C.CELL - 1);
    } else {
      ctx.fillStyle = '#26364c'; ctx.fillRect(w.x, w.y, C.CELL, C.CELL);
      ctx.strokeStyle = '#7fa6c8'; ctx.lineWidth = 2;
      ctx.strokeRect(w.x + 2, w.y + 2, C.CELL - 4, C.CELL - 4);
      ctx.strokeStyle = 'rgba(150,220,255,.26)';
      ctx.beginPath(); ctx.moveTo(w.x + 8, w.y + C.CELL - 8); ctx.lineTo(w.x + C.CELL - 8, w.y + 8); ctx.stroke();
    }
  }

  function drawTank(t, color, accent, opts) {
    const o = opts || {};
    ctx.save(); ctx.translate(t.x, t.y); ctx.rotate(t.angle);
    ctx.shadowColor = o.flashColor || color; ctx.shadowBlur = o.flashColor ? 22 : 14;
    ctx.fillStyle = '#111827'; ctx.fillRect(-16, -13, 32, 26);
    ctx.fillStyle = o.flashColor || color; ctx.fillRect(-14, -11, 28, 22);
    ctx.fillStyle = accent; ctx.fillRect(-9, -7, 18, 14);
    // Barrel: grows/glows when the enemy is about to shoot (readable threat cue).
    ctx.shadowBlur = 0;
    if (o.charge > 0) {
      ctx.fillStyle = 'rgba(251,113,133,' + (.35 + .5 * o.charge) + ')';
      ctx.fillRect(9, -2 - 2 * o.charge, 20 + 6 * o.charge, 4 + 4 * o.charge);
    }
    ctx.fillStyle = '#0f172a'; ctx.fillRect(9, -3, 20, 6);
    ctx.restore();
  }

  function drawFx() {
    for (const f of game.fx || []) {
      const k = Math.max(0, Math.min(1, f.life / (f.max || .2)));
      if (f.kind === 'ring') {
        ctx.globalAlpha = k;
        ctx.strokeStyle = f.color; ctx.lineWidth = 1 + 2 * k;
        ctx.beginPath(); ctx.arc(f.x, f.y, 8 + 22 * (1 - k), 0, C.TAU); ctx.stroke();
      } else {
        // 'hit' / 'muzzle' sparks: a bright cross that shrinks away.
        ctx.globalAlpha = k;
        ctx.fillStyle = f.color;
        const s = (f.kind === 'muzzle' ? 5 : 3) + 7 * k;
        ctx.fillRect(f.x - s, f.y - 1, s * 2, 2);
        ctx.fillRect(f.x - 1, f.y - s, 2, s * 2);
        ctx.beginPath(); ctx.arc(f.x, f.y, 2 + 4 * k, 0, C.TAU); ctx.fill();
      }
    }
    ctx.globalAlpha = 1;
  }

  function drawPops(dt) {
    for (let i = pops.length - 1; i >= 0; i--) {
      const p = pops[i];
      p.life -= dt; p.y -= 34 * dt;
      if (p.life <= 0) { pops.splice(i, 1); continue; }
      ctx.globalAlpha = Math.min(1, p.life / .5);
      ctx.fillStyle = p.color; ctx.font = 'bold 15px Consolas, monospace'; ctx.textAlign = 'center';
      ctx.fillText(p.text, p.x, p.y);
    }
    ctx.globalAlpha = 1;
  }

  function draw(dt) {
    ctx.clearRect(0, 0, C.W, C.H);
    const grad = ctx.createLinearGradient(0, 0, C.W, C.H);
    grad.addColorStop(0, '#0b1020'); grad.addColorStop(.5, '#15132a'); grad.addColorStop(1, '#071a1c');
    ctx.fillStyle = grad; ctx.fillRect(0, 0, C.W, C.H);
    ctx.strokeStyle = 'rgba(94,234,212,.055)'; ctx.lineWidth = 1;
    for (let x = 0; x < C.W; x += C.CELL) { ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, C.H); ctx.stroke(); }
    for (let y = 0; y < C.H; y += C.CELL) { ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(C.W, y); ctx.stroke(); }

    ctx.save();
    if (shake > 0.1) ctx.translate((Math.random() - .5) * shake, (Math.random() - .5) * shake);

    game.walls.forEach(drawWall);
    for (const p of game.particles) {
      ctx.globalAlpha = Math.max(0, p.life * 2); ctx.fillStyle = p.color;
      ctx.fillRect(p.x - 2, p.y - 2, 4, 4);
    }
    ctx.globalAlpha = 1;

    // Projectile readability: glow halo + bright core + short trail.
    for (const b of game.bullets) {
      const col = b.friendly ? '#fde68a' : '#fda4af';
      ctx.strokeStyle = b.friendly ? 'rgba(253,230,138,.35)' : 'rgba(253,164,175,.35)';
      ctx.lineWidth = b.friendly ? 3 : 2.4;
      ctx.beginPath();
      ctx.moveTo(b.x - b.vx * .022, b.y - b.vy * .022);
      ctx.lineTo(b.x, b.y); ctx.stroke();
      ctx.fillStyle = col; ctx.shadowColor = col; ctx.shadowBlur = 14;
      ctx.beginPath(); ctx.arc(b.x, b.y, b.r + 1, 0, C.TAU); ctx.fill();
      ctx.shadowBlur = 0;
      ctx.fillStyle = '#fff';
      ctx.beginPath(); ctx.arc(b.x, b.y, Math.max(1, b.r - 1.6), 0, C.TAU); ctx.fill();
    }

    if (game.status !== 'menu') {
      for (const e of game.enemies) {
        drawTank(e, e.hitFlash > 0 ? '#fff' : '#fb7185', '#fda4af', {
          flashColor: e.hitFlash > 0 ? '#ffffff' : null, charge: e.charge || 0,
        });
        // Enemy HP pips + off-screen direction cue.
        if (e.hp > 1) healthBar(e.x - 14, e.y - 26, 28, e.hp, game.wave >= 3 ? 3 : 2, '#fda4af');
        if (e.charge > .55) arrowTo(game.player.x, game.player.y, Math.hypot(e.x - game.player.x, e.y - game.player.y), Math.atan2(e.y - game.player.y, e.x - game.player.x));
      }
      drawTank(game.player, game.player.flash > 0 ? '#fef08a' : '#34d399', '#a7f3d0', {});
      // Aim guide: dashed line from barrel, clipped by walls at first hit.
      const p = game.player;
      let len = 190;
      for (let d = 18; d < len; d += 6) {
        const x = p.x + Math.cos(p.angle) * d, y = p.y + Math.sin(p.angle) * d;
        if (C.blocked(game, x, y, 2)) { len = d; break; }
      }
      ctx.save();
      ctx.strokeStyle = 'rgba(167,243,208,.35)'; ctx.lineWidth = 2; ctx.setLineDash([7, 9]);
      ctx.beginPath();
      ctx.moveTo(p.x + Math.cos(p.angle) * 20, p.y + Math.sin(p.angle) * 20);
      ctx.lineTo(p.x + Math.cos(p.angle) * Math.min(len, 190), p.y + Math.sin(p.angle) * Math.min(len, 190));
      ctx.stroke(); ctx.restore();
    }
    drawFx();
    drawPops(dt);
    ctx.restore();

    // HUD
    chip(16, 16, 420, 44);
    ctx.font = 'bold 15px Consolas, monospace'; ctx.textAlign = 'left';
    ctx.fillStyle = '#99f6e4'; ctx.fillText('WAVE ' + game.wave + '/3', 30, 44);
    ctx.fillStyle = '#f8fafc'; ctx.fillText('SCORE ' + String(game.score).padStart(5, '0'), 142, 44);
    // Lives / HP as bars so damage reads at a glance.
    const hearts = '♥'.repeat(Math.max(0, game.player.lives));
    ctx.fillStyle = '#fca5a5'; ctx.fillText(hearts || '—', 300, 44);
    healthBar(300, 48, 60, game.player.hp, 3, game.player.hp > 1 ? '#34d399' : '#fb7185');
    chip(C.W - 176, 16, 160, 44, 'rgba(251,113,133,.45)');
    ctx.textAlign = 'right';
    ctx.fillStyle = '#fda4af'; ctx.fillText('ENEMIES ' + game.enemies.length, C.W - 30, 44);
    ctx.fillStyle = '#94a3b8'; ctx.font = '12px Consolas, monospace';
    ctx.fillText('BEST ' + String(bestScore).padStart(5, '0'), C.W - 30, 58);

    if (game.status === 'playing' && !game.paused) {
      const charging = game.enemies.some(e => (e.charge || 0) > .55);
      ctx.textAlign = 'right';
      ctx.font = 'bold 13px Consolas, monospace';
      ctx.fillStyle = charging ? '#fb7185' : 'rgba(148,163,184,.55)';
      ctx.fillText(charging ? '!! 敌人即将开火' : 'SOLID', C.W - 30, C.H - 18);
    }

    if (game.status !== 'playing') {
      ctx.fillStyle = 'rgba(3,6,18,.78)'; ctx.fillRect(0, 0, C.W, C.H);
      ctx.textAlign = 'center';
      ctx.fillStyle = '#5eead4'; ctx.font = 'bold 58px Consolas, monospace';
      ctx.fillText(game.status === 'menu' ? 'TANK ARENA' : game.status === 'won' ? 'VICTORY' : 'GAME OVER', C.W / 2, 260);
      ctx.fillStyle = '#e2e8f0'; ctx.font = '19px Consolas, monospace';
      const sub = game.status === 'menu' ? 'WASD 移动 · 鼠标瞄准 · 左键/空格开火' : '最终得分 ' + game.score + ' · Enter 同图重开 · R 换图';
      ctx.fillText(sub, C.W / 2, 312);
      if (game.status === 'menu') {
        ctx.fillStyle = '#94a3b8'; ctx.font = '15px Consolas, monospace';
        ctx.fillText('P 暂停 · R 换图 · 清空 3 波敌人获胜', C.W / 2, 350);
        ctx.fillStyle = '#5eead4'; ctx.font = 'bold 16px Consolas, monospace';
        ctx.fillText('[ 点击或按 Enter 开始 ]', C.W / 2, 400);
      } else {
        ctx.fillStyle = '#94a3b8'; ctx.font = '15px Consolas, monospace';
        ctx.fillText('命中率 ' + (game.shots ? Math.round(game.kills / game.shots * 100) : 0) + '% · 击杀 ' + game.kills, C.W / 2, 350);
      }
    } else if (game.paused) {
      ctx.fillStyle = 'rgba(3,6,18,.62)'; ctx.fillRect(0, 0, C.W, C.H);
      ctx.textAlign = 'center'; ctx.fillStyle = '#fef08a'; ctx.font = 'bold 42px Consolas, monospace';
      ctx.fillText('PAUSED', C.W / 2, 300);
      ctx.fillStyle = '#cbd5e1'; ctx.font = '16px Consolas, monospace';
      ctx.fillText('按 P 继续', C.W / 2, 340);
    }
    // touch hint
    ctx.textAlign = 'left'; ctx.fillStyle = 'rgba(148,163,184,.65)'; ctx.font = '12px Consolas, monospace';
    ctx.fillText('PROCEDURAL NEON · NO ASSETS · 60Hz STEP', 20, C.H - 18);
  }

  function loop(now) {
    const dt = Math.min(.1, (now - last) / 1000); last = now;
    const wasBullets = game.bullets.length, wasKills = game.kills, wasLives = game.player.lives, wasWave = game.wave;
    C.update(game, dt, { keys, aim, fire: firing });
    if (game.bullets.length > wasBullets) tone(680 + Math.random() * 80, .045, 'square', .018);
    if (game.kills > wasKills) {
      tone(110, .18, 'sawtooth', .045);
      const e = game.enemies[0];
      pops.push({ x: e ? e.x : C.W / 2, y: e ? e.y - 18 : C.H / 2, life: .8, text: '+100', color: '#fef08a' });
      shake = Math.min(9, shake + 5);
    }
    if (game.player.lives < wasLives) { shake = Math.max(shake, 10); tone(70, .3, 'triangle', .05); }
    if (game.wave > wasWave) { tone(880, .12, 'triangle', .04); }
    shake = Math.max(0, shake - dt * 32);
    fpsFrames++;
    if (now - fpsAt >= 500 && clockEl) {
      clockEl.textContent = 'FPS ' + Math.round(fpsFrames * 1000 / (now - fpsAt));
      fpsFrames = 0; fpsAt = now;
    }
    if ((game.status === 'won' || game.status === 'lost') && !bestRecorded) {
      bestRecorded = true;
      if (game.score > bestScore) {
        bestScore = game.score;
        try { localStorage.setItem('tank-arena-best', String(bestScore)); } catch (_) { /* storage optional */ }
      }
    }
    draw(dt);
    requestAnimationFrame(loop);
  }

  // Small read-only hook for the automated browser playtest.
  window.__tankState = () => C.snapshot(game);
  resize();
  draw(0);
  requestAnimationFrame(loop);
})();
