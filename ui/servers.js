/* Serverlar sayfası: katıldığın server'lar (canlı durum), kendi server'ını kur ve yönet, Discord ayarı.
   Yalnızca Tauri içinde ve live.js yüklendikten sonra çalışır. Dışarıdan gelen her metin esc() ya da textContent ile yazılır. */
(function () {
  'use strict';
  function boot() {
    var L = window.__kz && window.__kz.live;
    if (!L) return;
    var kz = window.__kz, inv = L.inv, listen = L.listen, esc = L.esc, $ = L.$, $$ = L.$$, toast = L.toast, errText = L.errText;
    var host = $('#v-servers');
    Array.prototype.slice.call(host.children).forEach(function (c) { c.style.display = 'none'; });
    host.insertAdjacentHTML('beforeend', '<div id="srvRoot"></div>');
    var view = $('#srvRoot');
    var hosts = [], joined = [], sel = null, tab = 'console', logs = {}, claims = {}, statusT = null, statusCache = {};
    var KIND = { vanilla: 'Vanilla', paper: 'Paper (eklentili)', fabric: 'Fabric', forge: 'Forge', neoforge: 'NeoForge' };
    var STATE = { stopped: ['off', 'Kapalı'], starting: ['mid', 'Başlıyor'], running: ['on', 'Çalışıyor'], stopping: ['mid', 'Kapanıyor'] };

    function hostById(id) { return hosts.filter(function (h) { return h.id === id; })[0]; }
    function dot(cls, title) { return '<span class="sdot ' + cls + '" title="' + esc(title) + '"></span>'; }
    function addrOf(h) { return h.public_address || ''; }

    /* ---------- liste ---------- */
    function load() {
      return Promise.all([inv('list_servers'), inv('host_list')]).then(function (r) { joined = r[0]; hosts = r[1]; draw(); }).catch(function (e) { toast(errText(e)); });
    }
    function draw() {
      if (sel && hostById(sel)) return drawDetail();
      sel = null;
      var jcards = joined.map(function (s) {
        var key = s.host + ':' + s.port, c = statusCache[key];
        var d = c ? (c.online ? dot('on', 'Çevrimiçi') : dot('off', 'Çevrimdışı')) : dot('', 'Kontrol ediliyor');
        var btn = s.managed ? '<button class="btn primary sm" data-resync="' + esc(s.instance) + '">' + kz.ic('refresh', 14) + 'Güncelle ve katıl</button>' : '<button class="btn primary sm" data-play="' + esc(s.instance) + '">' + kz.ic('play', 14) + 'Katıl</button>';
        return '<article class="card srv"><div class="top"><span data-sk="' + esc(key) + '">' + d + '</span><b style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis">' + esc(s.name) + '</b></div>' +
          '<div class="hint mono" style="word-break:break-all">' + esc(s.host + (s.port !== 25565 ? ':' + s.port : '')) + '</div>' +
          '<div class="hint" data-si="' + esc(key) + '">' + (c && c.online ? esc(c.players + ' oyuncu · ' + c.version) : (c ? 'Çevrimdışı ya da uyuyor' : 'Kontrol ediliyor...')) + '</div>' +
          '<div style="display:flex;gap:8px;flex-wrap:wrap"><span class="pill mute">' + esc(s.game) + '</span><span class="pill mute">' + esc(L.loaderName(s.loader)) + '</span></div>' + btn + '</article>';
      }).join('');
      var hcards = hosts.map(function (h) {
        var st = STATE[h.state] || STATE.stopped;
        return '<article class="card srv"><div class="top">' + dot(st[0], st[1]) + '<b style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis">' + esc(h.name) + '</b><span class="pill mute">' + esc(h.mode === 'local' ? 'Bu bilgisayar' : 'Ayrı makine') + '</span></div>' +
          '<div style="display:flex;gap:8px;flex-wrap:wrap"><span class="pill mute">' + esc(KIND[h.kind] || h.kind) + '</span><span class="pill mute">' + esc(h.game) + '</span><span class="pill mute">Port ' + h.port + '</span></div>' +
          '<div class="hint mono">' + esc(addrOf(h) || 'Dış adres henüz yok') + '</div>' +
          '<button class="btn primary sm" data-hm="' + esc(h.id) + '">Yönet</button></article>';
      }).join('');
      view.innerHTML =
        '<div class="ph"><div><h1>Serverlar</h1><p>Katıldığın server\'lar ve kendi kurduğun server\'lar. Yeşil nokta çevrimiçi, kırmızı çevrimdışı demek.</p></div><button class="btn primary" id="hNew">' + kz.ic('plus', 18) + 'Server kur</button></div>' +
        '<h2 style="margin:18px 0 10px">Katıldığım server\'lar</h2>' +
        (joined.length ? '<div class="srv-grid">' + jcards + '</div>' : '<div class="card hint">Henüz bir server\'a katılmadın. <b>Server\'a Katıl</b> sayfasından bir davet bağlantısı ya da adres kullan.</div>') +
        '<h2 style="margin:22px 0 10px">Benim server\'larım</h2>' +
        (hosts.length ? '<div class="srv-grid">' + hcards + '</div>' : '<div class="card hint">Kendi server\'ını kurmak için sağ üstteki <b>Server kur</b> düğmesine bas.</div>');
      $('#hNew').addEventListener('click', openCreate);
      refreshStatuses();
    }

    function refreshStatuses() {
      joined.forEach(function (s) {
        var key = s.host + ':' + s.port;
        inv('server_status', { host: s.host, port: s.port }).then(function (r) {
          statusCache[key] = r;
          $$('[data-sk]').filter(function (n) { return n.dataset.sk === key; }).forEach(function (n) { n.innerHTML = r.online ? dot('on', 'Çevrimiçi') : dot('off', 'Çevrimdışı'); });
          $$('[data-si]').filter(function (n) { return n.dataset.si === key; }).forEach(function (n) { n.textContent = r.online ? r.players + ' oyuncu · ' + r.version : 'Çevrimdışı ya da uyuyor'; });
        }).catch(function () { });
      });
    }

    /* ---------- kurma sihirbazı ---------- */
    function openCreate() {
      var m = $('#hModal');
      if (!m) {
        document.body.insertAdjacentHTML('beforeend',
          '<div class="modal" id="hModal" hidden role="dialog" aria-modal="true" aria-labelledby="hTitle"><div class="sheet" style="max-height:92vh;overflow:auto">' +
          '<h2 id="hTitle">Server kur</h2>' +
          '<div class="field"><label for="hName">Server adı</label><input id="hName" class="in" maxlength="40" placeholder="örn. Dostlar SMP" autocomplete="off"></div>' +
          '<div class="field"><label>Nerede çalışacak?</label><div class="seg" id="hWhere" style="grid-template-columns:repeat(2,1fr)"><button aria-pressed="true" data-w="local">Bu bilgisayarda</button><button aria-pressed="false" data-w="remote">Ayrı bir makinede</button></div>' +
          '<p class="hint" id="hWhereNote"></p></div>' +
          '<div class="hform"><div class="field"><label for="hKind">Tür</label><select id="hKind" class="in">' + Object.keys(KIND).map(function (k) { return '<option value="' + k + '">' + esc(KIND[k]) + '</option>'; }).join('') + '</select></div>' +
          '<div class="field"><label for="hGame">Minecraft sürümü</label><select id="hGame" class="in"><option>Yükleniyor...</option></select></div>' +
          '<div class="field"><label for="hPort">Port</label><input id="hPort" class="in" type="number" min="1024" max="65535" value="25565"></div>' +
          '<div class="field"><label for="hRam">RAM (MB)</label><input id="hRam" class="in" type="number" min="512" max="65536" step="256" value="4096"></div></div>' +
          '<label class="chk" id="hUpnpW"><input type="checkbox" id="hUpnp" checked> Router\'da portu otomatik aç (internetten bağlanılsın)</label>' +
          '<label class="chk"><input type="checkbox" id="hCracked" checked> Karma (çevrimdışı) hesaplar da girebilsin</label>' +
          '<p class="hint">Launcher\'da şimdilik yalnızca çevrimdışı hesap var. Karma girişi açık server\'a kötü niyetli biri başka birinin adıyla girebilir; yalnızca tanıdıklarınla oynayacaksan aç ve whitelist kullan.</p>' +
          '<label class="chk"><input type="checkbox" id="hEula"> <span><a href="#" data-eula="1" style="text-decoration:underline">Minecraft EULA</a> şartlarını okudum ve kabul ediyorum.</span></label>' +
          '<div class="err" id="hErr" aria-live="polite"></div>' +
          '<div style="display:flex;gap:10px;justify-content:flex-end"><button class="btn" id="hCancel">Vazgeç</button><button class="btn primary" id="hGo">' + kz.ic('download', 16) + 'Kur</button></div></div></div>');
        m = $('#hModal');
        $('#hCancel').addEventListener('click', function () { m.hidden = true; });
        $('#hWhere').addEventListener('click', function (e) { var b = e.target.closest('[data-w]'); if (b) setWhere(b.dataset.w); });
        $('[data-eula]', m).addEventListener('click', function (e) { e.preventDefault(); toast('Şartlar: aka.ms/MinecraftEULA (tarayıcında aç)'); });
        $('#hGo').addEventListener('click', create);
      }
      $('#hErr').textContent = '';
      setWhere('local');
      L.versions(function (l) { $('#hGame').innerHTML = l.map(function (v) { return '<option>' + esc(v) + '</option>'; }).join(''); });
      m.hidden = false; $('#hName').focus();
    }
    function curWhere() { return $('#hWhere [aria-pressed=true]').dataset.w; }
    function setWhere(w) {
      $$('#hWhere button').forEach(function (b) { b.setAttribute('aria-pressed', b.dataset.w === w); });
      $('#hUpnpW').style.display = w === 'local' ? 'flex' : 'none';
      $('#hWhereNote').textContent = w === 'local'
        ? 'Server bu bilgisayarda çalışır; Kazma içinden başlatır, konsolunu gösterir ve istersen internetten bağlanabilmesi için portu açar.'
        : 'Kazma server dosyalarını hazırlar ve bir zip paketi (başlatma betikleriyle) üretir. Paketi ayrı makineye kopyalayıp orada çalıştırırsın. Launcher o makineyi uzaktan yönetmez.';
    }
    function create() {
      var err = $('#hErr'); err.textContent = '';
      if (!$('#hEula').checked) { err.textContent = 'Devam etmek için EULA\'yı kabul etmelisin.'; return; }
      var req = { name: $('#hName').value.trim(), kind: $('#hKind').value, game: $('#hGame').value, port: parseInt($('#hPort').value, 10) || 25565, ram: parseInt($('#hRam').value, 10) || 4096, mode: curWhere(), upnp: curWhere() === 'local' && $('#hUpnp').checked, eula: true, cracked: $('#hCracked').checked };
      if (!req.name) { err.textContent = 'Server adı yaz.'; return; }
      $('#hModal').hidden = true; L.openLaunch(req.name + ' kuruluyor');
      inv('host_create', { req: req }).then(function (spec) {
        L.doneStep(); $('#launchModal').hidden = true; toast(spec.name + ' hazır'); sel = spec.id; tab = 'console'; return load();
      }).catch(function (e) { L.launchFail(e); });
    }

    /* ---------- yönetim paneli ---------- */
    function stateBtns(h) {
      if (h.mode === 'remote') return '<button class="btn primary" id="hExport">' + kz.ic('download', 16) + 'Paket oluştur (zip)</button><button class="btn" id="hExports">' + kz.ic('folder', 16) + 'Paketler klasörü</button>';
      var s = h.state;
      return '<button class="btn primary" data-ha="start"' + (s !== 'stopped' ? ' disabled' : '') + '>' + kz.ic('play', 16) + 'Başlat</button>' +
        '<button class="btn" data-ha="restart"' + (s !== 'running' ? ' disabled' : '') + '>' + kz.ic('refresh', 16) + 'Yeniden başlat</button>' +
        '<button class="btn danger" data-ha="stop"' + (s !== 'running' && s !== 'starting' ? ' disabled' : '') + '>' + kz.ic('stop', 16) + 'Kapat</button>';
    }
    function drawDetail() {
      var h = hostById(sel), st = STATE[h.state] || STATE.stopped;
      var tabs = [['console', 'Konsol'], ['settings', 'Ayarlar'], ['mods', h.kind === 'paper' ? 'Eklentiler' : 'Modlar'], ['net', 'Bağlantı']].filter(function (t) { return !(h.mode === 'remote' && t[0] === 'console'); });
      if (h.mode === 'remote' && tab === 'console') tab = 'settings';
      view.innerHTML =
        '<div class="ph"><div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><button class="btn sm" id="hBack">← Serverlar</button>' + dot(st[0], st[1]) + '<h1 style="margin:0">' + esc(h.name) + '</h1><span class="pill mute">' + esc(KIND[h.kind] || h.kind) + ' ' + esc(h.game) + '</span><span class="pill ' + (h.state === 'running' ? 'good' : 'mute') + '" id="hState">' + esc(st[1]) + '</span></div>' +
        '<div style="display:flex;gap:8px;flex-wrap:wrap" id="hBtns">' + stateBtns(h) + '</div></div>' +
        '<div class="tabs" role="tablist">' + tabs.map(function (t) { return '<button role="tab" data-ht="' + t[0] + '" aria-selected="' + (tab === t[0]) + '">' + t[1] + '</button>'; }).join('') + '</div>' +
        '<div id="hBody" style="display:flex;flex-direction:column;gap:14px;margin-top:14px"></div>';
      $('#hBack').addEventListener('click', function () { sel = null; load(); });
      drawTab(h);
    }
    function actErr(e) { toast(errText(e)); }
    function drawTab(h) {
      var b = $('#hBody');
      if (tab === 'console') return tabConsole(h, b);
      if (tab === 'settings') return tabSettings(h, b);
      if (tab === 'mods') return tabMods(h, b);
      return tabNet(h, b);
    }
    function tabConsole(h, b) {
      b.innerHTML = '<div class="hcon" id="hcon" role="log" aria-live="off" tabindex="0"></div><div class="joinbar"><input id="hCmd" class="in" style="flex:1" placeholder="Komut yaz (örn. list, say merhaba, op oyuncu)" autocomplete="off"><button class="btn primary" id="hSend">Gönder</button></div>' +
        '<div style="display:flex;gap:8px;flex-wrap:wrap"><button class="btn sm" data-q="list">Oyuncular</button><button class="btn sm" data-q="save-all">Dünyayı kaydet</button><button class="btn sm" data-q="whitelist list">Whitelist</button><button class="btn sm" data-o="1">' + kz.ic('folder', 14) + 'Server klasörü</button></div>';
      var con = $('#hcon');
      (logs[h.id] || []).forEach(function (l) { addLine(con, l); });
      inv('host_log', { id: h.id }).then(function (l) { if (!(logs[h.id] || []).length) { logs[h.id] = l.slice(); con.textContent = ''; l.forEach(function (x) { addLine(con, x); }); } });
      var send = function (t) { if (!t) return; inv('host_send', { id: h.id, command: t }).then(function () { addLine(con, '> ' + t); }).catch(actErr); };
      $('#hSend').addEventListener('click', function () { var i = $('#hCmd'); send(i.value.trim()); i.value = ''; });
      $('#hCmd').addEventListener('keydown', function (e) { if (e.key === 'Enter') { $('#hSend').click(); } });
      $$('[data-q]', b).forEach(function (x) { x.addEventListener('click', function () { send(x.dataset.q); }); });
      $('[data-o]', b).addEventListener('click', function () { inv('host_open_folder', { id: h.id }).catch(actErr); });
    }
    function addLine(con, text) {
      var near = con.scrollTop + con.clientHeight >= con.scrollHeight - 40;
      con.appendChild(document.createTextNode(text + '\n'));
      while (con.childNodes.length > 2000) con.removeChild(con.firstChild);
      if (near) con.scrollTop = con.scrollHeight;
    }

    var PROPS = [['motd', 'Server açıklaması (MOTD)', 'text'], ['max-players', 'En çok oyuncu', 'number'], ['difficulty', 'Zorluk', ['peaceful', 'easy', 'normal', 'hard']], ['gamemode', 'Oyun modu', ['survival', 'creative', 'adventure', 'spectator']],
      ['view-distance', 'Görüş mesafesi', 'number'], ['simulation-distance', 'Simülasyon mesafesi', 'number'], ['level-name', 'Dünya klasörü adı', 'text'], ['spawn-protection', 'Başlangıç koruması', 'number']];
    var FLAGS = [['online-mode', 'Premium giriş zorunlu (kapalıysa karma/çevrimdışı hesaplar girebilir)'], ['pvp', 'PvP açık'], ['white-list', 'Whitelist (yalnızca izinli oyuncular)'], ['allow-flight', 'Uçmaya izin ver'], ['enable-command-block', 'Komut bloğu'], ['hardcore', 'Hardcore']];
    function tabSettings(h, b) {
      inv('host_props', { id: h.id }).then(function (p) {
        b.innerHTML = '<div class="card" style="display:flex;flex-direction:column;gap:14px"><div class="hform">' + PROPS.map(function (r) {
          var v = p[r[0]] == null ? '' : p[r[0]];
          if (Array.isArray(r[2])) return '<div class="field"><label>' + r[1] + '</label><select class="in" data-p="' + r[0] + '">' + r[2].map(function (o) { return '<option' + (o === v ? ' selected' : '') + '>' + o + '</option>'; }).join('') + '</select></div>';
          return '<div class="field"><label>' + r[1] + '</label><input class="in" data-p="' + r[0] + '" type="' + r[2] + '" value="' + esc(v) + '"></div>';
        }).join('') + '</div>' + FLAGS.map(function (f) {
          return '<label class="chk"><input type="checkbox" data-pf="' + f[0] + '"' + (p[f[0]] === 'true' || (p[f[0]] == null && f[0] === 'online-mode') ? ' checked' : '') + '> ' + f[1] + '</label>';
        }).join('') + '<div class="err" id="hpErr"></div><p class="hint">Ayarlar server yeniden başlayınca geçerli olur.</p>' +
          '<div style="display:flex;gap:8px"><button class="btn primary" id="hpSave">Kaydet</button></div></div>' +
          '<div class="card" style="display:flex;flex-direction:column;gap:10px"><h2 style="margin:0">Tehlikeli bölge</h2><p class="hint">Server\'ı siler (dünya dosyaları dahil). Geri alınamaz.</p><div><button class="btn danger" id="hDel"' + (h.state !== 'stopped' ? ' disabled' : '') + '>Server\'ı sil</button></div></div>';
        $('#hpSave').addEventListener('click', function () {
          var u = {}; $$('[data-p]', b).forEach(function (x) { if (x.value !== '') u[x.dataset.p] = x.value; });
          $$('[data-pf]', b).forEach(function (x) { u[x.dataset.pf] = x.checked ? 'true' : 'false'; });
          inv('host_set_props', { id: h.id, updates: u }).then(function () { toast('Ayarlar kaydedildi'); }).catch(function (e) { $('#hpErr').textContent = errText(e); });
        });
        $('#hDel').addEventListener('click', function () {
          if (!confirm('"' + h.name + '" server\'ı ve dünyası silinsin mi? Bu geri alınamaz.')) return;
          inv('host_delete', { id: h.id }).then(function () { sel = null; return load(); }).catch(actErr);
        });
      }).catch(function (e) { b.innerHTML = '<div class="card hint">Ayar dosyası henüz yok. Server\'ı bir kez başlat.</div>'; });
    }

    function tabMods(h, b) {
      if (h.kind === 'vanilla') { b.innerHTML = '<div class="card hint">Vanilla server mod ya da eklenti desteklemez. Mod/eklenti için Paper, Fabric, Forge veya NeoForge türünde yeni bir server kur.</div>'; return; }
      var plug = h.kind === 'paper';
      b.innerHTML = '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">' + (plug ? 'Eklenti' : 'Mod') + ' ekle (Modrinth)</h2>' +
        '<div class="joinbar"><input id="hmQ" class="in" style="flex:1" placeholder="Ara (örn. luckperms, lithium)" autocomplete="off"><button class="btn primary" id="hmGo">Ara</button></div><div class="err" id="hmErr"></div><div id="hmOut" style="display:flex;flex-direction:column;gap:8px"></div>' +
        '<p class="hint">Yalnızca bu server\'ın sürümüne ve yükleyicisine uyan dosyalar indirilir, SHA-512 ile doğrulanır. Yalnızca istemcide çalışan modlar server\'ı bozabilir; "server: unsupported" yazanları ekleme.</p></div>' +
        (plug ? '' : '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Oyuncular için otomatik mod eşitleme</h2><p class="hint">Server\'ın mod klasöründen bir paket kodu üretilir. Oyuncular bu kodu <b>Server\'a Katıl</b> sayfasına yapıştırınca aynı modlar otomatik inip kurulur. Kodun çalışması için server\'ın dış adresi (Bağlantı sekmesi) bilinmeli.</p>' +
          '<div><button class="btn primary" id="hpc">' + kz.ic('link', 16) + 'Paket kodu üret</button> <button class="btn" data-o="mods">' + kz.ic('folder', 16) + 'Server klasörünü aç</button></div><div class="err" id="hpcErr"></div><div id="hpcOut"></div></div>');
      var run = function () {
        $('#hmErr').textContent = ''; $('#hmOut').innerHTML = '<span class="hint">Aranıyor...</span>';
        inv('host_search', { id: h.id, query: $('#hmQ').value.trim() }).then(function (l) {
          $('#hmOut').innerHTML = l.length ? l.slice(0, 12).map(function (x) {
            return '<div class="srow"><div><b>' + esc(x.title) + '</b><small>' + esc(x.description) + '</small><small class="mono">server: ' + esc(x.server_side) + ' · istemci: ' + esc(x.client_side) + '</small></div><button class="btn sm primary" data-hadd="' + esc(x.project_id) + '">Ekle</button></div>';
          }).join('') : '<span class="hint">Sonuç yok.</span>';
          $$('[data-hadd]', b).forEach(function (x) {
            x.addEventListener('click', function () {
              x.disabled = true; x.textContent = 'Ekleniyor...';
              inv('host_install', { id: h.id, project: x.dataset.hadd }).then(function (n) { x.textContent = 'Eklendi'; toast(n.length ? n.join(', ') + ' eklendi' : 'Zaten ekli'); }).catch(function (e) { x.disabled = false; x.textContent = 'Ekle'; $('#hmErr').textContent = errText(e); });
            });
          });
        }).catch(function (e) { $('#hmOut').innerHTML = ''; $('#hmErr').textContent = errText(e); });
      };
      $('#hmGo').addEventListener('click', run);
      $('#hmQ').addEventListener('keydown', function (e) { if (e.key === 'Enter') run(); });
      var pc = $('#hpc');
      if (pc) {
        $('[data-o]', b).addEventListener('click', function () { inv('host_open_folder', { id: h.id }).catch(actErr); });
        pc.addEventListener('click', function () {
          pc.disabled = true; $('#hpcErr').textContent = ''; $('#hpcOut').innerHTML = '';
          inv('host_pack_code', { id: h.id }).then(function (r) {
            pc.disabled = false;
            $('#hpcOut').innerHTML = '<div class="mono" style="word-break:break-all;padding:10px;border:1px solid var(--line);border-radius:var(--radius-sm)">' + esc(r.code) + '</div><p class="hint">' + r.included + ' mod pakete girdi' + (r.missing.length ? '; Modrinth\'te bulunamayan: ' + esc(r.missing.join(', ')) : '') + '.</p><button class="btn sm" id="hpcCopy">' + kz.ic('copy', 14) + 'Kopyala</button>';
            $('#hpcCopy').addEventListener('click', function () { (navigator.clipboard ? navigator.clipboard.writeText(r.code) : Promise.reject()).then(function () { toast('Kopyalandı'); }).catch(function () { toast('Kopyalanamadı, elle seç'); }); });
          }).catch(function (e) { pc.disabled = false; $('#hpcErr').textContent = errText(e); });
        });
      }
    }

    function tabNet(h, b) {
      var local = h.mode === 'local';
      b.innerHTML = '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Dış adres</h2>' +
        '<p class="hint">Arkadaşlarının bağlanacağı adres. Port otomatik açılınca ya da tünel adresini girince burada görünür; paket kodları bu adresi kullanır.</p>' +
        '<div class="joinbar"><input id="hAddr" class="in mono" style="flex:1" placeholder="örn. 85.100.20.4:25565 ya da ornek.joinmc.link" value="' + esc(h.public_address || '') + '" autocomplete="off"><button class="btn primary" id="hAddrSave">Kaydet</button></div><div class="err" id="hAddrErr"></div></div>' +
        (local ? '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Otomatik port yönlendirme (UPnP)</h2>' +
          '<label class="chk"><input type="checkbox" id="hUp"' + (h.upnp ? ' checked' : '') + '> Server başlayınca router\'da port ' + h.port + ' otomatik açılsın, kapanınca kapansın</label>' +
          '<p class="hint">Çoğu ev router\'ında çalışır. Router UPnP\'yi desteklemiyorsa ya da internet sağlayıcın ortak IP (CGNAT) veriyorsa port açılsa bile dışarıdan erişilemez; o zaman aşağıdaki tüneli kullan. Sonuç konsola yazılır.</p></div>' +
          '<div class="card" style="display:flex;flex-direction:column;gap:12px"><h2 style="margin:0">Tünel (playit.gg)</h2>' +
          '<div class="callout">' + kz.ic('shield', 20) + '<div>Router ayarı gerektirmez. Kazma, playit.gg\'nin resmî ajanını GitHub\'dan indirir ve dijital imzasını doğrulayıp öyle çalıştırır. İlk kullanımda playit.gg\'de ücretsiz bir hesap bağlaman ve bir <b>Minecraft Java</b> tüneli (127.0.0.1:' + h.port + ') oluşturman gerekir; tünel adresini yukarıdaki kutuya yapıştır. Ajanın çıktısı konsolda görünür.</div></div>' +
          '<div style="display:flex;gap:8px;flex-wrap:wrap"><button class="btn primary" id="tStart"' + (h.tunnel !== 'stopped' ? ' disabled' : '') + '>' + kz.ic('play', 16) + 'Tüneli başlat</button><button class="btn" id="tStop"' + (h.tunnel === 'stopped' ? ' disabled' : '') + '>' + kz.ic('stop', 16) + 'Tüneli durdur</button>' +
          '<button class="btn" id="tClaim"' + (claims[h.id] ? '' : ' hidden') + '>' + kz.ic('link', 16) + 'playit.gg hesabını bağla</button></div><div class="err" id="tErr"></div>' +
          '<p class="hint">Tünel durumu: ' + esc(h.tunnel === 'stopped' ? 'kapalı' : 'çalışıyor') + '. Hesap bağlama adresi konsolda belirir; bağlantı kurulunca düğme görünür.</p></div>'
          : '<div class="card hint">Ayrı makinede port yönlendirme o makinenin router\'ında yapılır. Paketin içindeki OKU-BENI.txt adımları anlatır.</div>');
      $('#hAddrSave').addEventListener('click', function () {
        inv('host_set_address', { id: h.id, address: $('#hAddr').value.trim() }).then(function (s) { h.public_address = s.public_address; toast('Adres kaydedildi'); }).catch(function (e) { $('#hAddrErr').textContent = errText(e); });
      });
      if (!local) return;
      $('#hUp').addEventListener('change', function () { inv('host_upnp', { id: h.id, on: this.checked }).then(function (s) { h.upnp = s.upnp; toast(s.upnp ? 'Otomatik port açık' : 'Otomatik port kapalı'); }).catch(actErr); });
      $('#tStart').addEventListener('click', function () { this.disabled = true; $('#tErr').textContent = ''; inv('tunnel_start', { id: h.id }).then(load).catch(function (e) { $('#tErr').textContent = errText(e); $('#tStart').disabled = false; }); });
      $('#tStop').addEventListener('click', function () { inv('tunnel_stop', { id: h.id }).catch(actErr); });
      $('#tClaim').addEventListener('click', function () { inv('tunnel_open_claim', { url: claims[h.id] }).catch(function (e) { $('#tErr').textContent = errText(e); }); });
    }

    /* ---------- olaylar ---------- */
    document.addEventListener('click', function (e) {
      var hm = e.target.closest('[data-hm]'); if (hm) { sel = hm.dataset.hm; tab = 'console'; draw(); return; }
      var t = e.target.closest('[data-ht]'); if (t && view.contains(t)) { tab = t.dataset.ht; var h = hostById(sel); if (h) { $$('[data-ht]').forEach(function (x) { x.setAttribute('aria-selected', x === t); }); drawTab(h); } return; }
      var a = e.target.closest('[data-ha]'); if (a && sel) {
        var act = { start: 'host_start', stop: 'host_stop', restart: 'host_restart' }[a.dataset.ha];
        a.disabled = true; inv(act, { id: sel }).catch(function (er) { actErr(er); load(); }); return;
      }
      if (e.target.closest('#hExport')) {
        var bt = e.target.closest('#hExport'); bt.disabled = true;
        inv('host_export', { id: sel }).then(function (p) { bt.disabled = false; toast('Paket hazır: ' + p); inv('host_open_exports'); }).catch(function (er) { bt.disabled = false; actErr(er); });
        return;
      }
      if (e.target.closest('#hExports')) { inv('host_open_exports').catch(actErr); }
    });
    listen('server-log', function (ev) {
      var p = ev.payload; var l = logs[p.id] = logs[p.id] || []; l.push(p.line); if (l.length > 2000) l.shift();
      if (sel === p.id && tab === 'console') { var c = $('#hcon'); if (c) addLine(c, p.line); }
    });
    listen('server-state', function (ev) {
      var p = ev.payload, h = hostById(p.id); if (!h) return; h.state = p.state;
      if (sel === p.id) { var st = STATE[p.state] || STATE.stopped; var s = $('#hState'); if (s) { s.textContent = st[1]; s.className = 'pill ' + (p.state === 'running' ? 'good' : 'mute'); } var bt = $('#hBtns'); if (bt) bt.innerHTML = stateBtns(h); }
      else if (!sel && location.hash === '#servers') draw();
    });
    listen('tunnel-state', function (ev) { var h = hostById(ev.payload.id); if (h) { h.tunnel = ev.payload.state; if (sel === h.id && tab === 'net') drawTab(h); } });
    listen('tunnel-claim', function (ev) { claims[ev.payload.id] = ev.payload.url; if (sel === ev.payload.id && tab === 'net') drawTab(hostById(sel)); toast('playit.gg hesap bağlama bağlantısı hazır (Bağlantı sekmesi)'); });
    listen('server-changed', function () { load(); });
    addEventListener('hashchange', function () {
      if (location.hash === '#servers') { load(); clearInterval(statusT); statusT = setInterval(function () { if (location.hash === '#servers' && !sel) refreshStatuses(); }, 30000); }
      else { clearInterval(statusT); }
    });
    if (location.hash === '#servers') load();

    /* ---------- Discord ayarı (hesap penceresine eklenir) ---------- */
    var sheet = $('#loginModal .sheet');
    if (sheet) {
      var foot = sheet.lastElementChild;
      foot.insertAdjacentHTML('beforebegin',
        '<div class="field" id="dcBox"><label for="dcId">Discord etkinliği (Kazma Launcher oynuyor)</label>' +
        '<label class="chk"><input type="checkbox" id="dcOn"> Oynarken Discord\'da göster</label>' +
        '<input id="dcId" class="in mono" placeholder="Discord uygulama kimliği (sayı)" autocomplete="off" inputmode="numeric">' +
        '<p class="hint">Discord Developer Portal\'da bir uygulama oluşturup Application ID\'sini yaz; "Rich Presence → Art Assets" bölümüne logoyu <b>kazma</b> adıyla yükle. Boş bırakırsan etkinlik gösterilmez.</p><div class="err" id="dcErr"></div></div>');
      var fill = function () { var c = L.cfg(); $('#dcOn').checked = c.discord !== false; $('#dcId').value = c.discord_app_id || ''; $('#dcErr').textContent = ''; };
      var save = function () { inv('set_discord', { enabled: $('#dcOn').checked, appId: $('#dcId').value.trim() }).then(function (c) { L.setCfg(c); $('#dcErr').textContent = ''; toast('Discord ayarı kaydedildi'); }).catch(function (e) { $('#dcErr').textContent = errText(e); }); };
      $('#dcOn').addEventListener('change', save); $('#dcId').addEventListener('change', save);
      $('#acctBtn').addEventListener('click', fill);
    }
  }
  if (window.__kz && window.__kz.live) boot(); else document.addEventListener('kazma-live', boot);
})();
