//! Uygulama düzeyi işlemler: ayarlar, instance kaydı, oyunu başlatma ve server'a katılma.
//! Arayüz yalnızca bu fonksiyonları çağırır.

use crate::install::{self, GameExit};
use crate::net::*;
use crate::report::Reporter;
use crate::{discord, gametheme, importer, modtheme, modrinth, serverkit, skins, sync};
use kazma_core::instance::{self, InstanceSpec, LoaderSpec};
use crate::ping;
use kazma_core::invite::{self, Invite};
use kazma_core::launch::{Account, LaunchOptions};
use kazma_core::manifest::{self, Loader, ModEntry, PackManifest, SyncPlan};
use kazma_core::pack::{self, PackCode};
use kazma_core::trust::{fingerprint, Pin};
use kazma_core::version::Env;
use kazma_core::{account, sign};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn default_root() -> PathBuf {
    if let Ok(a) = std::env::var("APPDATA") {
        return PathBuf::from(a).join(".kazma");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".kazma")
}

// ---------- ayarlar ----------

fn default_ram() -> u32 {
    4096
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub player: String,
    #[serde(default = "default_ram")]
    pub ram: u32,
    #[serde(default)]
    pub last_instance: Option<String>,
    /// Seçili skinin kimliği (skin kitaplığından).
    #[serde(default)]
    pub skin: Option<String>,
    /// Başka launcher'dan alınan kimlik (UUID). Boşsa çevrimdışı UUID kullanılır.
    #[serde(default)]
    pub uuid: Option<String>,
    /// Discord etkinliği (Kazma Launcher oynuyor) gösterilsin mi?
    #[serde(default = "default_true")]
    pub discord: bool,
    /// Discord geliştirici portalındaki uygulama kimliği (Rich Presence için).
    #[serde(default)]
    pub discord_app_id: Option<String>,
    /// Oyun menüsüne Kazma temasını (panorama) uygula.
    #[serde(default = "default_true")]
    pub game_theme: bool,
    /// "obsidian" ya da "terminal" (launcher teması ile aynı).
    #[serde(default = "default_variant")]
    pub theme_variant: String,
}

fn default_variant() -> String {
    "obsidian".into()
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config { player: String::new(), ram: default_ram(), last_instance: None, skin: None, uuid: None, discord: true, discord_app_id: None, game_theme: true, theme_variant: default_variant() }
    }
}

pub fn load_config(root: &Path) -> Config {
    fs::read_to_string(root.join("config.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_config(root: &Path, c: &Config) -> R<()> {
    write_atomic(&root.join("config.json"), serde_json::to_string_pretty(c).map_err(|e| e.to_string())?.as_bytes())
}

pub fn set_player(root: &Path, name: &str) -> R<Config> {
    account::validate_player_name(name).map_err(|e| e.to_string())?;
    let mut c = load_config(root);
    if c.player != name {
        c.uuid = None;
    }
    c.player = name.to_string();
    save_config(root, &c)?;
    Ok(c)
}

/// Oyuncu adı ile birlikte başka launcher'dan bulunan kimliği (UUID) kullan; böylece sunucudaki envanter/ilerleme aynı oyuncuya bağlı kalır.
pub fn set_identity(root: &Path, name: &str, uuid: &str) -> R<Config> {
    account::validate_player_name(name).map_err(|e| e.to_string())?;
    let u = importer::dash_uuid(uuid).ok_or("Geçersiz kimlik (UUID).")?;
    let mut c = load_config(root);
    c.player = name.to_string();
    c.uuid = Some(u);
    save_config(root, &c)?;
    Ok(c)
}

fn custom_sources(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("sources.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn scan_sources(root: &Path) -> Vec<importer::Source> {
    importer::scan_all(&custom_sources(root))
}

pub fn scan_accounts(root: &Path) -> Vec<importer::FoundAccount> {
    importer::scan_accounts(&custom_sources(root))
}

/// Kullanıcının seçtiği klasörü tarama listesine ekler (Minecraft oyun klasörü olmalı).
pub fn add_source(root: &Path, path: &str) -> R<Vec<importer::Source>> {
    let p = Path::new(path);
    let cand = importer::Candidate { launcher: "Özel klasör".into(), dir: p.to_path_buf(), meta_dir: None };
    if importer::scan(&cand).is_none() {
        return Err("Bu klasörde Minecraft verisi (saves, mods veya versions) bulunamadı.".into());
    }
    let mut v = custom_sources(root);
    if !v.iter().any(|x| x == path) {
        v.push(path.to_string());
    }
    write_atomic(&root.join("sources.json"), serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?.as_bytes())?;
    Ok(scan_sources(root))
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    pub source: String,
    /// Kaynak profilinin sürüm kimliği; boşsa yalnızca bu instance'a eklenir (`instance` verilmeli).
    pub profile: Option<usize>,
    /// Var olan bir instance'a aktar.
    pub instance: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub worlds: Vec<String>,
    #[serde(default)]
    pub extras: importer::Extras,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub instance: InstanceInfo,
    pub worlds: Vec<String>,
    pub notes: Vec<String>,
}

/// Başka launcher'ın klasöründen dünya/ayar/paketleri Kazma instance'ına KOPYALAR (kaynak değişmez).
pub fn import_run(root: &Path, req: &ImportRequest, rep: &dyn Reporter) -> R<ImportResult> {
    let sources = scan_sources(root);
    let src = sources.iter().find(|s| s.id == req.source).ok_or("Kaynak bulunamadı; listeyi yenile.")?;
    let src_dir = PathBuf::from(&src.path);
    let info = match (&req.instance, req.profile) {
        (Some(id), _) => read_info(root, id)?,
        (None, Some(i)) => {
            let p = src.profiles.get(i).ok_or("Profil bulunamadı.")?;
            let (game, loader) = (p.game.clone().ok_or("Bu profilin sürümü tanınmadı; var olan bir instance seç.")?, p.loader.clone().unwrap_or_else(|| "vanilla".into()));
            let lv = p.loader_version.clone().filter(|v| !v.is_empty());
            let name = req.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| p.name.clone());
            rep.stage("Instance oluşturuluyor");
            match create_instance(root, &name, &game, &loader, lv) {
                Ok(i) => i,
                Err(e) => return Err(e),
            }
        }
        _ => return Err("Hedef instance ya da kaynak profil seçilmeli.".into()),
    };
    let dst = instance_dir(root, &info.id)?;
    let worlds = importer::import_worlds(&src_dir, &dst, &req.worlds, rep)?;
    let notes = importer::import_extras(&src_dir, &dst, &req.extras, rep)?;
    Ok(ImportResult { instance: info, worlds, notes })
}

/// Skin kitaplığından bir skini seç (None: seçimi kaldır).
pub fn set_skin(root: &Path, id: Option<String>) -> R<Config> {
    if let Some(i) = &id {
        skins::skin_bytes(root, i)?;
    }
    let mut c = load_config(root);
    c.skin = id;
    save_config(root, &c)?;
    Ok(c)
}

/// Başlatmadan önce seçili skini instance'a yazar. Hata oyunu engellemez.
fn apply_active_skin(root: &Path, instance_id: &str, cfg: &Config) {
    if let Some(id) = &cfg.skin {
        let _ = skins::apply_to_instance(root, &root.join("instances").join(instance_id), &cfg.player, id);
    }
}

/// Güvenilir kaynak listesindeki bir adresi varsayılan tarayıcıda açar.
pub fn open_trusted_url(url: &str) -> R<()> {
    if !skins::browse_url_allowed(url) {
        return Err("Bu adres izinli listede değil".into());
    }
    open_in_file_manager(Path::new(url))
}

pub fn open_in_file_manager(path: &Path) -> R<()> {
    #[cfg(windows)]
    let mut c = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    c.arg(path).spawn().map(|_| ()).map_err(|e| format!("klasör açılamadı: {e}"))
}

/// Instance klasörünün yolu (yoksa oluşturulur). Dosya Gezgini'nde açmak için.
pub fn instance_dir(root: &Path, id: &str) -> R<PathBuf> {
    let info = read_info(root, id)?;
    let d = root.join("instances").join(&info.id);
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

pub fn open_instance_folder(root: &Path, id: &str) -> R<()> {
    open_in_file_manager(&instance_dir(root, id)?)
}

/// Mod, kaynak paketi ya da shader arama. Modlar yükleyiciye göre, paketler yalnızca sürüme göre aranır.
pub fn search_content(root: &Path, instance: &str, kind: &str, query: &str, category: Option<&str>) -> R<Vec<modrinth::Hit>> {
    let info = read_info(root, instance)?;
    let k = modrinth::Kind::parse(kind)?;
    if k == modrinth::Kind::Mod && info.loader == "vanilla" {
        return Err("Mod aramak için Fabric, Quilt, Forge ya da NeoForge'lu bir instance seç.".into());
    }
    modrinth::search_live(query, &info.game, Some(&info.loader), category, k)
}

pub fn install_content(root: &Path, instance: &str, kind: &str, project: &str, rep: &dyn Reporter) -> R<Vec<String>> {
    let info = read_info(root, instance)?;
    let k = modrinth::Kind::parse(kind)?;
    if k == modrinth::Kind::Mod {
        if info.loader == "vanilla" {
            return Err("Mod eklemek için Fabric, Quilt, Forge ya da NeoForge'lu bir instance seç.".into());
        }
        if info.managed {
            return Err("Bu instance'ın modlarını server'ın manifesti ya da paket kodu yönetir; elle mod eklenmez.".into());
        }
    }
    let dir = root.join("instances").join(&info.id).join(k.dir());
    modrinth::install_live(&dir, project, &info.game, Some(&info.loader), k, rep)
}

// ---------- instance kaydı ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub id: String,
    pub name: String,
    pub game: String,
    /// "vanilla", "fabric", "quilt", "forge" ya da "neoforge"
    pub loader: String,
    #[serde(default)]
    pub loader_version: String,
    /// "player" (oyuncunun kendi seçimi) ya da "server" (bir server'a ait)
    pub kind: String,
    #[serde(default)]
    pub server_host: Option<String>,
    #[serde(default)]
    pub server_port: Option<u16>,
    #[serde(default)]
    pub last_played: Option<u64>,
    /// Modları bir manifest ya da paket kodu yönetiyor (true) ya da oyuncu kendi ekliyor (false).
    #[serde(default)]
    pub managed: bool,
    /// Yönetilen instance'ı güncel tutan davet bağlantısı ya da paket kodu.
    #[serde(default)]
    pub source: Option<String>,
    /// Kayıtta tutulmaz, listelerken hesaplanır.
    #[serde(default, skip_deserializing)]
    pub mods: usize,
    /// Kazma Tema modu kurulu mu / bu instance için kurulabilir mi? (listelerken hesaplanır)
    #[serde(default, skip_deserializing)]
    pub theme_mod: bool,
    #[serde(default, skip_deserializing)]
    pub theme_eligible: bool,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn info_path(root: &Path, id: &str) -> PathBuf {
    root.join("instances").join(id).join("instance.json")
}

fn write_info(root: &Path, i: &InstanceInfo) -> R<()> {
    write_atomic(&info_path(root, &i.id), serde_json::to_string_pretty(i).map_err(|e| e.to_string())?.as_bytes())
}

pub fn read_info(root: &Path, id: &str) -> R<InstanceInfo> {
    if id.is_empty() || id.contains(['/', '\\', ':']) || id.contains("..") {
        return Err("instance kimliği geçersiz".into());
    }
    let t = fs::read_to_string(info_path(root, id)).map_err(|_| format!("instance bulunamadı: {id}"))?;
    let mut i: InstanceInfo = serde_json::from_str(&t).map_err(|e| format!("instance.json okunamadı: {e}"))?;
    i.id = id.to_string(); // klasör adı esastır
    i.mods = count_mods(&root.join("instances").join(id).join("mods"));
    i.theme_mod = modtheme::installed(&root.join("instances").join(id).join("mods"));
    i.theme_eligible = modtheme::eligible(&i.loader, &i.game);
    Ok(i)
}

fn count_mods(dir: &Path) -> usize {
    fs::read_dir(dir).map(|r| r.flatten().filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".jar")).count()).unwrap_or(0)
}

pub fn list_instances(root: &Path) -> Vec<InstanceInfo> {
    let mut v: Vec<InstanceInfo> = fs::read_dir(root.join("instances"))
        .map(|r| r.flatten().filter(|e| e.path().is_dir()).filter_map(|e| read_info(root, &e.file_name().to_string_lossy()).ok()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(a.name.cmp(&b.name)));
    v
}

/// Instance klasörünü (dünyalar, ayarlar ve modlar dahil) siler. Geri alınamaz.
pub fn delete_instance(root: &Path, id: &str) -> R<()> {
    let info = read_info(root, id)?; // kimliği de doğrular (yol karakteri yok, kayıt var)
    let dir = root.join("instances").join(&info.id);
    fs::remove_dir_all(&dir).map_err(|e| format!("Silinemedi ({e}). Oyun açıksa önce kapat."))?;
    let mut c = load_config(root);
    if c.last_instance.as_deref() == Some(id) {
        c.last_instance = None;
        save_config(root, &c)?;
    }
    Ok(())
}

pub fn spec_of(i: &InstanceInfo) -> R<InstanceSpec> {
    let loader = LoaderSpec::from_kind(&i.loader, &i.loader_version).map_err(|e| e.to_string())?;
    Ok(InstanceSpec { id: i.id.clone(), game: i.game.clone(), loader })
}

fn info_for(spec: &InstanceSpec, name: &str, kind: &str, host: Option<(&str, u16)>, managed: bool) -> InstanceInfo {
    InstanceInfo {
        id: spec.id.clone(),
        name: name.to_string(),
        game: spec.game.clone(),
        loader: spec.loader.kind().into(),
        loader_version: spec.loader.version().into(),
        kind: kind.into(),
        server_host: host.map(|h| h.0.to_string()),
        server_port: host.map(|h| h.1),
        last_played: None,
        managed,
        source: None,
        mods: 0,
        theme_mod: false,
        theme_eligible: false,
    }
}

fn clean_name(n: &str) -> R<String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 40 || n.chars().any(|c| c.is_control()) {
        return Err("instance adı 1-40 karakter olmalı".into());
    }
    Ok(n.to_string())
}

/// `loader_version`: boşsa seçilen yükleyicinin bu Minecraft sürümü için en uygun sürümü ağdan bulunur.
pub fn create_instance(root: &Path, name: &str, game: &str, loader: &str, loader_version: Option<String>) -> R<InstanceInfo> {
    let name = clean_name(name)?;
    let lv = match (loader, loader_version) {
        ("vanilla", _) => String::new(),
        (_, Some(v)) => v,
        (kind, None) => install::latest_loader(kind, game)?,
    };
    let spec = instance::from_selection(game, loader, &lv).map_err(|e| e.to_string())?;
    if info_path(root, &spec.id).is_file() {
        let ex = read_info(root, &spec.id)?;
        return Err(format!("Bu sürüm için zaten bir instance var: {}", ex.name));
    }
    write_info(root, &info_for(&spec, &name, "player", None, false))?;
    read_info(root, &spec.id)
}

fn mark_played(root: &Path, id: &str) {
    if let Ok(mut i) = read_info(root, id) {
        i.last_played = Some(now());
        let _ = write_info(root, &i);
    }
    let mut c = load_config(root);
    c.last_instance = Some(id.to_string());
    let _ = save_config(root, &c);
}

fn account_from(cfg: &Config) -> R<Account> {
    if cfg.player.is_empty() {
        return Err("Önce oyuncu adını belirle (sol alttaki hesap kutusu).".into());
    }
    let mut a = Account::offline(&cfg.player).map_err(|e| e.to_string())?;
    if let Some(u) = &cfg.uuid {
        a.uuid = u.clone();
    }
    Ok(a)
}

pub fn launch_instance(root: &Path, id: &str, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<GameExit> {
    let cfg = load_config(root);
    let acct = account_from(&cfg)?;
    let info = read_info(root, id)?;
    let spec = spec_of(&info)?;
    let env = Env::current();
    let inst = install::ensure_installed(root, &spec, &env, java, rep)?;
    let join = match (&info.kind[..], &info.server_host, info.server_port) {
        ("server", Some(h), Some(p)) => Some((h.clone(), p)),
        _ => None,
    };
    let opts = LaunchOptions { max_mb: cfg.ram, join_server: join, ..Default::default() };
    mark_played(root, id);
    apply_active_skin(root, id, &cfg);
    // Menü teması en iyi çabayla uygulanır; başarısız olursa oyun yine açılır.
    let jar = root.join(format!("versions/{0}/{0}.jar", inst.resolved.jar_id));
    if let Err(e) = gametheme::apply(&root.join("instances").join(&info.id), &info.game, &jar, &cfg.theme_variant, cfg.game_theme) {
        rep.stage(&format!("Menü teması uygulanamadı: {e}"));
    }
    let presence = presence_for(&cfg, &info);
    if let Some((app, act)) = &presence {
        let (app, act) = (app.clone(), act.clone());
        std::thread::spawn(move || {
            let _ = discord::update(&app, Some(&act));
        });
    }
    let exit = install::launch_game(root, &spec, &inst, &env, &acct, &opts, java, rep);
    if let Some((app, _)) = presence {
        std::thread::spawn(move || discord::clear(&app));
    }
    exit
}

/// Discord etkinliği: açıksa ve uygulama kimliği varsa.
pub fn presence_for(cfg: &Config, info: &InstanceInfo) -> Option<(String, discord::Activity)> {
    let app = cfg.discord_app_id.clone().filter(|a| cfg.discord && discord::valid_app_id(a))?;
    let loader = match info.loader.as_str() {
        "vanilla" => "Vanilla".to_string(),
        l => l[..1].to_uppercase() + &l[1..],
    };
    let state = match (&info.server_host, info.kind.as_str()) {
        (Some(h), "server") => format!("Sunucu: {h} · {} {loader}", info.game),
        _ => format!("Minecraft {} · {loader}", info.game),
    };
    Some((app, discord::Activity { details: "Kazma Launcher'da oynuyor".into(), state, start: now(), large_image: "kazma".into(), large_text: "Kazma Launcher".into() }))
}

/// Kazma Tema modunu (ve gerekli Fabric API'yi) instance'a kurar; launcher temasının rengini mod için yazar.
pub fn install_theme_mod(root: &Path, instance: &str, rep: &dyn Reporter) -> R<()> {
    let info = read_info(root, instance)?;
    let dir = root.join("instances").join(&info.id);
    let game = info.game.clone();
    modtheme::install_with(&dir, &info.loader, &info.game, &modtheme::fetch_live, &|m: &Path| {
        modrinth::install_live(m, "fabric-api", &game, Some("fabric"), modrinth::Kind::Mod, rep).map(|_| ())
    }, rep)?;
    let cfg = load_config(root);
    let _ = fs::create_dir_all(dir.join("config"));
    let _ = write_atomic(&dir.join("config").join("kazma-theme.txt"), cfg.theme_variant.as_bytes());
    Ok(())
}

pub fn remove_theme_mod(root: &Path, instance: &str) -> R<()> {
    let info = read_info(root, instance)?;
    modtheme::remove(&root.join("instances").join(&info.id))
}

pub fn set_game_theme(root: &Path, enabled: bool, variant: &str) -> R<Config> {
    if !["obsidian", "terminal"].contains(&variant) {
        return Err("Bilinmeyen tema.".into());
    }
    let mut c = load_config(root);
    c.game_theme = enabled;
    c.theme_variant = variant.to_string();
    save_config(root, &c)?;
    Ok(c)
}

pub fn set_discord(root: &Path, enabled: bool, app_id: &str) -> R<Config> {
    let id = app_id.trim();
    if !id.is_empty() && !discord::valid_app_id(id) {
        return Err("Discord uygulama kimliği 15-22 haneli bir sayı olmalı.".into());
    }
    let mut c = load_config(root);
    c.discord = enabled;
    c.discord_app_id = if id.is_empty() { None } else { Some(id.to_string()) };
    save_config(root, &c)?;
    Ok(c)
}

// ---------- adresle bağlanma (Aternos vb.) ----------

#[derive(Debug, Serialize)]
pub struct ProbeView {
    pub host: String,
    pub port: u16,
    pub online: bool,
    pub version_name: String,
    pub players: String,
    pub motd: String,
    /// Server'ın söylediği sürümden çıkarılan Minecraft sürümü.
    pub guessed_game: Option<String>,
    /// "forge" ise server Forge/NeoForge modları kullanıyor olabilir.
    pub loader_hint: Option<String>,
    pub note: Option<String>,
    /// Bu adres için daha önce oluşturulmuş instance.
    pub existing: Option<String>,
}

/// Ağ gerektirmeyen kısım: sorgu sonucunu arayüz için yorumlar.
pub fn probe_with(root: &Path, host: &str, port: u16, available: &[String]) -> ProbeView {
    let existing = instance::for_server(host, port, "x", "vanilla", "").ok().map(|s| s.id).filter(|id| info_path(root, id).is_file());
    let mut v = ProbeView { host: host.into(), port, online: false, version_name: String::new(), players: String::new(), motd: String::new(), guessed_game: None, loader_hint: None, note: None, existing };
    match ping::query(host, port, std::time::Duration::from_secs(5)) {
        Ok(st) => {
            v.online = true;
            v.guessed_game = ping::guess_game_version(&st.version_name, available);
            let sleeping = st.protocol <= 0 || st.version_name.to_lowercase().contains("offline") || st.motd.to_lowercase().contains("offline");
            if sleeping || v.guessed_game.is_none() {
                v.note = Some("Server şu an kapalı, uyuyor ya da sürümünü söylemiyor. Aternos'ta server'ı sahibinin başlatması gerekir. Sürümü elle seçebilirsin.".into());
            }
            v.players = format!("{}/{}", st.players_online, st.players_max);
            v.version_name = st.version_name;
            v.motd = st.motd;
            v.loader_hint = st.loader_hint;
        }
        Err(e) => v.note = Some(format!("Server'a ulaşılamadı ({e}). Adresi ve portu kontrol et; Aternos'ta server'ın açık olması gerekir. Yine de sürümü elle seçip devam edebilirsin.")),
    }
    v
}

pub fn probe_server(root: &Path, address: &str, rep: &dyn Reporter) -> R<ProbeView> {
    let inv = invite::parse(address).map_err(|e| e.to_string())?;
    rep.stage("Server sorgulanıyor");
    let available = install::list_versions(false).unwrap_or_default();
    Ok(probe_with(root, &inv.host, inv.port, &available))
}

/// Davet bağlantısı olmayan bir server için instance oluşturur ya da günceller (modları oyuncu kendisi ekler).
/// `launch` doğruysa hemen oyunu açıp bağlanır.
pub fn connect_server(root: &Path, address: &str, game: &str, loader: &str, loader_version: Option<String>, launch: bool, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<(InstanceInfo, Option<GameExit>)> {
    let inv = invite::parse(address).map_err(|e| e.to_string())?;
    let lv = match (loader, loader_version) {
        ("vanilla", _) => String::new(),
        (_, Some(v)) => v,
        (kind, None) => {
            rep.stage("Yükleyici sürümü aranıyor");
            install::latest_loader(kind, game)?
        }
    };
    let spec = instance::for_server(&inv.host, inv.port, game, loader, &lv).map_err(|e| e.to_string())?;
    let mut info = info_for(&spec, &inv.address(), "server", Some((&inv.host, inv.port)), false);
    if let Ok(old) = read_info(root, &spec.id) {
        info.last_played = old.last_played;
        info.managed = old.managed && old.game == info.game && old.loader == info.loader;
        info.name = old.name;
    }
    write_info(root, &info)?;
    let info = read_info(root, &spec.id)?;
    let exit = if launch { Some(launch_instance(root, &spec.id, java, rep)?) } else { None };
    Ok((info, exit))
}

// ---------- katılınan server'lar ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServerEntry {
    pub instance: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub game: String,
    pub loader: String,
    pub last_played: Option<u64>,
    pub managed: bool,
}

/// Katıldığımız (instance'ı olan) server'lar, en son oynananlar önce.
pub fn list_servers(root: &Path) -> Vec<ServerEntry> {
    let mut v: Vec<ServerEntry> = list_instances(root)
        .into_iter()
        .filter(|i| i.kind == "server")
        .filter_map(|i| Some(ServerEntry { host: i.server_host.clone()?, port: i.server_port?, instance: i.id, name: i.name, game: i.game, loader: i.loader, last_played: i.last_played, managed: i.managed }))
        .collect();
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(a.name.cmp(&b.name)));
    v
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServerStatus {
    pub online: bool,
    pub players: String,
    pub motd: String,
    pub version: String,
}

/// Hızlı durum sorgusu (kısa zaman aşımı). Uyuyan/kapalı Aternos server'ı "çevrimdışı" sayılır.
pub fn server_status(host: &str, port: u16) -> ServerStatus {
    match ping::query(host, port, std::time::Duration::from_secs(3)) {
        Ok(st) => {
            let sleeping = st.protocol <= 0 || st.version_name.to_lowercase().contains("offline") || st.motd.to_lowercase().contains("offline");
            ServerStatus { online: !sleeping, players: format!("{}/{}", st.players_online, st.players_max), motd: st.motd, version: st.version_name }
        }
        Err(_) => ServerStatus { online: false, players: String::new(), motd: String::new(), version: String::new() },
    }
}

// ---------- server'a katılma ----------

#[derive(Debug, Clone)]
pub struct Prepared {
    pub invite: Invite,
    pub manifest: PackManifest,
    pub key: String,
    pub fingerprint: String,
    pub is_new: bool,
    /// Manifest imzalı mı (davet bağlantısı) yoksa imzasız paket kodu mu.
    pub signed: bool,
    /// Instance'ı güncel tutmak için saklanan davet bağlantısı ya da paket kodu.
    pub source: String,
    pub spec: InstanceSpec,
    pub plan: SyncPlan,
}

/// Ağ gerektirmeyen kısım: doğrulama, imza, instance ve plan.
pub fn prepare_from_manifest(root: &Path, inv: Invite, m: PackManifest, manifest_host: &str) -> R<Prepared> {
    m.validate_with(&[inv.host.as_str(), manifest_host]).map_err(|e| e.to_string())?;
    let server_id = format!("{}:{}", inv.host, inv.port);
    let trust = sync::load_trust(root);
    let pinned = trust.servers.get(&server_id).cloned();
    let key = sign::verify(&m, pinned.as_deref()).map_err(|_| {
        if pinned.is_some() {
            format!("{server_id} için daha önce kaydedilen imza anahtarı değişmiş. Server ele geçirilmiş ya da araya biri girmiş olabilir. Katılım iptal edildi.")
        } else {
            "Manifestin imzası geçersiz ya da eksik. Katılım iptal edildi.".to_string()
        }
    })?;
    let is_new = trust.check(&server_id, &key).map_err(|e| e.to_string())? == Pin::New;
    let spec = instance::from_manifest(&m, &inv.host, inv.port).map_err(|e| e.to_string())?;
    let local = sync::scan_mods(&root.join(spec.mods_rel()))?;
    let plan = manifest::plan(&m, &local);
    Ok(Prepared { fingerprint: fingerprint(&key), invite: inv, manifest: m, key, is_new, signed: true, source: String::new(), spec, plan })
}

/// Ağ gerektirmeyen kısım: paket kodundaki sürümler çözüldükten sonra plan.
pub fn prepare_from_pack(root: &Path, pc: &PackCode, entries: Vec<ModEntry>) -> R<Prepared> {
    if pc.a.is_empty() {
        return Err("Paket kodunda server adresi yok. Server sahibinden adresli bir kod iste.".into());
    }
    let inv = invite::parse(&pc.a).map_err(|e| e.to_string())?;
    let m = PackManifest { name: pc.n.clone(), game: pc.g.clone(), loader: Loader { kind: pc.l.clone(), version: pc.lv.clone() }, mods: entries, signer: None, signature: None };
    m.validate_with(&[inv.host.as_str()]).map_err(|e| e.to_string())?;
    let spec = instance::from_manifest(&m, &inv.host, inv.port).map_err(|e| e.to_string())?;
    let local = sync::scan_mods(&root.join(spec.mods_rel()))?;
    let plan = pack::plan_by_hash(&m.mods, &local);
    Ok(Prepared { invite: inv, manifest: m, key: String::new(), fingerprint: String::new(), is_new: false, signed: false, source: String::new(), spec, plan })
}

/// Davet bağlantısı (`kazma://join/...?m=`, imzalı manifest) ya da paket kodu (`kazma://pack/...`).
pub fn join_prepare(root: &Path, text: &str, rep: &dyn Reporter) -> R<Prepared> {
    let text = text.trim();
    let mut p = if pack::is_pack_code(text) {
        let pc = pack::decode(text).map_err(|e| e.to_string())?;
        rep.stage("Mod listesi Modrinth'ten alınıyor");
        let entries = modrinth::resolve_pack_live(&pc.m)?;
        prepare_from_pack(root, &pc, entries)?
    } else {
        let inv = invite::parse(text).map_err(|e| e.to_string())?;
        let murl = inv.manifest_url.clone().ok_or("Bu bağlantıda mod listesi yok. Server sahibinden davet bağlantısı (?m=...) ya da paket kodu (kazma://pack/...) iste. Düz adresle bağlanmak için \"Sunucu adresi\" sekmesini kullan.")?;
        rep.stage("Server manifesti alınıyor");
        let m = sync::fetch_manifest(&murl)?;
        let host = url::Url::parse(&murl).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
        prepare_from_manifest(root, inv, m, &host)?
    };
    p.source = text.to_string();
    Ok(p)
}

/// Server sahibi: mod klasöründen paket kodu üretir (Modrinth'te bulunan modlarla).
pub fn make_pack(dir: &Path, name: &str, address: &str, game: &str, loader: &str, loader_version: Option<String>, rep: &dyn Reporter) -> R<serverkit::PackBuild> {
    if loader == "vanilla" {
        return Err("Paket kodu mod yükleyicili server'lar için; bir yükleyici seç.".into());
    }
    invite::parse(address).map_err(|e| e.to_string())?;
    let lv = match loader_version {
        Some(v) => v,
        None => {
            rep.stage("Yükleyici sürümü aranıyor");
            install::latest_loader(loader, game)?
        }
    };
    rep.stage("Mod dosyaları taranıyor");
    let (mods, unknown) = serverkit::scan_lenient(dir)?;
    if mods.is_empty() {
        return Err("Bu klasörde tanınan mod yok. Server'ın mods klasörünü seç.".into());
    }
    rep.stage("Modrinth'te aranıyor");
    serverkit::build_pack_code(name, address, game, (loader, &lv), &mods, unknown, &modrinth::lookup_hashes_live)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct JoinRow {
    pub id: String,
    pub version: String,
    /// ready | download | update | disable | keep
    pub status: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct JoinView {
    pub server_name: String,
    pub address: String,
    pub game: String,
    pub loader: String,
    pub loader_version: String,
    pub instance_id: String,
    pub fingerprint: String,
    pub is_new: bool,
    pub signed: bool,
    pub rows: Vec<JoinRow>,
    pub download_bytes: u64,
    pub noop: bool,
}

pub fn join_view(p: &Prepared) -> JoinView {
    let ver_of = |id: &str| p.manifest.mods.iter().find(|m| m.id == id).map(|m| m.version.clone()).unwrap_or_default();
    let mut rows = Vec::new();
    for e in &p.plan.download {
        rows.push(JoinRow { id: e.id.clone(), version: e.version.clone(), status: "download".into(), size: e.size });
    }
    for (l, e) in &p.plan.update {
        rows.push(JoinRow { id: e.id.clone(), version: format!("{} → {}", l.file, e.version), status: "update".into(), size: e.size });
    }
    for l in &p.plan.disable {
        rows.push(JoinRow { id: l.id.clone(), version: l.file.clone(), status: "disable".into(), size: 0 });
    }
    for l in &p.plan.keep {
        rows.push(JoinRow { id: l.id.clone(), version: l.file.clone(), status: "keep".into(), size: 0 });
    }
    for id in &p.plan.ready {
        rows.push(JoinRow { id: id.clone(), version: ver_of(id), status: "ready".into(), size: 0 });
    }
    JoinView {
        server_name: p.manifest.name.clone(),
        address: p.invite.address(),
        game: p.manifest.game.clone(),
        loader: p.manifest.loader.kind.clone(),
        loader_version: p.manifest.loader.version.clone(),
        instance_id: p.spec.id.clone(),
        fingerprint: p.fingerprint.clone(),
        is_new: p.is_new,
        signed: p.signed,
        rows,
        download_bytes: p.plan.download_bytes(),
        noop: p.plan.is_noop(),
    }
}

/// Server'ın sürümünü kurar, modları eşitler ve doğrudan server'a bağlanarak oyunu açar.
pub fn join_run(root: &Path, p: &Prepared, trust_new: bool, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<GameExit> {
    let cfg = load_config(root);
    let acct = account_from(&cfg)?;
    let server_id = format!("{}:{}", p.invite.host, p.invite.port);
    if !p.signed && !trust_new {
        return Err("Bu paket imzasız. Kodu kimden aldığına güvendiğini onaylamadan katılamazsın.".into());
    }
    if p.signed && p.is_new {
        if !trust_new {
            return Err("Bu server'a güvendiğini onaylamadan katılamazsın.".into());
        }
        let mut t = sync::load_trust(root);
        t.pin(&server_id, &p.key);
        sync::save_trust(root, &t)?;
    }
    write_info(root, &{
        let mut i = info_for(&p.spec, &p.manifest.name, "server", Some((&p.invite.host, p.invite.port)), true);
        i.last_played = read_info(root, &p.spec.id).ok().and_then(|x| x.last_played);
        i.source = Some(p.source.clone()).filter(|s| !s.is_empty());
        i
    })?;
    let env = Env::current();
    let inst = install::ensure_installed(root, &p.spec, &env, java, rep)?;

    let mods_dir = root.join(p.spec.mods_rel());
    if !p.plan.is_noop() {
        let total = p.plan.download.len() + p.plan.update.len();
        let n = AtomicUsize::new(0);
        rep.stage(&format!("{total} mod indiriliyor"));
        let fetch = |u: &str| {
            let b = get_bytes(u, 300 << 20);
            rep.progress("modlar", n.fetch_add(1, Ordering::Relaxed) + 1, total.max(1));
            b
        };
        sync::apply_plan(&mods_dir, &p.plan, &fetch)?;
    }
    let opts = LaunchOptions { max_mb: cfg.ram, join_server: Some((p.invite.host.clone(), p.invite.port)), ..Default::default() };
    mark_played(root, &p.spec.id);
    apply_active_skin(root, &p.spec.id, &cfg);
    install::launch_game(root, &p.spec, &inst, &env, &acct, &opts, java, rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;
    use kazma_core::manifest::{Loader, ModEntry, Side};
    use kazma_core::verify::sha512_hex;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-app-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lists_only_server_instances_newest_first() {
        let r = tmp("srvlist");
        connect_server(&r, "a.example.com", "1.21.1", "vanilla", None, false, &None, &NullReporter).unwrap();
        connect_server(&r, "b.example.com:25570", "1.20.1", "vanilla", None, false, &None, &NullReporter).unwrap();
        create_instance(&r, "Kendi", "1.21.1", "vanilla", None).unwrap();
        let id = list_servers(&r).iter().find(|s| s.host == "b.example.com").unwrap().instance.clone();
        mark_played(&r, &id);
        let l = list_servers(&r);
        assert_eq!(l.len(), 2, "player instance listelenmez");
        assert_eq!((l[0].host.as_str(), l[0].port), ("b.example.com", 25570));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn discord_presence_needs_id_and_switch() {
        let r = tmp("disc");
        let inst = create_instance(&r, "Kule", "1.21.1", "vanilla", None).unwrap();
        assert!(presence_for(&load_config(&r), &inst).is_none(), "kimlik yoksa etkinlik yok");
        assert!(set_discord(&r, true, "abc").is_err());
        let c = set_discord(&r, true, " 1234567890123456789 ").unwrap();
        let (app, act) = presence_for(&c, &inst).unwrap();
        assert_eq!((app.as_str(), act.state.as_str()), ("1234567890123456789", "Minecraft 1.21.1 · Vanilla"));
        let c = set_discord(&r, false, "1234567890123456789").unwrap();
        assert!(presence_for(&c, &inst).is_none());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn dead_server_reports_offline() {
        assert!(!server_status("127.0.0.1", 1).online);
    }

    #[test]
    fn identity_overrides_uuid_and_resets_on_rename() {
        let r = tmp("ident");
        let c = set_identity(&r, "Notch", "069A79F444E94726A5BEFCA90E38AAF5").unwrap();
        assert_eq!(c.uuid.as_deref(), Some("069a79f4-44e9-4726-a5be-fca90e38aaf5"));
        assert_eq!(account_from(&load_config(&r)).unwrap().uuid, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        assert!(set_identity(&r, "Notch", "xyz").is_err());
        let c = set_player(&r, "Baska").unwrap();
        assert_eq!(c.uuid, None, "ad değişince eski kimlik düşer");
        assert_eq!(account_from(&c).unwrap().uuid, kazma_core::account::offline_uuid("Baska").unwrap());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn import_into_existing_instance_copies_worlds() {
        let (r, src) = (tmp("imp-root"), tmp("imp-src"));
        let w = src.join("saves/Koy/region");
        fs::create_dir_all(&w).unwrap();
        fs::write(src.join("saves/Koy/level.dat"), b"x").unwrap();
        fs::write(w.join("r.mca"), b"y").unwrap();
        add_source(&r, src.to_str().unwrap()).unwrap();
        let id = scan_sources(&r).iter().find(|s| s.path == src.to_string_lossy()).unwrap().id.clone();
        let inst = create_instance(&r, "Hedef", "1.21.1", "vanilla", None).unwrap();
        let req = ImportRequest { source: id, profile: None, instance: Some(inst.id.clone()), name: None, worlds: vec!["Koy".into()], extras: Default::default() };
        let out = import_run(&r, &req, &NullReporter).unwrap();
        assert_eq!(out.worlds, ["Koy"]);
        assert!(instance_dir(&r, &inst.id).unwrap().join("saves/Koy/region/r.mca").is_file());
        let bad = ImportRequest { source: "yok".into(), profile: None, instance: None, name: None, worlds: vec![], extras: Default::default() };
        assert!(import_run(&r, &bad, &NullReporter).is_err());
        assert!(add_source(&r, r.join("bos").to_str().unwrap()).is_err());
        let _ = fs::remove_dir_all(&r);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn config_roundtrip_and_validation() {
        let r = tmp("cfg");
        assert_eq!(load_config(&r), Config::default());
        let c = set_player(&r, "Mustafa").unwrap();
        assert_eq!((c.player.as_str(), c.ram), ("Mustafa", 4096));
        assert_eq!(load_config(&r).player, "Mustafa");
        assert!(set_player(&r, "a b").is_err());
        assert_eq!(load_config(&r).player, "Mustafa", "geçersiz ad kaydedilmez");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn instances_create_list_and_dedupe() {
        let r = tmp("inst");
        assert!(list_instances(&r).is_empty());
        let a = create_instance(&r, "Vanilla 1.20.1", "1.20.1", "vanilla", None).unwrap();
        assert_eq!((a.id.as_str(), a.loader.as_str(), a.kind.as_str()), ("1.20.1-vanilla", "vanilla", "player"));
        let b = create_instance(&r, "Modlu", "1.21.1", "fabric", Some("0.16.9".into())).unwrap();
        assert_eq!(b.id, "1.21.1-fabric-0.16.9");
        assert!(create_instance(&r, "Kopya", "1.20.1", "vanilla", None).unwrap_err().contains("zaten"), "aynı sürüm tekrarlanmaz");
        assert!(create_instance(&r, "  ", "1.20.1", "vanilla", None).is_err());
        assert!(create_instance(&r, "x", "../kotu", "vanilla", None).is_err());
        fs::create_dir_all(r.join("instances/1.21.1-fabric-0.16.9/mods")).unwrap();
        fs::write(r.join("instances/1.21.1-fabric-0.16.9/mods/a.jar"), b"x").unwrap();
        fs::write(r.join("instances/1.21.1-fabric-0.16.9/mods/a.jar.disabled"), b"x").unwrap();
        let l = list_instances(&r);
        assert_eq!(l.len(), 2);
        assert_eq!(l.iter().find(|i| i.id == b.id).unwrap().mods, 1, ".disabled sayılmaz");
        let s = spec_of(&b).unwrap();
        assert_eq!(s.version_id(), "fabric-loader-0.16.9-1.21.1");
        assert!(read_info(&r, "../x").is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn all_loaders_can_be_created_with_explicit_versions() {
        let r = tmp("loaders");
        for (k, g, v) in [("quilt", "1.21.1", "0.26.1"), ("forge", "1.20.1", "47.3.0"), ("neoforge", "1.21.1", "21.1.172")] {
            let i = create_instance(&r, k, g, k, Some(v.into())).unwrap();
            assert_eq!((i.loader.as_str(), i.loader_version.as_str()), (k, v));
            assert_eq!(spec_of(&i).unwrap().loader.kind(), k);
        }
        assert_eq!(list_instances(&r).len(), 3);
        assert!(create_instance(&r, "x", "1.21.1", "magic", Some("1".into())).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn direct_connect_creates_unmanaged_server_instance() {
        let r = tmp("direct");
        let (i, exit) = connect_server(&r, "my_world.aternos.me:12345", "1.20.1", "forge", Some("47.3.0".into()), false, &None, &NullReporter).unwrap();
        assert!(exit.is_none());
        assert_eq!((i.id.as_str(), i.kind.as_str(), i.managed), ("server-my_world.aternos.me-12345", "server", false));
        assert_eq!((i.server_host.as_deref(), i.server_port), (Some("my_world.aternos.me"), Some(12345)));
        // aynı server için sürüm değiştirilince kayıt güncellenir, tek instance kalır
        let (j, _) = connect_server(&r, "my_world.aternos.me:12345", "1.21.1", "vanilla", None, false, &None, &NullReporter).unwrap();
        assert_eq!((j.game.as_str(), j.loader.as_str()), ("1.21.1", "vanilla"));
        assert_eq!(list_instances(&r).len(), 1);
        assert!(connect_server(&r, "bad host", "1.21.1", "vanilla", None, false, &None, &NullReporter).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn probe_reports_unreachable_and_online_servers() {
        use std::io::{Read, Write};
        let r = tmp("probe");
        let av: Vec<String> = vec!["1.20.1".into(), "1.21.1".into()];
        let dead = probe_with(&r, "127.0.0.1", 1, &av);
        assert!(!dead.online);
        assert!(dead.note.unwrap().contains("ulaşılamadı"));

        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let n = crate::ping::read_varint(&mut c).unwrap() as usize;
            let mut b = vec![0u8; n + 2];
            c.read_exact(&mut b).unwrap();
            let json = br#"{"version":{"name":"Forge 1.20.1","protocol":763},"players":{"max":8,"online":2},"description":{"text":"Dostlar"},"forgeData":{}}"#;
            let mut body = Vec::new();
            crate::ping::write_varint(&mut body, 0);
            crate::ping::write_varint(&mut body, json.len() as u32);
            body.extend(json);
            let mut f = Vec::new();
            crate::ping::write_varint(&mut f, body.len() as u32);
            f.extend(body);
            c.write_all(&f).unwrap();
        });
        let v = probe_with(&r, "127.0.0.1", port, &av);
        t.join().unwrap();
        assert!(v.online);
        assert_eq!((v.guessed_game.as_deref(), v.loader_hint.as_deref(), v.players.as_str(), v.motd.as_str()), (Some("1.20.1"), Some("forge"), "2/8", "Dostlar"));
        assert!(v.note.is_none());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn delete_removes_everything_for_that_instance_only() {
        let r = tmp("del");
        let a = create_instance(&r, "A", "1.20.1", "vanilla", None).unwrap();
        let b = create_instance(&r, "B", "1.21.1", "fabric", Some("0.16.9".into())).unwrap();
        fs::create_dir_all(r.join("instances").join(&a.id).join("saves/Dunya")).unwrap();
        fs::write(r.join("instances").join(&a.id).join("saves/Dunya/level.dat"), b"x").unwrap();
        let mut c = load_config(&r);
        c.last_instance = Some(a.id.clone());
        save_config(&r, &c).unwrap();
        delete_instance(&r, &a.id).unwrap();
        assert!(!r.join("instances").join(&a.id).exists());
        assert!(r.join("instances").join(&b.id).join("instance.json").is_file(), "diğer instance'a dokunulmaz");
        assert_eq!(load_config(&r).last_instance, None);
        assert!(delete_instance(&r, &a.id).is_err(), "ikinci silme hata verir");
        assert!(delete_instance(&r, "../instances").is_err());
        assert!(delete_instance(&r, "..\\x").is_err());
        assert!(r.join("instances").is_dir());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn pack_code_flow_is_unsigned_and_needs_explicit_trust() {
        let r = tmp("pack");
        let data = crate::sync::tests_jar("create", "*", "x");
        let pc = PackCode { v: 1, n: "Dostlar".into(), a: "my_world.aternos.me:12345".into(), g: "1.20.1".into(), l: "forge".into(), lv: "47.3.0".into(), m: vec!["VerA".into()] };
        let e = entry("create", &data);
        let mut p = prepare_from_pack(&r, &pc, vec![e.clone()]).unwrap();
        p.source = "kazma://pack/xyz".into();
        assert!(!p.signed && !p.is_new);
        assert_eq!(p.spec.id, "server-my_world.aternos.me-12345");
        assert_eq!(p.plan.download.len(), 1);
        let v = join_view(&p);
        assert!(!v.signed);
        assert_eq!(v.loader, "forge");
        set_player(&r, "Mustafa").unwrap();
        assert!(join_run(&r, &p, false, &None, &NullReporter).err().unwrap().contains("imzasız"));
        // yerel dosya paketle aynıysa hazır sayılır
        let dir = r.join(p.spec.mods_rel());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("create.jar"), &data).unwrap();
        let p2 = prepare_from_pack(&r, &pc, vec![e]).unwrap();
        assert!(p2.plan.is_noop());
        // adressiz kod reddedilir, güvenilmeyen adres reddedilir
        let mut no_addr = pc.clone();
        no_addr.a = String::new();
        assert!(prepare_from_pack(&r, &no_addr, vec![]).err().unwrap().contains("adres"));
        let mut evil = entry("evil", b"e");
        evil.url = "https://evil.example/e.jar".into();
        assert!(prepare_from_pack(&r, &pc, vec![evil]).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn join_prepare_gives_clear_messages_without_network() {
        let r = tmp("msg");
        let e = join_prepare(&r, "my_world.aternos.me:12345", &NullReporter).err().unwrap();
        assert!(e.contains("paket kodu") && e.contains("Sunucu adresi"));
        assert!(join_prepare(&r, "kazma://pack/!!!", &NullReporter).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn active_skin_is_selected_and_written_per_instance() {
        let r = tmp("skinsel");
        assert!(set_skin(&r, Some("abcdef123456".into())).is_err(), "kitaplıkta olmayan skin seçilemez");
        let png = {
            let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
            b.extend(b"IHDR");
            b.extend(64u32.to_be_bytes());
            b.extend(64u32.to_be_bytes());
            b.extend([8, 6, 0, 0, 0, 9, 9, 9]);
            b
        };
        let s = skins::add_skin(&r, "A", "dosya", &png).unwrap();
        let c = set_skin(&r, Some(s.id.clone())).unwrap();
        assert_eq!(c.skin.as_deref(), Some(s.id.as_str()));
        let mut cfg = load_config(&r);
        cfg.player = "Mustafa".into();
        apply_active_skin(&r, "1.20.1-vanilla", &cfg);
        assert!(r.join("instances/1.20.1-vanilla/CustomSkinLoader/LocalSkin/skins/Mustafa.png").is_file());
        assert_eq!(set_skin(&r, None).unwrap().skin, None);
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn content_rules_by_kind() {
        let r = tmp("content");
        create_instance(&r, "Vanilla", "1.20.1", "vanilla", None).unwrap();
        let (_, _) = connect_server(&r, "x.aternos.me:1", "1.20.1", "forge", Some("47.3.0".into()), false, &None, &NullReporter).unwrap();
        // vanilla instance'a mod eklenmez
        assert!(install_content(&r, "1.20.1-vanilla", "mod", "p", &NullReporter).unwrap_err().contains("Mod eklemek"));
        assert!(search_content(&r, "1.20.1-vanilla", "mod", "q", None).unwrap_err().contains("Mod aramak"));
        assert!(search_content(&r, "1.20.1-vanilla", "bilinmeyen", "q", None).is_err());
        // yönetilen (manifestli) instance'a elle mod eklenmez, ama kaynak paketi eklenebilir kuralı bozulmaz
        let mut i = read_info(&r, "server-x.aternos.me-1").unwrap();
        i.managed = true;
        write_info(&r, &i).unwrap();
        assert!(install_content(&r, "server-x.aternos.me-1", "mod", "p", &NullReporter).unwrap_err().contains("yönetir"));
        assert!(instance_dir(&r, "../x").is_err());
        assert!(instance_dir(&r, "1.20.1-vanilla").unwrap().is_dir());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn launch_needs_player_name() {
        let r = tmp("noplayer");
        create_instance(&r, "A", "1.20.1", "vanilla", None).unwrap();
        let e = launch_instance(&r, "1.20.1-vanilla", &None, &NullReporter).err().unwrap();
        assert!(e.contains("oyuncu adını"));
        let _ = fs::remove_dir_all(&r);
    }

    fn signed(seed: u8, mods: Vec<ModEntry>) -> PackManifest {
        let mut m = PackManifest { name: "Dostlar SMP".into(), game: "1.21.1".into(), loader: Loader { kind: "fabric".into(), version: "0.16.9".into() }, mods, signer: None, signature: None };
        sign::sign(&mut m, &[seed; 32]);
        m
    }
    fn entry(id: &str, data: &[u8]) -> ModEntry {
        ModEntry { id: id.into(), version: "1.0".into(), file: format!("{id}.jar"), sha512: sha512_hex(data), url: format!("https://cdn.modrinth.com/data/{id}.jar"), side: Side::Both, size: 10 }
    }
    fn inv() -> Invite {
        invite::parse("kazma://join/play.example.com:25570").unwrap()
    }

    #[test]
    fn prepare_pins_and_detects_key_change() {
        let r = tmp("prep");
        let p = prepare_from_manifest(&r, inv(), signed(1, vec![entry("create", b"c")]), "files.example.com").unwrap();
        assert!(p.is_new);
        assert_eq!(p.spec.id, "server-play.example.com-25570");
        assert_eq!(p.plan.download.len(), 1);
        let v = join_view(&p);
        assert_eq!((v.game.as_str(), v.loader_version.as_str(), v.noop), ("1.21.1", "0.16.9", false));
        assert_eq!(v.rows[0], JoinRow { id: "create".into(), version: "1.0".into(), status: "download".into(), size: 10 });
        assert_eq!(v.fingerprint.len(), 19);

        // oyuncu adı yokken katılım istenir
        assert!(join_run(&r, &p, true, &None, &NullReporter).err().unwrap().contains("oyuncu adını"));
        set_player(&r, "Mustafa").unwrap();
        // güven onayı olmadan katılım reddedilir (oyun kurulmaya çalışılmadan önce)
        assert!(join_run(&r, &p, false, &None, &NullReporter).err().unwrap().contains("onaylamadan"));

        // anahtarı sabitle: aynı anahtarla tekrar hazırlanınca artık yeni değil
        let mut t = sync::load_trust(&r);
        t.pin("play.example.com:25570", &p.key);
        sync::save_trust(&r, &t).unwrap();
        let again = prepare_from_manifest(&r, inv(), signed(1, vec![]), "files.example.com").unwrap();
        assert!(!again.is_new);
        // başka anahtarla imzalı manifest reddedilir
        let evil = prepare_from_manifest(&r, inv(), signed(2, vec![]), "files.example.com");
        assert!(evil.err().unwrap().contains("anahtarı değişmiş"));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn prepare_rejects_unsigned_and_untrusted_urls() {
        let r = tmp("bad");
        let mut m = signed(1, vec![entry("create", b"c")]);
        m.signature = None;
        assert!(prepare_from_manifest(&r, inv(), m, "x.com").err().unwrap().contains("imzası"));
        let mut e = entry("evil", b"e");
        e.url = "https://evil.example/e.jar".into();
        assert!(prepare_from_manifest(&r, inv(), signed(1, vec![e]), "x.com").is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn view_covers_ready_update_disable_keep() {
        let r = tmp("view");
        let dir = r.join("instances/server-play.example.com-25570/mods");
        fs::create_dir_all(&dir).unwrap();
        let (a, b) = (crate::sync::tests_jar("sodium", "*", "a"), crate::sync::tests_jar("lithium", "*", "old"));
        fs::write(dir.join("sodium.jar"), &a).unwrap();
        fs::write(dir.join("lithium.jar"), &b).unwrap();
        fs::write(dir.join("iris.jar"), crate::sync::tests_jar("iris", "client", "i")).unwrap();
        fs::write(dir.join("optifine.jar"), crate::sync::tests_jar("optifine", "*", "o")).unwrap();
        let m = signed(1, vec![entry("sodium", &a), entry("lithium", b"new-lithium")]);
        let v = join_view(&prepare_from_manifest(&r, inv(), m, "x.com").unwrap());
        let st = |id: &str| v.rows.iter().find(|x| x.id == id).unwrap().status.clone();
        assert_eq!((st("sodium").as_str(), st("lithium").as_str(), st("iris").as_str(), st("optifine").as_str()), ("ready", "update", "keep", "disable"));
        let _ = fs::remove_dir_all(&r);
    }
}
