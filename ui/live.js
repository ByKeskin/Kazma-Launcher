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
  var LN = { vanilla: 'Vanilla', fabric: 'Fabric', quilt: 'Quilt', forge: 'Forge', neoforge: 'NeoForge' };
  function loaderName(k) { return LN[k] || k; }
  function loaderLabel(i) { return i.loader === 'vanilla' ? 'Vanilla' : loaderName(i.loader) + ' ' + i.loader_version; }
  function loaderOptions(sel) { return Object.keys(LN).map(function (k) { return '<option value="' + k + '"' + (k === sel ? ' selected' : '') + '>' + LN[k] + '</option>'; }).join(''); }
  function ago(ts) {
    if (!ts) return 'Henüz oynanmadı';
    var d = Math.floor((Date.now() / 1000 - ts) / 86400);
    return d <= 0 ? 'Bugün' : d === 1 ? 'Dün' : d + ' gün önce';
  }
  function trashIcon() { return '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/></svg>'; }
  function playButton(i, cls) {
    return (i.managed && i.source)
      ? '<button class="btn primary ' + cls + '" data-resync="' + esc(i.id) + '">' + kz.ic('refresh', 16) + 'Güncelle ve katıl</button>'
      : '<button class="btn primary ' + cls + '" data-play="' + esc(i.id) + '">' + kz.ic('play', cls === 'lg' ? 20 : 14) + 'Oyna</button>';
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
  function resync(id) {
    var i = instances.filter(function (x) { return x.id === id; })[0]; if (!i || !i.source) return;
    location.hash = 'join'; kz.route();
    $$('#jmSeg button').forEach(function (b) { if (b.dataset.jm === 'inv') b.click(); });
    $('#joinIn').value = i.source; $('#queryBtn').click();
  }
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
    '<div class="field"><label for="niLoader">Mod yükleyici</label><select id="niLoader" class="in">' + loaderOptions('vanilla') + '</select><span class="hint" id="niHint">Vanilla: modsuz, saf Minecraft.</span></div>' +
    '<div class="field"><label for="niName">Ad</label><input id="niName" class="in" maxlength="40" autocomplete="off"></div>' +
    '<div class="err" id="niErr" aria-live="polite"></div>' +
    '<div style="display:flex;gap:10px;justify-content:flex-end"><button class="btn" id="niCancel">Vazgeç</button><button class="btn primary" id="niOk">Oluştur</button></div></div></div>');
  var verCache = {}, nameTouched = false;
  function autoName() {
    var l = $('#niLoader').value;
    $('#niHint').textContent = l === 'vanilla' ? 'Vanilla: modsuz, saf Minecraft.' : (l === 'forge' || l === 'neoforge' ? loaderName(l) + ' ilk oyunda kurulur, birkaç dakika sürebilir. En uygun sürüm otomatik seçilir.' : loaderName(l) + ' için en uygun sürüm otomatik seçilir.');
    if (!nameTouched) $('#niName').value = 'Minecraft ' + $('#niVer').value + (l === 'vanilla' ? '' : ' ' + loaderName(l));
  }
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
  $('#niLoader').addEventListener('change', autoName);
  $('#niVer').addEventListener('change', autoName);
  $('#niName').addEventListener('input', function () { nameTouched = true; });
  $('#niCancel').addEventListener('click', function () { $('#newModal').hidden = true; });
  $('#niOk').addEventListener('click', function () {
    var game = $('#niVer').value; if (!game) { $('#niErr').textContent = 'Önce bir sürüm seç'; return; }
    $('#niOk').disabled = true; $('#niErr').textContent = ''; $('#niOk').textContent = 'Oluşturuluyor...';
    inv('create_instance', { name: $('#niName').value, game: game, loader: $('#niLoader').value, version: null })
      .then(function (i) { $('#newModal').hidden = true; toast(i.name + ' oluşturuldu'); return refresh(); })
      .catch(function (e) { $('#niErr').textContent = errText(e); })
      .then(function () { $('#niOk').disabled = false; $('#niOk').textContent = 'Oluştur'; });
  });

  /* ---------- instance silme ---------- */
  document.body.insertAdjacentHTML('beforeend',
    '<div class="modal" id="delModal" hidden role="dialog" aria-modal="true" aria-labelledby="delTitle"><div class="sheet">' +
    '<h2 id="delTitle">Instance silinsin mi?</h2><p id="delText"></p><p class="hint">Bu işlem geri alınamaz: dünyalar, ayarlar ve modlar dahil klasörün tamamı silinir. Oyun açıksa önce kapat.</p>' +
    '<div class="err" id="delErr" aria-live="polite"></div>' +
    '<div style="display:flex;gap:10px;justify-content:flex-end"><button class="btn" id="delCancel">Vazgeç</button><button class="btn dangerfill" id="delOk">Sil</button></div></div></div>');
  var delId = null;
  function openDelete(id) {
    var i = instances.filter(function (x) { return x.id === id; })[0]; if (!i) return;
    delId = id; $('#delErr').textContent = '';
    $('#delText').textContent = '"' + i.name + '" (' + i.game + ', ' + loaderLabel(i) + ') silinecek.';
    $('#delModal').hidden = false; $('#delCancel').focus();
  }
  $('#delCancel').addEventListener('click', function () { $('#delModal').hidden = true; });
  $('#delOk').addEventListener('click', function () {
    var b = $('#delOk'); b.disabled = true;
    inv('delete_instance', { id: delId }).then(function () { $('#delModal').hidden = true; toast('Instance silindi'); return refresh(); })
      .catch(function (e) { $('#delErr').textContent = errText(e); }).then(function () { b.disabled = false; });
  });

  /* ---------- ana sayfa ---------- */
  function renderHome() {
    var cur = instances.filter(function (i) { return i.id === cfg.last_instance; })[0] || instances[0];
    var hero = cur
      ? '<div class="hero"><canvas id="heroCv" aria-hidden="true"></canvas><div class="hero-in"><div><span class="pill acc"><i class="dot"></i>' + (cur.id === cfg.last_instance ? 'Son oynanan' : 'Hazır') + '</span>' +
        '<h1>' + esc(cur.name) + '</h1><div class="chips"><span class="pill mute">' + esc(loaderLabel(cur)) + '</span><span class="pill mute">Minecraft ' + esc(cur.game) + '</span><span class="pill mute">' + cur.mods + ' mod</span>' +
        (cur.kind === 'server' ? '<span class="pill mute">' + esc(cur.server_host) + '</span>' : '') + '</div></div>' +
        '<div class="hero-act">' + playButton(cur, 'lg') + '<button class="btn" data-go="join">' + kz.ic('join', 18) + 'Server\'a katıl</button></div></div></div>'
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
      return '<article class="card inst"><div class="thumb"><canvas data-cid="' + esc(i.id) + '" aria-hidden="true"></canvas><button class="del" data-del="' + esc(i.id) + '" aria-label="' + esc(i.name) + ' instance\'ını sil" title="Sil">' + trashIcon() + '</button></div><div class="body"><h3>' + esc(i.name) + '</h3>' +
        '<div class="meta"><span class="pill mute">' + esc(loaderLabel(i)) + '</span><span class="pill mute">' + esc(i.game) + '</span><span class="pill mute">' + i.mods + ' mod</span>' +
        (i.kind === 'server' ? '<span class="pill acc">Server</span>' : '') + '</div>' +
        '<div class="foot"><span>' + esc(ago(i.last_played)) + '</span><span class="acts2"><button class="btn sm icon" data-open="' + esc(i.id) + '" title="Dosya konumunu aç" aria-label="' + esc(i.name) + ' klasörünü aç">' + kz.ic('folder', 16) + '</button>' + playButton(i, 'sm') + '</span></div></div></article>';
    }).join('');
    $('#v-library').innerHTML =
      '<div class="ph"><div><h1>Kütüphane</h1><p>Her instance kendi Minecraft sürümüne, yükleyicisine ve modlarına sahip. Hepsi birbirinden bağımsız. Server\'lara katılınca o server için ayrı bir instance oluşur.</p></div>' +
      '<button class="btn primary" data-newinst="1">' + kz.ic('plus', 18) + 'Yeni instance</button></div>' +
      '<div class="grid">' + cards + '<button class="add" data-newinst="1"><span>' + kz.ic('plus', 28) + '</span>Yeni instance oluştur<small class="hint">Sürüm ve mod yükleyici seç</small></button></div>';
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
  var joinMode = 'inv', allVersions = null;
  $('#v-join').innerHTML =
    '<div class="ph"><div><h1>Server\'a katıl</h1><p>Mod eşitlemeli davet bağlantısıyla ya da herhangi bir server adresiyle (Aternos dahil) bağlan.</p></div></div>' +
    '<div class="seg" id="jmSeg" style="grid-template-columns:repeat(3,1fr);max-width:820px"><button aria-pressed="true" data-jm="inv">Davet / paket kodu (otomatik eşitleme)</button><button aria-pressed="false" data-jm="addr">Sunucu adresi (Aternos vb.)</button><button aria-pressed="false" data-jm="own">Server sahibi: paket kodu üret</button></div>' +
    '<div id="jm-inv" style="display:flex;flex-direction:column;gap:16px"><div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="field"><label for="joinIn">Davet bağlantısı ya da paket kodu</label>' +
    '<div class="joinbar"><input id="joinIn" class="in" placeholder="kazma://pack/...  ya da  kazma://join/...?m=..." autocomplete="off"><button class="btn primary" id="queryBtn">' + kz.ic('refresh', 18) + 'Server\'ı sorgula</button></div>' +
    '<span class="hint">Server sahibi sana bir paket kodu ya da davet bağlantısı verdiyse buraya yapıştır. Kazma server\'ın sürümüne geçer ve modları kendisi indirip eşitler.</span></div>' +
    '<div class="err" id="joinErr" aria-live="polite"></div></div><div id="joinOut" style="display:flex;flex-direction:column;gap:16px"></div></div>' +
    '<div id="jm-addr" style="display:none;flex-direction:column;gap:16px"><div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="field"><label for="addrIn">Sunucu adresi</label>' +
    '<div class="joinbar"><input id="addrIn" class="in" placeholder="ornek.aternos.me:12345" autocomplete="off"><button class="btn primary" id="probeBtn">' + kz.ic('refresh', 18) + 'Server\'ı sorgula</button></div>' +
    '<span class="hint">Kazma server\'a sorup sürümünü ve mod yükleyicisini tahmin eder. Server kapalıysa sürümü elle seçebilirsin.</span></div><div class="err" id="addrErr" aria-live="polite"></div></div><div id="addrOut" style="display:flex;flex-direction:column;gap:16px"></div></div>' +
    '<div id="jm-own" style="display:none;flex-direction:column;gap:16px"><div class="card" style="display:flex;flex-direction:column;gap:14px"><h2 style="margin:0">Paket kodu üret</h2>' +
    '<p class="hint">Server\'ın <b>mods</b> klasörünü seç; Kazma modları Modrinth\'te bulup tek bir kısa kod üretir. Kodu oyunculara gönder, katılırken modlar otomatik eşitlenir. Aternos\'ta mods klasörünü panelden indirip buraya koyabilirsin. Modrinth\'te olmayan modlar koda girmez, oyuncuların elle eklemesi gerekir.</p>' +
    '<div class="field"><label for="ownInst">Hazır instance\'tan al</label><select id="ownInst" class="in"><option value="">Klasör yolunu elle yazacağım</option></select><span class="hint">Server\'ın modlarını kendi instance\'ına elle koyduysan onu seç: mods klasörü, sürüm, yükleyici ve adres otomatik dolar.</span></div>' +
    '<div class="field"><label for="ownPath">Mods klasörünün yolu</label><input id="ownPath" class="in" placeholder="C:\\Users\\...\\mods" autocomplete="off"><span class="hint">Dosya Gezgini\'nde klasörü açıp adres çubuğundaki yolu kopyalayıp yapıştır.</span></div>' +
    '<div class="joinbar"><div class="field" style="flex:1;min-width:200px"><label for="ownName">Server adı</label><input id="ownName" class="in" maxlength="60" autocomplete="off"></div>' +
    '<div class="field" style="flex:1;min-width:200px"><label for="ownAddr">Server adresi</label><input id="ownAddr" class="in" placeholder="ornek.aternos.me:12345" autocomplete="off"></div></div>' +
    '<div class="joinbar"><div class="field" style="flex:1;min-width:160px"><label for="ownGame">Minecraft sürümü</label><select id="ownGame" class="in"><option>Yükleniyor...</option></select></div>' +
    '<div class="field" style="flex:1;min-width:160px"><label for="ownLoader">Mod yükleyici</label><select id="ownLoader" class="in"><option value="fabric">Fabric</option><option value="quilt">Quilt</option><option value="forge">Forge</option><option value="neoforge">NeoForge</option></select></div>' +
    '<div class="field" style="flex:1;min-width:160px"><label for="ownVer">Yükleyici sürümü (isteğe bağlı)</label><input id="ownVer" class="in" placeholder="otomatik" autocomplete="off"></div></div>' +
    '<div><button class="btn primary lg" id="ownGo">' + kz.ic('box', 18) + 'Kodu üret</button></div><div class="err" id="ownErr" aria-live="polite"></div></div><div id="ownOut" style="display:flex;flex-direction:column;gap:16px"></div></div>';
  $('#jmSeg').addEventListener('click', function (e) {
    var b = e.target.closest('[data-jm]'); if (!b) return; joinMode = b.dataset.jm;
    $$('#jmSeg button').forEach(function (x) { x.setAttribute('aria-pressed', x === b); });
    ['inv', 'addr', 'own'].forEach(function (m) { $('#jm-' + m).style.display = joinMode === m ? 'flex' : 'none'; });
    if (joinMode === 'own') ownInit();
  });
  $('#queryBtn').addEventListener('click', function () {
    var b = $('#queryBtn'); b.disabled = true; $('#joinErr').textContent = ''; $('#joinOut').innerHTML = '';
    inv('join_prepare', { invite: $('#joinIn').value.trim() }).then(renderJoin)
      .catch(function (e) { $('#joinErr').textContent = errText(e); })
      .then(function () { b.disabled = false; });
  });
  function renderJoin(v) {
    var rows = v.rows.filter(function (r) { return r.status !== 'ready'; });
    var readyN = v.rows.length - rows.length;
    var need = !v.signed || v.is_new || !v.noop;
    var rowsHtml = rows.map(function (r) {
      var st = ST[r.status] || ST.keep;
      return '<div class="srow"><div><b>' + esc(r.id) + '</b><small class="mono">' + esc(r.version) + '</small></div><span class="pill ' + st[0] + '">' + st[1] + '</span><span class="sz tn">' + (r.size ? mb(r.size) : '') + '</span></div>';
    }).join('');
    $('#joinOut').innerHTML =
      '<div class="card" style="display:flex;flex-direction:column;gap:12px"><div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><h2 style="margin:0">' + esc(v.server_name) + '</h2>' +
      '<span class="pill mute">Minecraft ' + esc(v.game) + '</span><span class="pill mute">' + esc(loaderName(v.loader)) + ' ' + esc(v.loader_version) + '</span><span class="pill mute">' + esc(v.address) + '</span></div>' +
      '<p class="hint">Bu server için ayrı bir instance kullanılır; kendi sürümün ve modların etkilenmez.</p></div>' +
      '<div class="card"><h2>Mod karşılaştırması</h2>' + (v.noop ? '<p class="hint">Modların zaten server\'la aynı.</p>' : '<p class="hint">' + readyN + ' mod zaten hazır. Toplam indirme: ' + mb(v.download_bytes) + '</p>') +
      '<div class="sync">' + rowsHtml + '</div></div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px">' +
      '<div class="callout">' + kz.ic('shield', 20) + '<div>' + (v.signed
        ? 'Dosyalar yalnızca Modrinth, CurseForge ya da server sahibinin adresinden gelir ve SHA-512 ile doğrulanır. Manifest server sahibinin anahtarıyla imzalı.'
        : 'Dosyalar Modrinth\'ten gelir ve SHA-512 ile doğrulanır. Paket kodu imzasızdır: kodu kimin hazırladığını doğrulayamayız, bu yüzden yalnızca tanıdığın birinden aldıysan devam et.') +
      (v.signed && v.is_new ? '<br><b>Bu server\'ı ilk kez görüyorsun. İmza parmak izi: <span class="mono">' + esc(v.fingerprint) + '</span></b><br>Bu parmak izini server sahibinden başka bir kanaldan doğrula.' : '') + '</div></div>' +
      (need ? '<label class="check" for="trust"><input type="checkbox" id="trust"><span>' + (!v.signed ? 'Bu paket koduna güveniyorum. Modların oyunda kod çalıştırabileceğini biliyorum.' : v.is_new ? 'Parmak izini doğruladım ve bu server\'ın modlarına güveniyorum. Modların oyunda kod çalıştırabileceğini biliyorum.' : 'Bu değişiklikleri onaylıyorum. Modlar oyunda kod çalıştırabilir.') + '</span></label>' : '') +
      '<div><button class="btn primary lg" id="syncBtn"' + (need ? ' disabled' : '') + '>' + kz.ic('download', 18) + (v.noop ? 'Katıl' : 'Eşitle ve katıl') + '</button></div></div>';
    var t = $('#trust'); if (t) t.addEventListener('change', function () { $('#syncBtn').disabled = !t.checked; });
    $('#syncBtn').addEventListener('click', function () {
      if (!cfg.player) { openLogin(); toast('Önce oyuncu adını belirle'); return; }
      openLaunch(v.server_name);
      inv('join_run', { trust: !!(t && t.checked) }).catch(launchFail);
    });
  }

  /* server sahibi: paket kodu üret */
  function loadVersionsInto(sel) {
    var fill = function (l) { sel.innerHTML = l.map(function (v) { return '<option>' + esc(v) + '</option>'; }).join(''); };
    if (allVersions) return fill(allVersions);
    inv('get_versions', { snapshots: false }).then(function (l) { allVersions = l; fill(l); }).catch(function (e) { sel.innerHTML = '<option value="">Alınamadı</option>'; $('#ownErr').textContent = errText(e); });
  }
  function ownInit() {
    if ($('#ownGame').options.length <= 1) loadVersionsInto($('#ownGame'));
    var opts = instances.filter(function (i) { return i.loader !== 'vanilla' && i.mods > 0 && !i.managed; });
    $('#ownInst').innerHTML = '<option value="">Klasör yolunu elle yazacağım</option>' + opts.map(function (i) { return '<option value="' + esc(i.id) + '">' + esc(i.name) + ' (' + esc(i.game) + ' · ' + esc(loaderName(i.loader)) + ', ' + i.mods + ' mod)</option>'; }).join('');
  }
  function setSel(sel, val) { if (!$$('option', sel).some(function (o) { return o.value === val || o.textContent === val; })) sel.insertAdjacentHTML('afterbegin', '<option>' + esc(val) + '</option>'); sel.value = val; }
  $('#ownInst').addEventListener('change', function () {
    var i = instances.filter(function (x) { return x.id === this.value; }, this)[0];
    $('#ownPath').disabled = !!i;
    if (!i) return;
    setSel($('#ownGame'), i.game); $('#ownLoader').value = i.loader; $('#ownVer').value = i.loader_version;
    $('#ownName').value = i.kind === 'server' ? (i.server_host || i.name) : i.name;
    if (i.server_host) $('#ownAddr').value = i.server_host + (i.server_port && i.server_port !== 25565 ? ':' + i.server_port : '');
  });
  $('#ownGo').addEventListener('click', function () {
    var b = $('#ownGo'); b.disabled = true; $('#ownErr').textContent = ''; $('#ownOut').innerHTML = ''; b.lastChild.textContent = 'Üretiliyor...';
    inv('make_pack', { instance: $('#ownInst').value || null, path: $('#ownPath').value.trim(), name: $('#ownName').value.trim(), address: $('#ownAddr').value.trim(), game: $('#ownGame').value, loader: $('#ownLoader').value, version: $('#ownVer').value.trim() || null })
      .then(function (r) {
        $('#ownOut').innerHTML = '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Paket kodun hazır</h2>' +
          '<p>' + r.included + ' mod koda girdi.</p><textarea id="ownCode" class="in mono" rows="4" readonly></textarea>' +
          '<div><button class="btn primary" id="ownCopy">' + kz.ic('copy', 16) + 'Kodu kopyala</button></div>' +
          (r.missing.length ? '<div class="callout">' + kz.ic('shield', 20) + '<div><b>Modrinth\'te bulunamadı, oyuncular bunları elle eklemeli:</b><br>' + r.missing.map(esc).join(', ') + '</div></div>' : '') +
          (r.skipped_server_only.length ? '<p class="hint">Yalnızca server\'da çalıştığı için koda girmedi: ' + r.skipped_server_only.map(esc).join(', ') + '</p>' : '') +
          '<p class="hint">Kodu kimden aldığını oyuncuların bilmesi önemli: kod imzasız olduğu için yalnızca tanıdıkların arasında paylaş.</p></div>';
        $('#ownCode').value = r.code;
        $('#ownCopy').addEventListener('click', function () {
          var t = $('#ownCode'); t.select();
          (navigator.clipboard ? navigator.clipboard.writeText(t.value) : Promise.reject()).then(function () { toast('Kod kopyalandı'); }).catch(function () { toast('Kopyalanamadı, kodu elle seçip kopyala'); });
        });
      })
      .catch(function (e) { $('#ownErr').textContent = errText(e); })
      .then(function () { b.disabled = false; b.lastChild.textContent = 'Kodu üret'; });
  });

  /* adresle bağlanma */
  $('#probeBtn').addEventListener('click', function () {
    var b = $('#probeBtn'); b.disabled = true; $('#addrErr').textContent = ''; $('#addrOut').innerHTML = '';
    var addr = $('#addrIn').value.trim();
    var vers = allVersions ? Promise.resolve(allVersions) : inv('get_versions', { snapshots: false }).then(function (l) { allVersions = l; return l; });
    Promise.all([inv('probe_server', { address: addr }), vers]).then(function (r) { renderProbe(addr, r[0], r[1]); })
      .catch(function (e) { $('#addrErr').textContent = errText(e); })
      .then(function () { b.disabled = false; });
  });
  function renderProbe(addr, p, versions) {
    var game = p.guessed_game || versions[0], loader = p.loader_hint === 'forge' ? 'forge' : 'vanilla';
    var opts = versions.map(function (v) { return '<option' + (v === game ? ' selected' : '') + '>' + esc(v) + '</option>'; }).join('');
    $('#addrOut').innerHTML =
      '<div class="card" style="display:flex;flex-direction:column;gap:10px"><div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><h2 style="margin:0">' + esc(p.host + (p.port === 25565 ? '' : ':' + p.port)) + '</h2>' +
      (p.online ? '<span class="pill good"><i class="dot"></i>Yanıt veriyor</span>' : '<span class="pill warn">Ulaşılamadı</span>') +
      (p.version_name ? '<span class="pill mute">' + esc(p.version_name) + '</span>' : '') + (p.players ? '<span class="pill mute">' + esc(p.players) + ' oyuncu</span>' : '') +
      (p.loader_hint === 'forge' ? '<span class="pill acc">Forge / NeoForge modları var</span>' : '') + '</div>' +
      (p.motd ? '<p class="hint">' + esc(p.motd) + '</p>' : '') + (p.note ? '<p>' + esc(p.note) + '</p>' : '') +
      (p.existing ? '<p class="hint">Bu adres için daha önce bir instance oluşturulmuş. Seçimi değiştirirsen güncellenir.</p>' : '') + '</div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px"><h2 style="margin:0">Nasıl bağlanılsın?</h2>' +
      '<div class="field"><label for="acGame">Minecraft sürümü</label><select id="acGame" class="in">' + opts + '</select></div>' +
      '<div class="field"><label for="acLoader">Mod yükleyici</label><select id="acLoader" class="in">' + loaderOptions(loader) + '</select>' +
      '<span class="hint">Server mod kullanıyorsa onun yükleyicisini seç. Fabric/Quilt server\'lar kendini duyurmaz, sahibinden öğren.</span></div>' +
      '<div class="callout">' + kz.ic('shield', 20) + '<div>Çevrimdışı hesapla yalnızca "cracked" (çevrimdışı mod) açık server\'lara girilir. Aternos\'ta bunu server sahibi <b>Cracked</b> ayarından açar. Premium server\'lar için Microsoft girişi gerekir, o henüz etkin değil.</div></div>' +
      '<div class="callout">' + kz.ic('refresh', 20) + '<div><b>Modları otomatik eşitlemek için</b> server sahibinden bir <b>paket kodu</b> iste ve soldaki "Davet / paket kodu" sekmesine yapıştır. Kod yoksa modları elle ekleyebilirsin: "Instance\'ı hazırla"ya bas, <b>Mod gezgini</b>\'nden bu instance\'ı seçip ekle, sonra bağlan.</div></div>' +
      '<div style="display:flex;gap:10px;flex-wrap:wrap"><button class="btn" id="acPrep">' + kz.ic('box', 18) + 'Instance\'ı hazırla</button><button class="btn primary lg" id="acGo">' + kz.ic('play', 18) + 'Bağlan</button></div><div class="err" id="acErr" aria-live="polite"></div></div>';
    var args = function () { return { address: addr, game: $('#acGame').value, loader: $('#acLoader').value, version: null }; };
    $('#acPrep').addEventListener('click', function () {
      var b = $('#acPrep'); b.disabled = true; $('#acErr').textContent = '';
      inv('connect_prepare', args()).then(function (i) { toast('Instance hazır: ' + i.name); return refresh(); })
        .catch(function (e) { $('#acErr').textContent = errText(e); }).then(function () { b.disabled = false; });
    });
    $('#acGo').addEventListener('click', function () {
      if (!cfg.player) { openLogin(); toast('Önce oyuncu adını belirle'); return; }
      $('#acErr').textContent = ''; openLaunch(p.host);
      inv('connect_launch', args()).catch(launchFail);
    });
  }

  /* ---------- içerik gezgini: mod, kaynak paketi, shader ---------- */
  var BR = {
    mod: { view: '#v-mods', hash: 'mods', title: 'Mod gezgini', verb: 'mod', cats: [['', 'Tümü'], ['optimization', 'Performans'], ['technology', 'Teknoloji'], ['adventure', 'Macera'], ['decoration', 'Dekorasyon'], ['utility', 'Araçlar'], ['library', 'Kütüphane'], ['worldgen', 'Dünya üretimi']],
      desc: 'Modrinth kataloğunda ara. Seçtiğin instance\'ın Minecraft sürümüne ve yükleyicisine (Fabric, Quilt, Forge, NeoForge) uyan en yeni dosya ve gerekli bağımlılıkları eklenir.',
      filter: function (i) { return i.loader !== 'vanilla' && !i.managed; },
      empty: 'Önce modlu bir instance gerek|Modlar bir yükleyiciyle çalışır: Fabric, Quilt, Forge ya da NeoForge. Yeni instance oluştururken bir yükleyici seç.' },
    resourcepack: { view: '#v-resources', hash: 'resources', title: 'Kaynak paketleri', verb: 'paket', cats: [['', 'Tümü'], ['vanilla-like', 'Vanilla benzeri'], ['realistic', 'Gerçekçi'], ['simplistic', 'Basit'], ['decoration', 'Dekorasyon'], ['combat', 'Savaş'], ['16x', '16x'], ['32x', '32x'], ['64x', '64x']],
      desc: 'Doku, ses ve model paketleri Modrinth\'ten. Seçtiğin instance\'ın Minecraft sürümüne uyan en yeni dosya resourcepacks klasörüne iner. Oyunda Seçenekler → Kaynak Paketleri\'nden etkinleştir.',
      filter: function () { return true; }, empty: 'Önce bir instance gerek|Kaynak paketini koyacağımız bir instance oluştur.' },
    shader: { view: '#v-shaders', hash: 'shaders', title: 'Shader paketleri', verb: 'shader', cats: [['', 'Tümü'], ['realistic', 'Gerçekçi'], ['semi-realistic', 'Yarı gerçekçi'], ['cartoon', 'Çizgi film'], ['fantasy', 'Fantastik'], ['vanilla-like', 'Vanilla benzeri'], ['potato', 'Düşük donanım']],
      desc: 'Işık ve gölge paketleri. Dosya instance\'ın shaderpacks klasörüne iner. Shader\'lar Iris (Fabric/Quilt/NeoForge), Oculus (Forge) ya da OptiFine gerektirir; oyunda Seçenekler → Video → Shader Paketleri\'nden seç.',
      filter: function () { return true; }, empty: 'Önce bir instance gerek|Shader paketini koyacağımız bir instance oluştur.' }
  };
  var bst = { mod: { cat: '', target: '', loaded: false, added: {} }, resourcepack: { cat: '', target: '', loaded: false, added: {} }, shader: { cat: '', target: '', loaded: false, added: {} } };
  function brInstances(kind) { return instances.filter(BR[kind].filter); }
  function irisHint(kind, inst) {
    if (kind !== 'shader' || !inst) return '';
    if (inst.loader === 'vanilla') return '<div class="callout">' + kz.ic('shield', 20) + '<div>Bu instance modsuz (Vanilla): shader için Iris ya da OptiFine gerekir. Fabric\'li bir instance oluşturup Iris\'i kurabilirsin.</div></div>';
    if (inst.loader === 'forge') return '<div class="callout">' + kz.ic('shield', 20) + '<div>Forge\'ta shader için <b>Oculus</b> modu gerekir. Mod gezgininden "oculus" ara ve ekle.</div></div>';
    if (inst.managed) return '';
    return '<div class="callout">' + kz.ic('shield', 20) + '<div>Shader\'ların çalışması için <b>Iris</b> modu gerekir. <button class="btn sm" data-iris="1">' + kz.ic('download', 14) + 'Iris\'i bu instance\'a kur</button></div></div>';
  }
  function renderBrowse(kind) {
    var c = BR[kind], st = bst[kind], fi = brInstances(kind), v = $(c.view);
    if (!fi.some(function (i) { return i.id === st.target; })) st.target = fi.length ? fi[0].id : '';
    if (!fi.length) {
      var e = c.empty.split('|');
      v.innerHTML = '<div class="ph"><div><h1>' + c.title + '</h1></div></div><div class="card view-note"><h2>' + e[0] + '</h2><p style="margin:8px 0 14px;color:var(--ink-2)">' + e[1] + '</p><button class="btn primary" data-newinst="1">' + kz.ic('plus', 18) + 'Instance oluştur</button></div>';
      return;
    }
    var cur = fi.filter(function (i) { return i.id === st.target; })[0];
    v.innerHTML = '<div class="ph"><div><h1>' + c.title + '</h1><p>' + esc(c.desc) + '</p></div></div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="joinbar"><input id="q-' + kind + '" class="in" style="font-family:var(--font-body)" placeholder="Ara..." aria-label="Ara"><button class="btn primary" data-bgo="' + kind + '">' + kz.ic('refresh', 18) + 'Ara</button></div>' +
      '<div class="chips" role="group" aria-label="Kategori">' + c.cats.map(function (x) { return '<button class="chip" aria-pressed="' + (x[0] === st.cat) + '" data-bcat="' + kind + '" data-c="' + x[0] + '">' + x[1] + '</button>'; }).join('') + '</div>' +
      '<div class="field"><label for="i-' + kind + '">Eklenecek instance</label><select id="i-' + kind + '" class="in" data-binst="' + kind + '">' + fi.map(function (i) { return '<option value="' + esc(i.id) + '"' + (i.id === st.target ? ' selected' : '') + '>' + esc(i.name) + ' (' + esc(i.game) + ' · ' + esc(loaderName(i.loader)) + ')</option>'; }).join('') + '</select></div>' +
      irisHint(kind, cur) + '<div class="err" id="e-' + kind + '" aria-live="polite"></div></div><div class="mods" id="l-' + kind + '"></div>';
    var q = $('#q-' + kind); q.addEventListener('keydown', function (e) { if (e.key === 'Enter') searchBrowse(kind); });
    searchBrowse(kind);
  }
  function sideLabel(h) { return h.server_side === 'unsupported' ? 'Yalnızca istemci' : h.client_side === 'unsupported' ? 'Yalnızca server' : 'İstemci + server'; }
  function searchBrowse(kind) {
    var st = bst[kind], list = $('#l-' + kind); $('#e-' + kind).textContent = ''; list.innerHTML = '<div class="card hint">Aranıyor...</div>';
    inv('search_content', { instance: st.target, kind: kind, query: $('#q-' + kind).value.trim(), category: st.cat || null }).then(function (hits) {
      if (!hits.length) { list.innerHTML = '<div class="card hint">Eşleşen sonuç yok. Aramayı kısaltmayı ya da kategoriyi "Tümü" yapmayı dene.</div>'; return; }
      list.innerHTML = hits.map(function (h) {
        var done = st.added[st.target + '|' + h.project_id];
        var icon = h.icon_url ? '<img class="ic" src="' + esc(h.icon_url) + '" alt="" width="48" height="48">' : '<div class="ic" style="background:var(--panel-2);display:grid;place-items:center;font-weight:800">' + esc((h.title || '?').charAt(0).toUpperCase()) + '</div>';
        return '<article class="card mod">' + icon + '<div class="tx"><h3>' + esc(h.title) + '</h3><p>' + esc(h.description) + '</p><div class="acts">' + (kind === 'mod' ? '<span class="pill mute">' + esc(sideLabel(h)) + '</span>' : '') +
          h.categories.slice(0, 3).map(function (c) { return '<span class="pill mute">' + esc(c) + '</span>'; }).join('') + '</div></div>' +
          '<button class="btn sm' + (done ? '' : ' primary') + '" style="align-self:flex-start" data-add="' + esc(h.project_id) + '" data-kind="' + kind + '"' + (done ? ' disabled' : '') + '>' + (done ? 'Eklendi' : 'Ekle') + '</button></article>';
      }).join('');
    }).catch(function (e) { list.innerHTML = ''; $('#e-' + kind).textContent = errText(e); });
  }
  function addContent(btn, kindOverride, projectOverride) {
    var kind = kindOverride || btn.dataset.kind, st = bst[kind], id = projectOverride || btn.dataset.add, label = btn.textContent;
    btn.disabled = true; btn.textContent = 'Ekleniyor...';
    inv('install_content', { instance: st.target, kind: kind === 'mod' || projectOverride ? 'mod' : kind, project: id }).then(function (names) {
      st.added[st.target + '|' + id] = true; btn.textContent = 'Eklendi'; btn.classList.remove('primary');
      toast(names.length ? names.join(', ') + ' eklendi' : 'Zaten ekli'); return refresh(false);
    }).catch(function (e) { btn.disabled = false; btn.textContent = label; var el = $('#e-' + kind); if (el) el.textContent = errText(e); else toast(errText(e)); });
  }
  var HASHKIND = { mods: 'mod', resources: 'resourcepack', shaders: 'shader' };
  addEventListener('hashchange', function () {
    var k = HASHKIND[location.hash.slice(1)];
    if (k && !bst[k].loaded) { bst[k].loaded = true; renderBrowse(k); }
    if (location.hash === '#skins') renderSkins();
  });

  /* ---------- skinler ---------- */
  var skinList = [];
  var instSkinTarget = '';
  function drawSkin(cv, dataUrl) {
    var img = new Image();
    img.onload = function () {
      var x = cv.getContext('2d'); cv.width = 16; cv.height = 32; x.imageSmoothingEnabled = false; x.clearRect(0, 0, 16, 32);
      var legacy = img.height === 32;
      var part = function (sx, sy, w, h, dx, dy, flip) {
        if (!flip) { x.drawImage(img, sx, sy, w, h, dx, dy, w, h); return; }
        x.save(); x.translate(dx + w, dy); x.scale(-1, 1); x.drawImage(img, sx, sy, w, h, 0, 0, w, h); x.restore();
      };
      part(8, 8, 8, 8, 4, 0); part(20, 20, 8, 12, 4, 8); part(44, 20, 4, 12, 0, 8); part(4, 20, 4, 12, 4, 20);
      if (legacy) { part(44, 20, 4, 12, 12, 8, true); part(4, 20, 4, 12, 8, 20, true); }
      else { part(36, 52, 4, 12, 12, 8); part(20, 52, 4, 12, 8, 20); part(20, 36, 8, 12, 4, 8); part(44, 36, 4, 12, 0, 8); part(52, 52, 4, 12, 12, 8); part(4, 36, 4, 12, 4, 20); part(4, 52, 4, 12, 8, 20); }
      part(40, 8, 8, 8, 4, 0);
    };
    img.src = dataUrl;
  }
  function renderSkins() {
    inv('list_skins').then(function (l) { skinList = l; drawSkinsPage(); }).catch(function (e) { toast(errText(e)); });
  }
  function drawSkinsPage() {
    var mi = instances.filter(function (i) { return i.loader !== 'vanilla' && !i.managed; });
    if (!mi.some(function (i) { return i.id === instSkinTarget; })) instSkinTarget = mi.length ? mi[0].id : '';
    var cards = skinList.map(function (s) {
      var on = cfg.skin === s.id;
      return '<article class="card skin' + (on ? ' on' : '') + '"><canvas data-sid="' + esc(s.id) + '" aria-label="' + esc(s.name) + ' önizleme"></canvas><div><b>' + esc(s.name) + '</b><div class="hint">' + esc(s.source) + '</div></div>' +
        '<div style="display:flex;gap:6px;flex-wrap:wrap;justify-content:center"><button class="btn sm' + (on ? '' : ' primary') + '" data-skin-use="' + esc(s.id) + '"' + (on ? ' disabled' : '') + '>' + (on ? 'Seçili' : 'Kullan') + '</button>' +
        (on ? '<button class="btn sm" data-skin-use="">Seçimi kaldır</button>' : '') + '<button class="btn sm danger" data-skin-del="' + esc(s.id) + '">Sil</button></div></article>';
    }).join('');
    $('#v-skins').innerHTML =
      '<div class="ph"><div><h1>Skinler</h1><p>Skin kitaplığın. Dosyadan ya da bir oyuncunun adıyla ekle, birini seç; oyun açılırken seçili skin instance\'a yazılır.</p></div></div>' +
      '<div class="card" style="display:flex;flex-direction:column;gap:14px"><h2 style="margin:0">Skin ekle</h2><div class="joinbar">' +
      '<div class="field" style="flex:1;min-width:220px"><label for="skinFile">PNG dosyasından (64x64 ya da 64x32)</label><input id="skinFile" class="in" type="file" accept="image/png"></div>' +
      '<div class="field" style="flex:1;min-width:220px"><label for="skinUser">Oyuncu adıyla (Mojang\'dan indirir)</label><div class="joinbar"><input id="skinUser" class="in" maxlength="16" placeholder="örn. Notch" autocomplete="off"><button class="btn primary" id="skinFetch">' + kz.ic('download', 16) + 'İndir</button></div></div></div>' +
      '<div class="err" id="skinErr" aria-live="polite"></div></div>' +
      (skinList.length ? '<div class="skin-grid">' + cards + '</div>' : '<div class="card hint">Henüz skin yok. Yukarıdan bir dosya seç ya da bir oyuncu adıyla indir.</div>') +
      '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Skin oyunda görünsün</h2>' +
      '<div class="callout">' + kz.ic('shield', 20) + '<div>Çevrimdışı hesaplarda vanilla Minecraft skin göstermez. Seçtiğin skinin oyunda görünmesi için instance\'a <b>CustomSkinLoader</b> modu gerekir. Skin yalnızca kendi ekranında görünür, server\'daki diğer oyuncular görmez (onların da aynı modu kullanması gerekir). Microsoft girişi gelince gerçek skin yüklemesi eklenecek.</div></div>' +
      (mi.length ? '<div class="joinbar"><select id="skinInst" class="in" style="flex:1;min-width:220px">' + mi.map(function (i) { return '<option value="' + esc(i.id) + '"' + (i.id === instSkinTarget ? ' selected' : '') + '>' + esc(i.name) + ' (' + esc(i.game) + ' · ' + esc(loaderName(i.loader)) + ')</option>'; }).join('') + '</select><button class="btn" id="skinCsl">' + kz.ic('download', 16) + 'CustomSkinLoader\'ı kur</button></div>' : '<p class="hint">Modlu (Fabric, Quilt, Forge, NeoForge) bir instance oluşturunca buradan modu kurabilirsin.</p>') + '</div>';
    $$('#v-skins canvas[data-sid]').forEach(function (cv) { inv('skin_data', { id: cv.dataset.sid }).then(function (u) { drawSkin(cv, u); }); });
    var file = $('#skinFile');
    file.addEventListener('change', function () {
      var f = file.files[0]; if (!f) return; $('#skinErr').textContent = '';
      var rd = new FileReader();
      rd.onload = function () {
        inv('add_skin', { name: f.name.replace(/\.[^.]+$/, '').slice(0, 40) || 'Skin', data: String(rd.result) }).then(function () { toast('Skin eklendi'); renderSkins(); })
          .catch(function (e) { $('#skinErr').textContent = errText(e); });
      };
      rd.readAsDataURL(f);
    });
    $('#skinFetch').addEventListener('click', function () {
      var b = $('#skinFetch'); b.disabled = true; $('#skinErr').textContent = '';
      inv('fetch_skin', { player: $('#skinUser').value.trim() }).then(function (s) { toast(s.name + ' skini eklendi'); renderSkins(); })
        .catch(function (e) { $('#skinErr').textContent = errText(e); b.disabled = false; });
    });
    var csl = $('#skinCsl');
    if (csl) {
      $('#skinInst').addEventListener('change', function () { instSkinTarget = this.value; });
      csl.addEventListener('click', function () { bst.mod.target = instSkinTarget; addContent(csl, 'mod', 'customskinloader'); });
    }
  }
  document.addEventListener('click', function (e) {
    var u = e.target.closest('[data-skin-use]');
    if (u) { inv('select_skin', { id: u.dataset.skinUse || null }).then(function (c) { cfg = c; toast(c.skin ? 'Skin seçildi' : 'Skin seçimi kaldırıldı'); drawSkinsPage(); }).catch(function (er) { toast(errText(er)); }); return; }
    var d = e.target.closest('[data-skin-del]');
    if (d) { inv('delete_skin', { id: d.dataset.skinDel }).then(function () { return refresh(false); }).then(renderSkins).catch(function (er) { toast(errText(er)); }); }
  });

  /* ---------- ortak tıklamalar ---------- */
  $('#main').addEventListener('click', function (e) {
    var d = e.target.closest('[data-del]'); if (d) { openDelete(d.dataset.del); return; }
    var rs = e.target.closest('[data-resync]'); if (rs) { resync(rs.dataset.resync); return; }
    var p = e.target.closest('[data-play]'); if (p) { play(p.dataset.play); return; }
    if (e.target.closest('[data-newinst]')) { openNew(); return; }
    var o = e.target.closest('[data-open]'); if (o) { inv('open_instance_folder', { id: o.dataset.open }).catch(function (er) { toast(errText(er)); }); return; }
    var go = e.target.closest('[data-bgo]'); if (go) { searchBrowse(go.dataset.bgo); return; }
    var bc = e.target.closest('[data-bcat]');
    if (bc) { var k = bc.dataset.bcat; bst[k].cat = bc.dataset.c; $$('[data-bcat="' + k + '"]').forEach(function (x) { x.setAttribute('aria-pressed', x === bc); }); searchBrowse(k); return; }
    var ir = e.target.closest('[data-iris]'); if (ir) { addContent(ir, 'shader', 'iris'); return; }
    var a = e.target.closest('[data-add]'); if (a) { addContent(a); }
  });

  document.addEventListener('change', function (e) {
    var sel = e.target.closest('[data-binst]'); if (!sel) return;
    var k = sel.dataset.binst; bst[k].target = sel.value; renderBrowse(k);
  });

  function refresh(rerender) {
    return Promise.all([inv('get_config'), inv('get_instances')]).then(function (r) {
      cfg = r[0]; instances = r[1]; renderAccount(); renderHome(); renderLibrary();
      if (rerender !== false) ['mod', 'resourcepack', 'shader'].forEach(function (k) { if (bst[k].loaded) renderBrowse(k); });
      if (rerender !== false && location.hash === '#skins') drawSkinsPage();
    }).catch(function (e) { toast(errText(e)); });
  }
  refresh().then(function () { if (!cfg.player) openLogin(); });
})();
