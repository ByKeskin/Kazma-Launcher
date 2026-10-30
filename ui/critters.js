/* Yükleme ekranı oyuncağı: ilerleme çubuğunun üstünde küçük blok yaratıklar yürür; üstüne tıklayınca patlarlar.
   Yaratıklar Kazma'ya özgü tasarımlardır. Renkler temadan (CSS değişkenleri) alınır. Yalnızca Tauri'de ve live.js'ten sonra çalışır. */
(function () {
  'use strict';
  var modal = document.getElementById('launchModal');
  var bar = document.getElementById('launchBar');
  if (!modal || !bar || !document.body.classList.contains('live')) return;
  var sheet = modal.querySelector('.sheet');
  var barBox = bar.parentNode;
  var cv = document.createElement('canvas');
  cv.id = 'critterCv'; cv.setAttribute('aria-hidden', 'true'); cv.title = 'Bana tıkla!';
  cv.style.cssText = 'width:100%;height:84px;display:block;cursor:pointer;image-rendering:pixelated;border-radius:8px';
  sheet.insertBefore(cv, barBox);
  var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var ctx = cv.getContext('2d');
  var PX = 4, W = 0, H = 0, raf = 0, t0 = 0, last = 0;

  // 9x9 piksel şekiller; harfler palete bağlanır: b gövde, d koyu, g parıltı, w beyaz, a aksan
  var SHAPES = [
    ['  bbbbb  ', ' bbbbbbb ', 'bbbbbbbbb', 'bbgbbbgbb', 'bbgbbbgbb', 'bbbbbbbbb', 'bbbddbbbb', ' bbbbbbb ', '  d   d  '],
    ['   aaa   ', '  aaaaa  ', ' aawaawaa', ' aaaaaaa ', ' aabbbaa ', '  aaaaa  ', ' aa   aa ', ' aa   aa ', 'dd     dd'],
    [' dddddd  ', 'dbbbbbbd ', 'dbgbbgbd ', 'dbbbbbbd ', ' dbbbbd  ', ' dbddbd  ', ' dd  dd  ', ' d    d  ', 'dd    dd ']
  ];
  function css(name, fb) { var v = getComputedStyle(document.body).getPropertyValue(name).trim(); return v || fb; }
  function palette() {
    var acc = css('--accent', '#4ee0c4'), ink = css('--ink', '#e8eef2');
    return { b: css('--ink-2', '#9aa7b4'), d: css('--panel-2', '#2a3340'), g: acc, a: acc, w: ink };
  }
  var crit = { type: 0, x: 0, dir: 1, dead: 0, born: 0 };
  var parts = [];
  var shake = 0;

  function resize() {
    var r = cv.getBoundingClientRect(), dpr = Math.min(window.devicePixelRatio || 1, 2);
    W = Math.max(120, Math.floor(r.width)); H = Math.floor(r.height);
    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.imageSmoothingEnabled = false;
  }
  function progress() { var p = parseFloat(bar.style.width) / 100; return isNaN(p) ? 0 : Math.max(0, Math.min(1, p)); }
  function targetX(now) {
    var p = progress(), span = W - 9 * PX - 16;
    var wander = p < 0.02 ? (Math.sin(now / 900) * 0.5 + 0.5) * 0.25 : 0; // henüz ilerleme yokken ileri geri gezinir
    return 8 + span * Math.min(1, p + wander);
  }
  function hit() { return { x: crit.x, y: H - 14 - 9 * PX, w: 9 * PX, h: 9 * PX }; }

  function explode() {
    var h = hit(), pal = palette(), cols = [pal.g, pal.w, pal.b, pal.a];
    for (var i = 0; i < 46; i++) {
      var ang = Math.random() * Math.PI * 2, sp = 60 + Math.random() * 220;
      parts.push({ x: h.x + h.w / 2, y: h.y + h.h / 2, vx: Math.cos(ang) * sp, vy: Math.sin(ang) * sp - 90, life: 0.7 + Math.random() * 0.6, c: cols[i % cols.length], s: PX * (Math.random() < 0.3 ? 1 : 0.5) });
    }
    crit.dead = performance.now(); shake = 0.25;
  }

  function drawShape(sh, x, y, pal, flip, squish) {
    for (var r = 0; r < 9; r++) for (var c = 0; c < 9; c++) {
      var ch = sh[r].charAt(flip ? 8 - c : c); if (ch === ' ') continue;
      ctx.fillStyle = pal[ch] || pal.b;
      ctx.fillRect(Math.round(x + c * PX), Math.round(y + r * PX * squish + (1 - squish) * 9 * PX), PX, Math.ceil(PX * squish) + 0);
    }
  }

  function frame(now) {
    if (modal.hidden) { raf = 0; return; }
    var dt = Math.min(0.05, (now - last) / 1000); last = now;
    var pal = palette();
    ctx.clearRect(0, 0, W, H);
    ctx.save();
    if (shake > 0) { ctx.translate((Math.random() - .5) * 6 * shake * 4, (Math.random() - .5) * 4 * shake * 4); shake = Math.max(0, shake - dt); }
    // zemin
    ctx.fillStyle = pal.d; ctx.fillRect(0, H - 10, W, 2);
    for (var gx = (Math.floor(now / 140) % 2) * PX; gx < W; gx += PX * 2) { ctx.fillStyle = pal.b; ctx.fillRect(gx, H - 8, PX, PX); }
    // yaratık
    if (!crit.dead) {
      var tx = targetX(now);
      var dx = tx - crit.x; crit.dir = dx >= 0 ? 1 : -1;
      crit.x += dx * Math.min(1, dt * 3.2);
      var step = Math.floor(now / 160) % 2, moving = Math.abs(dx) > 1.5;
      var bob = moving ? (step ? 0 : PX / 2) : Math.sin(now / 500) * 1;
      var age = Math.min(1, (now - crit.born) / 220), sq = 0.6 + 0.4 * age; // doğarken yukarı şişer
      drawShape(SHAPES[crit.type], crit.x, H - 14 - 9 * PX - bob, pal, crit.dir < 0, sq);
    } else if (now - crit.dead > 900) {
      crit.dead = 0; crit.type = (crit.type + 1) % SHAPES.length; crit.born = now;
    }
    // parçacıklar
    for (var i = parts.length - 1; i >= 0; i--) {
      var p = parts[i]; p.life -= dt; if (p.life <= 0) { parts.splice(i, 1); continue; }
      p.vy += 520 * dt; p.x += p.vx * dt; p.y += p.vy * dt;
      if (p.y > H - 10) { p.y = H - 10; p.vy *= -0.35; p.vx *= 0.7; }
      ctx.globalAlpha = Math.min(1, p.life * 2); ctx.fillStyle = p.c; ctx.fillRect(Math.round(p.x), Math.round(p.y), p.s, p.s);
    }
    ctx.globalAlpha = 1;
    ctx.restore();
    raf = requestAnimationFrame(frame);
  }

  function start() {
    if (reduce || raf) return;
    resize(); last = performance.now(); crit.born = last; crit.dead = 0; crit.x = 8; parts.length = 0;
    raf = requestAnimationFrame(frame);
  }
  cv.addEventListener('click', function (e) {
    if (crit.dead || reduce) return;
    var r = cv.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top, h = hit(), pad = 10;
    if (x >= h.x - pad && x <= h.x + h.w + pad && y >= h.y - pad && y <= h.y + h.h + pad) explode();
  });
  new MutationObserver(function () { if (!modal.hidden) start(); }).observe(modal, { attributes: true, attributeFilter: ['hidden'] });
  addEventListener('resize', function () { if (!modal.hidden) resize(); });
  if (reduce) cv.style.display = 'none';
  window.__kzCritter = { explode: function () { if (!crit.dead) explode(); }, state: function () { return { dead: !!crit.dead, type: crit.type, parts: parts.length }; } };
})();
