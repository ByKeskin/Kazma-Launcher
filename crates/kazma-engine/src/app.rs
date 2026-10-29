//! Uygulama düzeyi işlemler: ayarlar, instance kaydı, oyunu başlatma ve server'a katılma.
//! Arayüz yalnızca bu fonksiyonları çağırır.

use crate::install::{self, GameExit};
use crate::net::*;
use crate::report::Reporter;
use crate::sync;
use kazma_core::instance::{self, InstanceSpec, LoaderSpec};
use kazma_core::invite::{self, Invite};
use kazma_core::launch::{Account, LaunchOptions};
use kazma_core::manifest::{self, PackManifest, SyncPlan};
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
}

impl Default for Config {
    fn default() -> Self {
        Config { player: String::new(), ram: default_ram(), last_instance: None }
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
    c.player = name.to_string();
    save_config(root, &c)?;
    Ok(c)
}

// ---------- instance kaydı ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub id: String,
    pub name: String,
    pub game: String,
    /// "vanilla" ya da "fabric"
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
    /// Kayıtta tutulmaz, listelerken hesaplanır.
    #[serde(default, skip_deserializing)]
    pub mods: usize,
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

pub fn spec_of(i: &InstanceInfo) -> R<InstanceSpec> {
    let loader = match i.loader.as_str() {
        "vanilla" => LoaderSpec::Vanilla,
        "fabric" => LoaderSpec::Fabric { version: i.loader_version.clone() },
        o => return Err(format!("desteklenmeyen yükleyici: {o}")),
    };
    Ok(InstanceSpec { id: i.id.clone(), game: i.game.clone(), loader })
}

fn info_for(spec: &InstanceSpec, name: &str, kind: &str, host: Option<(&str, u16)>) -> InstanceInfo {
    let (loader, loader_version) = match &spec.loader {
        LoaderSpec::Vanilla => ("vanilla", String::new()),
        LoaderSpec::Fabric { version } => ("fabric", version.clone()),
    };
    InstanceInfo {
        id: spec.id.clone(),
        name: name.to_string(),
        game: spec.game.clone(),
        loader: loader.into(),
        loader_version,
        kind: kind.into(),
        server_host: host.map(|h| h.0.to_string()),
        server_port: host.map(|h| h.1),
        last_played: None,
        mods: 0,
    }
}

fn clean_name(n: &str) -> R<String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 40 || n.chars().any(|c| c.is_control()) {
        return Err("instance adı 1-40 karakter olmalı".into());
    }
    Ok(n.to_string())
}

/// `fabric`: None = vanilla, Some(None) = en son kararlı Fabric, Some(Some(v)) = belirli sürüm.
pub fn create_instance(root: &Path, name: &str, game: &str, fabric: Option<Option<String>>) -> R<InstanceInfo> {
    let name = clean_name(name)?;
    let loader = match fabric {
        None => None,
        Some(Some(v)) => Some(v),
        Some(None) => Some(install::latest_stable_fabric(game)?),
    };
    let spec = instance::from_selection(game, loader.as_deref()).map_err(|e| e.to_string())?;
    if info_path(root, &spec.id).is_file() {
        let ex = read_info(root, &spec.id)?;
        return Err(format!("Bu sürüm için zaten bir instance var: {}", ex.name));
    }
    let info = info_for(&spec, &name, "player", None);
    write_info(root, &info)?;
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
    Account::offline(&cfg.player).map_err(|e| e.to_string())
}

pub fn launch_instance(root: &Path, id: &str, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<GameExit> {
    let cfg = load_config(root);
    let acct = account_from(&cfg)?;
    let info = read_info(root, id)?;
    let spec = spec_of(&info)?;
    let env = Env::current();
    let inst = install::ensure_installed(root, &spec, &env, rep)?;
    let join = match (&info.kind[..], &info.server_host, info.server_port) {
        ("server", Some(h), Some(p)) => Some((h.clone(), p)),
        _ => None,
    };
    let opts = LaunchOptions { max_mb: cfg.ram, join_server: join, ..Default::default() };
    mark_played(root, id);
    install::launch_game(root, &spec, &inst, &env, &acct, &opts, java, rep)
}

// ---------- server'a katılma ----------

#[derive(Debug, Clone)]
pub struct Prepared {
    pub invite: Invite,
    pub manifest: PackManifest,
    pub key: String,
    pub fingerprint: String,
    pub is_new: bool,
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
    Ok(Prepared { fingerprint: fingerprint(&key), invite: inv, manifest: m, key, is_new, spec, plan })
}

pub fn join_prepare(root: &Path, invite_text: &str, rep: &dyn Reporter) -> R<Prepared> {
    let inv = invite::parse(invite_text).map_err(|e| e.to_string())?;
    let murl = inv.manifest_url.clone().ok_or("Davette manifest adresi yok (?m=...). Server sahibinden tam davet bağlantısını iste.")?;
    rep.stage("Server manifesti alınıyor");
    let m = sync::fetch_manifest(&murl)?;
    let host = url::Url::parse(&murl).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    prepare_from_manifest(root, inv, m, &host)
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
    if p.is_new {
        if !trust_new {
            return Err("Bu server'a güvendiğini onaylamadan katılamazsın.".into());
        }
        let mut t = sync::load_trust(root);
        t.pin(&server_id, &p.key);
        sync::save_trust(root, &t)?;
    }
    write_info(root, &{
        let mut i = info_for(&p.spec, &p.manifest.name, "server", Some((&p.invite.host, p.invite.port)));
        i.last_played = read_info(root, &p.spec.id).ok().and_then(|x| x.last_played);
        i
    })?;
    let env = Env::current();
    let inst = install::ensure_installed(root, &p.spec, &env, rep)?;

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
        let a = create_instance(&r, "Vanilla 1.20.1", "1.20.1", None).unwrap();
        assert_eq!((a.id.as_str(), a.loader.as_str(), a.kind.as_str()), ("1.20.1-vanilla", "vanilla", "player"));
        let b = create_instance(&r, "Modlu", "1.21.1", Some(Some("0.16.9".into()))).unwrap();
        assert_eq!(b.id, "1.21.1-fabric-0.16.9");
        assert!(create_instance(&r, "Kopya", "1.20.1", None).unwrap_err().contains("zaten"), "aynı sürüm tekrarlanmaz");
        assert!(create_instance(&r, "  ", "1.20.1", None).is_err());
        assert!(create_instance(&r, "x", "../kotu", None).is_err());
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
    fn launch_needs_player_name() {
        let r = tmp("noplayer");
        create_instance(&r, "A", "1.20.1", None).unwrap();
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
