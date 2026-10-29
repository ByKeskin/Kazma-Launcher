/* Canlı mod: yalnızca Tauri içinde çalışır. Prototipin örnek verili görünümlerini gerçek veriyle değiştirir.
   Tarayıcıda (Tauri yokken) bu dosya hiçbir şey yapmaz. Dışarıdan gelen her metin esc() ile kaçışlanır. */
(function () {
  'use strict';
  var T = window.__TAURI__;
  if (!T || !T.core || !T.event || !window.__kz) return;
  document.body.classList.add('live');
  var kz = window.__kz, inv = T.core.invoke, listen = T.event.listen;
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var esc = function (s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]; }); };
  var errText = function (e) { return typeof e === 'string' ? e : (e && e.message) || String(e); };
  var toast = function (m) { kz.toast(m); };
  var SCN = ['day', 'dusk', 'night', 'nether'];
  var cfg = { player: '', ram: 4096, last_instance: null };
  var instances = [];

  function hash(s) { var h = 7; for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0; return h; }
  function scene(cv, id) { var h = hash(id); kz.paint(cv, SCN[h % 4], h % 97); }
  function loaderLabel(i) { return i.loader === 'fabric' ? 'Fabric ' + i.loader_version : 'Vanilla'; }
  function ago(ts) {
    if (!ts) return 'Henüz oynanmadı';
    var d = Math.floor((Date.now() / 1000 - ts) / 86400);
    return d <= 0 ? 'Bugün' : d === 1 ? 'Dün' : d + ' gün önce';
  }
  function mb(b) { return (b / 1048576).toFixed(1) + ' MB'; }

  /* ---------- hesap ---------- */
  function renderAccount() {
    $('#acctName').textContent = cfg.player || 'Oyuncu adı seç';
    $('#acctType').textContent = 'Çevrimdışı hesap';
    var cv = $('#head'), x = cv.getContext('2d');
    var g = ['HHHHHHHH', 'HHHHHHHH', 'HSSSSSSH', 'SSSSSSSS', 'SWESSEWS', 'SSSSSSSS', 'SSSMMSSS', 'SSSSSSSS'];
    var m = { H: '#7A4B1E', S: '#C99672', W: '#FFFFFF', E: '#3A4FD6', M: '#A0614A' };
    g.forEach(function (row, y) { row.split('').forEach(function (c, i) { x.fillStyle = m[c]; x.fillRect(i, y, 1, 1); }); });
  }
  var oldAcct = $('#acctBtn'); var acctBtn = oldAcct.cloneNode(true); oldAcct.replaceWith(acctBtn);

  $('#loginModal').outerHTML =
    '<div class="modal" id="loginModal" hidden role="dialog" aria-modal="true" aria-labelledby="loginTitle"><div class="sheet">' +
    '<h2 id="loginTitle">Hesap</h2>' +
    '<div class="seg" id="loginSeg" style="grid-template-columns:repeat(2,1fr)"><button aria-pressed="true" data-l="off">Çevrimdışı</button><button aria-pressed="false" data-l="ms">Microsoft</button></div>' +
    '<div id="lg-off" style="display:flex;flex-direction:column;gap:10px"><div class="field"><label for="offName">Oyuncu adı</label><input id="offName" class="in" maxlength="16" placeholder="3-16 karakter, harf, rakam, _" autocomplete="off"></div>' +
    '<div class="err" id="offErr" aria-live="polite"></div><p class="hint">Çevrimdışı hesapla tek kişilik dünyalara ve karma girişi açık server\'lara girilir.</p></div>' +
    '<div id="lg-ms" style="display:none;flex-direction:column;gap:10px"><p>Microsoft ile giriş henüz etkin değil. Minecraft\'ın giriş servisi yeni uygulamalar için Mojang onayı istiyor ve başvurumuz beklemede.</p><p class="hint">Onay gelince bu sekmeden giriş yapılacak. O zamana kadar çevrimdışı hesapla oynayabilirsin.</p></div>' +
    '<div style="display:flex;gap:10px;justify-content:flex-end"><button class="btn" id="loginCancel">Vazgeç</button><button class="btn primary" id="loginOk">Kaydet</button></div></div></div>';
  var lmode = 'off';
  function openLogin() { $('#offName').value = cfg.player; $('#offErr').textContent = ''; $('#loginModal').hidden = false; $('#offName').focus(); }
  acctBtn.addEventListener('click', openLogin);
  $('#loginCancel').addEventListener('click', function () { $('#loginModal').hidden = true; });
  $('#loginSeg').addEventListener('click', function (e) {
    var b = e.target.closest('[data-l]'); if (!b) return; lmode = b.dataset.l;
    $$('#loginSeg button').forEach(function (x) { x.setAttribute('aria-pressed', x === b); });
    $('#lg-off').style.display = lmode === 'off' ? 'flex' : 'none';
    $('#lg-ms').style.display = lmode === 'ms' ? 'flex' : 'none';
    $('#loginOk').disabled = lmode === 'ms';
  });
  $('#loginOk').addEventListener('click', function () {
    inv('set_player', { name: $('#offName').value.trim() }).then(function (c) {
      cfg = c; renderAccount(); $('#loginModal').hidden = true; toast(c.player + ' olarak oynayacaksın'); renderHome();
    }).catch(function (e) { $('#offErr').textContent = errText(e); });
  });

  /* ---------- başlatma penceresi ---------- */
  $('#launchModal').outerHTML =
    '<div class="modal" id="launchModal" hidden role="dialog" aria-modal="true" aria-labelledby="launchTitle"><div class="sheet">' +
    '<div><span class="lbl" id="launchLbl">Hazırlanıyor</span><h2 id="launchTitle"></h2></div>' +
    '<div class="steps" id="launchSteps"></div><div class="bar" style="height:8px"><i id="launchBar"></i></div>' +
    '<div class="err" id="launchErr" style="white-space:pre-wrap"></div>' +
    '<div style="display:flex;justify-content:flex-end"><button class="btn" id="launchClose">Kapat</button></div></div></div>';
  var curStep = null;
  function doneStep() { if (curStep) { curStep.className = 'step done'; curStep.querySelector('.m').innerHTML = kz.ic('check', 13); curStep = null; } }
  function stageStep(text) {
    doneStep();
    var d = document.createElement('div'); d.className = 'step on';
    d.innerHTML = '<span class="m"></span><span class="t"></span>'; d.querySelector('.t').textContent = text;
    $('#launchSteps').appendChild(d); curStep = d;
  }
  function openLaunch(title) {
    $('#launchTitle').textContent = title; $('#launchLbl').textContent = 'Hazırlanıyor';
    $('#launchSteps').innerHTML = ''; $('#launchBar').style.width = '0%'; $('#launchErr').textContent = ''; curStep = null;
    $('#launchModal').hidden = false;
  }
  function launchFail(e) { doneStep(); $('#launchLbl').textContent = 'Hata'; $('#launchErr').textContent = errText(e); }
  $('#launchClose').addEventListener('click', function () { $('#launchModal').hidden = true; });
  listen('progress', function (ev) {
    if ($('#launchModal').hidden) return;
    var p = ev.payload;
    if (p.kind === 'stage') {
      stageStep(p.text); $('#launchBar').style.width = '0%';
      if (p.text.indexOf('Oyun açılıyor') === 0) { $('#launchLbl').textContent = 'Oyun açıldı. Kapanınca burası kapanır'; $('#launchBar').style.width = '100%'; }
    } else {
      if (!curStep) stageStep(p.text);
      curStep.querySelector('.t').textContent = p.text + ': ' + p.done + '/' + p.total;
      $('#launchBar').style.width = Math.round(p.done / Math.max(p.total, 1) * 100) + '%';
    }
  });
  listen('launch-done', function (ev) {
    var p = ev.payload; doneStep();
    if (p.error) { launchFail(p.error); }
    else if (p.ok) { $('#launchModal').hidden = true; toast('Oyun kapandı'); }
    else { launchFail('Oyun ' + p.code + ' koduyla kapandı.\nGünlük dosyası: ' + p.log); }
    refresh();
  });
  function play(id) {
    if (!cfg.player) { openLogin(); toast('Önce oyuncu adını belirle'); return; }
    var i = instances.filter(function (x) { return x.id === id; })[0];
    openLaunch(i ? i.name : id);
    inv('launch_instance', { id: id }).catch(launchFail);
  }

  /* ---------- yeni instance penceresi ---------- */
  document.body.insertAdjacentHTML('beforeend',
    '<div class="modal" id="newModal" hidden role="dialog" aria-modal="true" aria-labelledby="newTitle"><div class="sheet">' +
    '<h2 id="newTitle">Yeni instance</h2>' +
    '<div class="field"><label for="niVer">Minecraft sürümü</label><select id="niVer" class="in"><option>Sürümler yükleniyor...</option></select></div>' +
    '<label class="check" for="niSnap"><input type="checkbox" id="niSnap"><span>Ön sürümleri (snapshot) göster</span></label>' +
    '<label class="check" for="niFab"><input type="checkbox" id="niFab"><span>Fabric (mod desteği) kur</span></label>' +
    '<div class="field"><label for="niName">Ad</label><input id="niName" class="in" maxlength="40" autocomplete="off"></div>' +
    '<div class="err" id="niErr" aria-live="polite"></div>' +
    '<div style="display:flex;gap:10px;justify-content:flex-end"><button class="btn" id="niCancel">Vazgeç</button><button class="btn primary" id="niOk">Oluştur</button></div></div></div>');
  var verCache = {}, nameTouched = false;
  function autoName() { if (!nameTouched) $('#niName').value = 'Minecraft ' + $('#niVer').value + ($('#niFab').checked ? ' Fabric' : ''); }
  function loadVersions() {
    var snap = $('#niSnap').checked, sel = $('#niVer');
    var show = function (list) { sel.innerHTML = list.map(function (v) { return '<option>' + esc(v) + '</option>'; }).join(''); autoName(); };
    if (verCache[snap]) return show(verCache[snap]);
    sel.innerHTML = '<option>Sürümler yükleniyor...</option>';
    inv('get_versions', { snapshots: snap }).then(function (l) { verCache[snap] = l; show(l); })
      .catch(function (e) { sel.innerHTML = '<option value="">Alınamadı</option>'; $('#niErr').textContent = errText(e); });
  }
  function openNew() { nameTouched = false; $('#niErr').textContent = ''; $('#newModal').hidden = false; loadVersions(); }
  $('#niSnap').addEventListener('change', loadVersions);
  $('#niFab').addEventListener('change', autoName);
  $('#niVer').addEventListener('change', autoName);
  $('#niName').addEventListener('input', function () { nameTouched = true; });
  $('#niCancel').addEventListener('click', function () { $('#newModal').hidden = true; });
  $('#niOk').addEventListener('click', function () {
    var game = $('#niVer').value; if (!game) { $('#niErr').textContent = 'Önce bir sürüm seç'; return; }
    $('#niOk').disabled = true; $('#niErr').textContent = '';
    inv('create_instance', { name: $('#niName').value, game: game, fabric: $('#niFab').checked ? 'latest' : null })
      .then(function (i) { $('#newModal').hidden = true; toast(i.name + ' oluşturuldu'); return refresh(); })
      .catch(function (e) { $('#niErr').textContent = errText(e); })
      .then(function () { $('#niOk').disabled = false; });
  });

  /* ---------- ana sayfa ---------- */
  function renderHome() {
    var cur = instances.filter(function (i) { return i.id === cfg.last_instance; })[0] || instances[0];
    var hero = cur
      ? '<div class="hero"><canvas id="heroCv" aria-hidden="true"></canvas><div class="hero-in"><div><span class="pill acc"><i class="dot"></i>' + (cur.id === cfg.last_instance ? 'Son oynanan' : 'Hazır') + '</span>' +
        '<h1>' + esc(cur.name) + '</h1><div class="chips"><span class="pill mute">' + esc(loaderLabel(cur)) + '</span><span class="pill mute">Minecraft ' + esc(cur.game) + '</span><span class="pill mute">' + cur.mods + ' mod</span>' +
        (cur.kind === 'server' ? '<span class="pill mute">' + esc(cur.server_host) + '</span>' : '') + '</div></div>' +
        '<div class="hero-act"><button class="btn primary lg" data-play="' + esc(cur.id) + '">' + kz.ic('play', 20) + 'Oyna</button><button class="btn" data-go="join">' + kz.ic('join', 18) + 'Server\'a katıl</button></div></div></div>'
      : '<div class="hero"><canvas id="heroCv" aria-hidden="true"></canvas><div class="hero-in"><div><h1>Hoş geldin</h1><p>Başlamak için bir Minecraft sürümü seç.</p></div>' +
        '<div class="hero-act"><button class="btn primary lg" data-newinst="1">' + kz.ic('plus', 20) + 'İlk instance\'ını oluştur</button><button class="btn" data-go="join">' + kz.ic('join', 18) + 'Server\'a katıl</button></div></div></div>';
    $('#v-home').innerHTML = hero +
      '<div class="tiles">' +
      '<button class="tile" data-newinst="1"><span class="ib">' + kz.ic('plus', 20) + '</span><div><b>Sürüm seç, instance oluştur</b><span>İstediğin Minecraft sürümü, isteğe bağlı Fabric ile.</span></div></button>' +
      '<button class="tile" data-go="join"><span class="ib">' + kz.ic('link', 20) + '</span><div><b>Davet bağlantısıyla gir</b><span>Server\'ın sürümüne geçer, modları eşitler, bağlanır.</span></div></button>' +
      '<button class="tile" data-go="mods"><span class="ib">' + kz.ic('box', 20) + '</span><div><b>Mod keşfet</b><span>Modrinth kataloğu. Instance\'ına ekle.</span></div></button></div>' +
      (cfg.player ? '' : '<div class="card"><b>Oyuncu adın henüz belli değil.</b> Sol alttaki hesap kutusundan bir ad seç.</div>');
    scene($('#heroCv'), cur ? cur.id : 'bos');
  }

  /* ---------- kütüphane ---------- */
  function renderLibrary() {
    var cards = instances.map(function (i) {
      return '<article class="card inst"><div class="thumb"><canvas data-cid="' + esc(i.id) + '" aria-hidden="true"></canvas></div><div class="body"><h3>' + esc(i.name) + '</h3>' +
        '<div class="meta"><span class="pill mute">' + esc(loaderLabel(i)) + '</span><span class="pill mute">' + esc(i.game) + '</span><span class="pill mute">' + i.mods + ' mod</span>' +
        (i.kind === 'server' ? '<span class="pill acc">Server</span>' : '') + '</div>' +
        '<div class="foot"><span>' + esc(ago(i.last_played)) + '</span><button class="btn primary sm" data-play="' + esc(i.id) + '">' + kz.ic('play', 14) + 'Oyna</button></div></div></article>';
    }).join('');
    $('#v-library').innerHTML =
      '<div class="ph"><div><h1>Kütüphane</h1><p>Her instance kendi Minecraft sürümüne, yükleyicisine ve modlarına sahip. Hepsi birbirinden bağımsız. Server\'lara katılınca o server için ayrı bir instance oluşur.</p></div>' +
      '<button class="btn primary" data-newinst="1">' + kz.ic('plus', 18) + 'Yeni instance</button></div>' +
      '<div class="grid">' + cards + '<button class="add" data-newinst="1"><span>' + kz.ic('plus', 28) + '</span>Yeni instance oluştur<small class="hint">Sürüm ve Fabric seç</small></button></div>';
    $$('#v-library canvas[data-cid]').forEach(function (cv) { scene(cv, cv.dataset.cid); });
  }

  /* ---------- server'lar (henüz yok) ---------- */
  (function () {
    var v = $('#v-servers');
    Array.prototype.slice.call(v.children).forEach(function (c) { c.style.display = 'none'; });
    v.insertAdjacentHTML('afterbegin',
      '<div class="ph"><div><h1>Serverlar</h1></div></div><div class="card view-note"><h2>Server yöneticisi bir sonraki sürümde</h2>' +
      '<p style="margin:8px 0 14px;color:var(--ink-2)">Server kurma, başlatma ve konsol henüz bu pencerede yok. Server\'ın modlarını oyunculara otomatik dağıtan kısım ise hazır ve <b>Server\'a Katıl</b> sayfasından denenebilir.</p>' +
      '<p class="hint">Server sahibi araçları şimdilik komut satırında: <code>kazma-cli make-key</code>, <code>make-manifest</code>, <code>make-invite</code>. Ayrıntılar README\'de.</p></div>');
  })();

  /* ---------- server'a katıl ---------- */
  var ST = { ready: ['good', 'Hazır'], download: ['warn', 'İndirilecek'], update: ['warn', 'Güncellenecek'], disable: ['bad', 'Devre dışı olacak'], keep: ['mute', 'Yerelde kalır'] };
  $('#v-join').innerHTML =
    '<div class="ph"><div><h1>Server\'a katıl</h1><p>Server sahibinin verdiği davet bağlantısını yapıştır. Kazma server\'ın Minecraft ve Fabric sürümüne geçer, eksik modları indirir ve seni bağlar.</p></div></div>' +
    '<div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="field"><label for="joinIn">Davet bağlantısı</label>' +
    '<div class="joinbar"><input id="joinIn" class="in" placeholder="kazma://join/sunucu:25565?m=..." autocomplete="off"><button class="btn primary" id="queryBtn">' + kz.ic('refresh', 18) + 'Server\'ı sorgula</button></div></div>' +
    '<div class="err" id="joinErr" aria-live="polite"></div></div><div id="joinOut" style="display:flex;flex-direction:column;gap:16px"></div>';
  $('#queryBtn').addEventListener('click', function () {
    var b = $('#queryBtn'); b.disabled = true; $('#joinErr').textContent = ''; $('#joinOut').innerHTML = '';
    inv('join_prepare', { invite: $('#joinIn').value.trim() }).then(renderJoin)
      .catch(function (e) { $('#joinErr').textContent = errText(e); })
      .then(function () { b.disabled = false; });
  });
  function renderJoin(v) {
    var rows = v.rows.filter(function (r) { return r.status !== 'ready'; });
    var readyN = v.rows.length - rows.length;
    var need = v.is_new || !v.noop;
    var rowsHtml = rows.map(function (r) {
      var st = ST[r.status] || ST.keep;
      return '<div class="srow"><div><b>' + esc(r.id) + '</b><small class="mono">' + esc(r.version) + '</small></div><span class="pill ' + st[0] + '">' + st[1] + '</span><span class="sz tn">' + (r.size ? mb(r.size) : '') + '</span></div>';
    }).join('');
    $('#joinOut').innerHTML =
      '<div class="card" style="display:flex;flex-direction:column;gap:12px"><div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><h2 style="margin:0">' + esc(v.server_name) + '</h2>' +
      '<span class="pill mute">Minecraft ' + esc(v.game) + '</span><span class="pill mute">' + esc(v.loader) + ' ' + esc(v.loader_version) + '</span><span class="pill mute">' + esc(v.address) + '</span></div>' +
      '<p class="hint">Bu server için ayrı bir instance kullanılır; kendi sürümün ve modların etkilenmez.</p></div>' +
      '<div class="card"><h2>Mod karşılaştırması</h2>' + (v.noop ? '<p class="hint">Modların zaten server\'la aynı.</p>' : '<p class="hint">' + readyN + ' mod zaten hazır. Toplam indirme: ' + mb(v.download_bytes) + '</p>') +
      '<div class="sync">' + rowsHtml + '</div></div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px">' +
      '<div class="callout">' + kz.ic('shield', 20) + '<div>Dosyalar yalnızca Modrinth, CurseForge ya da server sahibinin adresinden gelir ve SHA-512 ile doğrulanır. Manifest server sahibinin anahtarıyla imzalı.' +
      (v.is_new ? '<br><b>Bu server\'ı ilk kez görüyorsun. İmza parmak izi: <span class="mono">' + esc(v.fingerprint) + '</span></b><br>Bu parmak izini server sahibinden başka bir kanaldan doğrula.' : '') + '</div></div>' +
      (need ? '<label class="check" for="trust"><input type="checkbox" id="trust"><span>' + (v.is_new ? 'Parmak izini doğruladım ve bu server\'ın modlarına güveniyorum. Modların oyunda kod çalıştırabileceğini biliyorum.' : 'Bu değişiklikleri onaylıyorum. Modlar oyunda kod çalıştırabilir.') + '</span></label>' : '') +
      '<div><button class="btn primary lg" id="syncBtn"' + (need ? ' disabled' : '') + '>' + kz.ic('download', 18) + (v.noop ? 'Katıl' : 'Eşitle ve katıl') + '</button></div></div>';
    var t = $('#trust'); if (t) t.addEventListener('change', function () { $('#syncBtn').disabled = !t.checked; });
    $('#syncBtn').addEventListener('click', function () {
      if (!cfg.player) { openLogin(); toast('Önce oyuncu adını belirle'); return; }
      openLaunch(v.server_name);
      inv('join_run', { trust: !!(t && t.checked) }).catch(launchFail);
    });
  }

  /* ---------- mod gezgini ---------- */
  var CATS = [['', 'Tümü'], ['optimization', 'Performans'], ['technology', 'Teknoloji'], ['adventure', 'Macera'], ['decoration', 'Dekorasyon'], ['utility', 'Araçlar'], ['library', 'Kütüphane'], ['worldgen', 'Dünya üretimi']];
  var cat = '', modTarget = '', added = {}, modsLoaded = false;
  function fabricInstances() { return instances.filter(function (i) { return i.loader === 'fabric' && i.kind === 'player'; }); }
  function renderModsShell() {
    var fi = fabricInstances();
    if (!fi.some(function (i) { return i.id === modTarget; })) modTarget = fi.length ? fi[0].id : '';
    if (!fi.length) {
      $('#v-mods').innerHTML = '<div class="ph"><div><h1>Mod gezgini</h1></div></div><div class="card view-note"><h2>Önce Fabric\'li bir instance gerek</h2><p style="margin:8px 0 14px;color:var(--ink-2)">Modlar Fabric yükleyicisiyle çalışır. Yeni instance oluştururken "Fabric kur"u işaretle.</p><button class="btn primary" data-newinst="1">' + kz.ic('plus', 18) + 'Instance oluştur</button></div>';
      return;
    }
    $('#v-mods').innerHTML =
      '<div class="ph"><div><h1>Mod gezgini</h1><p>Modrinth kataloğunda ara. Seçtiğin instance\'ın sürümüne uyan en yeni dosya ve gerekli bağımlılıkları eklenir.</p></div></div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="joinbar"><input id="modQ" class="in" style="font-family:var(--font-body)" placeholder="Mod ara, örn. sodium" aria-label="Mod ara"><button class="btn primary" id="modGo">' + kz.ic('refresh', 18) + 'Ara</button></div>' +
      '<div class="chips" id="catChips" role="group" aria-label="Kategori">' + CATS.map(function (c) { return '<button class="chip" aria-pressed="' + (c[0] === cat) + '" data-c="' + c[0] + '">' + c[1] + '</button>'; }).join('') + '</div>' +
      '<div class="field"><label for="modInst">Eklenecek instance</label><select id="modInst" class="in">' + fi.map(function (i) { return '<option value="' + esc(i.id) + '"' + (i.id === modTarget ? ' selected' : '') + '>' + esc(i.name) + ' (' + esc(i.game) + ')</option>'; }).join('') + '</select></div>' +
      '<div class="err" id="modErr" aria-live="polite"></div></div><div class="mods" id="modList"></div>';
    $('#modGo').addEventListener('click', searchMods);
    $('#modQ').addEventListener('keydown', function (e) { if (e.key === 'Enter') searchMods(); });
    $('#modInst').addEventListener('change', function () { modTarget = this.value; searchMods(); });
    $('#catChips').addEventListener('click', function (e) {
      var b = e.target.closest('[data-c]'); if (!b) return; cat = b.dataset.c;
      $$('#catChips .chip').forEach(function (x) { x.setAttribute('aria-pressed', x === b); }); searchMods();
    });
    searchMods();
  }
  function sideLabel(h) { return h.server_side === 'unsupported' ? 'Yalnızca istemci' : h.client_side === 'unsupported' ? 'Yalnızca server' : 'İstemci + server'; }
  function searchMods() {
    var list = $('#modList'); $('#modErr').textContent = ''; list.innerHTML = '<div class="card hint">Aranıyor...</div>';
    inv('search_mods', { instance: modTarget, query: $('#modQ').value.trim(), category: cat || null }).then(function (hits) {
      if (!hits.length) { list.innerHTML = '<div class="card hint">Eşleşen mod yok. Aramayı kısaltmayı ya da kategoriyi "Tümü" yapmayı dene.</div>'; return; }
      list.innerHTML = hits.map(function (h) {
        var key = modTarget + '|' + h.project_id, done = added[key];
        var icon = h.icon_url ? '<img class="ic" src="' + esc(h.icon_url) + '" alt="" width="48" height="48">' : '<div class="ic" style="background:var(--panel-2);display:grid;place-items:center;font-weight:800">' + esc((h.title || '?').charAt(0).toUpperCase()) + '</div>';
        return '<article class="card mod">' + icon + '<div class="tx"><h3>' + esc(h.title) + '</h3><p>' + esc(h.description) + '</p><div class="acts"><span class="pill mute">' + esc(sideLabel(h)) + '</span>' +
          h.categories.slice(0, 2).map(function (c) { return '<span class="pill mute">' + esc(c) + '</span>'; }).join('') + '</div></div>' +
          '<button class="btn sm' + (done ? '' : ' primary') + '" style="align-self:flex-start" data-add="' + esc(h.project_id) + '"' + (done ? ' disabled' : '') + '>' + (done ? 'Eklendi' : 'Ekle') + '</button></article>';
      }).join('');
    }).catch(function (e) { list.innerHTML = ''; $('#modErr').textContent = errText(e); });
  }
  function addMod(btn) {
    var id = btn.dataset.add; btn.disabled = true; btn.textContent = 'Ekleniyor...';
    inv('install_mod', { instance: modTarget, project: id }).then(function (names) {
      added[modTarget + '|' + id] = true; btn.textContent = 'Eklendi'; btn.classList.remove('primary');
      toast(names.length ? names.length + ' dosya eklendi' : 'Zaten ekli'); return refresh(false);
    }).catch(function (e) { btn.disabled = false; btn.textContent = 'Ekle'; $('#modErr').textContent = errText(e); });
  }

  /* ---------- ortak tıklamalar ---------- */
  $('#main').addEventListener('click', function (e) {
    var p = e.target.closest('[data-play]'); if (p) { play(p.dataset.play); return; }
    if (e.target.closest('[data-newinst]')) { openNew(); return; }
    var a = e.target.closest('[data-add]'); if (a) { addMod(a); }
  });
  addEventListener('hashchange', function () { if (location.hash === '#mods' && !modsLoaded) { modsLoaded = true; renderModsShell(); } });

  function refresh(rerender) {
    return Promise.all([inv('get_config'), inv('get_instances')]).then(function (r) {
      cfg = r[0]; instances = r[1]; renderAccount(); renderHome(); renderLibrary();
      if (rerender !== false && modsLoaded) renderModsShell();
    }).catch(function (e) { toast(errText(e)); });
  }
  refresh().then(function () { if (!cfg.player) openLogin(); });
})();
