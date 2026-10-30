//! Diğer launcher'lardaki (resmi, TLauncher, SKLauncher, Prism, CurseForge, Modrinth, ATLauncher...) dünyaları,
//! ayarları ve modları Kazma instance'larına KOPYALAR; asıl dosyalara dokunulmaz. Hesap dosyalarından yalnızca
//! oyuncu adı, kimlik (UUID) ve hesap türü okunur; erişim belirteçleri (token) hiçbir zaman okunup saklanmaz.

use crate::net::*;
use crate::report::Reporter;
use kazma_core::account::validate_player_name;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorldInfo {
    pub name: String,
    pub modified: u64,
    pub size_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub version_id: String,
    pub game: Option<String>,
    pub loader: Option<String>,
    pub loader_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub launcher: String,
    pub path: String,
    pub worlds: Vec<WorldInfo>,
    pub profiles: Vec<ProfileInfo>,
    pub mods: usize,
    pub resourcepacks: usize,
    pub shaderpacks: usize,
    pub has_options: bool,
    pub has_servers: bool,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub launcher: String,
    pub dir: PathBuf,
    /// Instance kök klasörü (Prism, CurseForge gibi): sürüm bilgisi buradan okunur.
    pub meta_dir: Option<PathBuf>,
}

// ---------- sürüm kimliği ayrıştırma ----------

fn is_game(s: &str) -> bool {
    let snap = s.len() == 6 && s.as_bytes()[2] == b'w' && s[..2].bytes().all(|b| b.is_ascii_digit()) && s[3..5].bytes().all(|b| b.is_ascii_digit()) && s.as_bytes()[5].is_ascii_lowercase();
    let rel = s.contains('.') && s.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    snap || rel
}

/// NeoForge sürümünden Minecraft sürümü: 21.1.172 -> 1.21.1, 21.0.5 -> 1.21.
fn neoforge_game(lv: &str) -> Option<String> {
    let mut it = lv.split('.');
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    if major < 20 {
        return None;
    }
    Some(if minor == 0 { format!("1.{major}") } else { format!("1.{major}.{minor}") })
}

/// Resmi launcher'ın sürüm kimliğinden (game, yükleyici, yükleyici sürümü).
pub fn parse_version_id(id: &str) -> Option<(String, String, String)> {
    let two = |rest: &str| rest.rsplit_once('-').filter(|(lv, g)| is_game(g) && !lv.is_empty()).map(|(lv, g)| (g.to_string(), lv.to_string()));
    if let Some(r) = id.strip_prefix("fabric-loader-") {
        return two(r).map(|(g, lv)| (g, "fabric".into(), lv));
    }
    if let Some(r) = id.strip_prefix("quilt-loader-") {
        return two(r).map(|(g, lv)| (g, "quilt".into(), lv));
    }
    if let Some(lv) = id.strip_prefix("neoforge-") {
        return neoforge_game(lv).map(|g| (g, "neoforge".into(), lv.to_string()));
    }
    if let Some((g, lv)) = id.split_once("-forge-") {
        return (is_game(g) && !lv.is_empty()).then(|| (g.to_string(), "forge".into(), lv.to_string()));
    }
    is_game(id).then(|| (id.to_string(), "vanilla".into(), String::new()))
}

fn profile(name: &str, version_id: &str) -> ProfileInfo {
    let p = parse_version_id(version_id);
    ProfileInfo { name: name.into(), version_id: version_id.into(), game: p.as_ref().map(|x| x.0.clone()), loader: p.as_ref().map(|x| x.1.clone()), loader_version: p.as_ref().map(|x| x.2.clone()) }
}

/// `launcher_profiles.json` ve `TlauncherProfiles.json`: profiller ve son kullanılan sürüm kimlikleri.
pub fn parse_launcher_profiles(json: &str) -> Vec<ProfileInfo> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    let Some(o) = v.get("profiles").and_then(Value::as_object) else { return vec![] };
    let mut out: Vec<ProfileInfo> = o
        .iter()
        .filter_map(|(k, p)| {
            let ver = p.get("lastVersionId").and_then(Value::as_str)?;
            if ver.starts_with("latest") {
                return None;
            }
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(k);
            Some(profile(name, ver))
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.version_id == b.version_id);
    out
}

/// Prism/MultiMC `mmc-pack.json`: bileşenlerden oyun sürümü ve yükleyici.
pub fn parse_mmc_pack(json: &str, name: &str) -> Option<ProfileInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let comps = v.get("components")?.as_array()?;
    let ver = |uid: &str| comps.iter().find(|c| c.get("uid").and_then(Value::as_str) == Some(uid)).and_then(|c| c.get("version")).and_then(Value::as_str).map(String::from);
    let game = ver("net.minecraft")?;
    let (loader, lv) = [("net.fabricmc.fabric-loader", "fabric"), ("org.quiltmc.quilt-loader", "quilt"), ("net.neoforged", "neoforge"), ("net.minecraftforge", "forge")]
        .iter()
        .find_map(|(uid, k)| ver(uid).map(|v| (k.to_string(), v)))
        .unwrap_or(("vanilla".into(), String::new()));
    Some(ProfileInfo { name: name.into(), version_id: format!("{game} ({loader})"), game: Some(game), loader: Some(loader), loader_version: Some(lv) })
}

/// CurseForge `minecraftinstance.json`: `gameVersion` ve `baseModLoader.name` ("forge-47.3.0").
pub fn parse_curseforge_instance(json: &str, name: &str) -> Option<ProfileInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let game = v.get("gameVersion")?.as_str()?.to_string();
    let (loader, lv) = v
        .pointer("/baseModLoader/name")
        .and_then(Value::as_str)
        .and_then(|n| {
            let mut it = n.split('-');
            let k = it.next()?.to_string();
            let ver = it.next()?.to_string();
            ["forge", "fabric", "quilt", "neoforge"].contains(&k.as_str()).then_some((k, ver))
        })
        .unwrap_or(("vanilla".into(), String::new()));
    Some(ProfileInfo { name: name.into(), version_id: format!("{game} ({loader})"), game: Some(game), loader: Some(loader), loader_version: Some(lv) })
}

// ---------- kaynakları bulma ----------

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| match e.file_type() {
        Ok(t) if t.is_dir() => dir_size(&e.path()),
        Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
        _ => 0,
    }).sum()
}

fn mtime(p: &Path) -> u64 {
    fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn count(dir: &Path, ext: Option<&str>) -> usize {
    fs::read_dir(dir).map(|r| r.flatten().filter(|e| ext.map_or(true, |x| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(x))).count()).unwrap_or(0)
}

pub fn list_worlds(dir: &Path) -> Vec<WorldInfo> {
    let mut w: Vec<WorldInfo> = fs::read_dir(dir.join("saves"))
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().join("level.dat").is_file())
                .map(|e| WorldInfo { name: e.file_name().to_string_lossy().into_owned(), modified: mtime(&e.path().join("level.dat")), size_mb: dir_size(&e.path()) / (1 << 20) })
                .collect()
        })
        .unwrap_or_default();
    w.sort_by(|a, b| b.modified.cmp(&a.modified));
    w
}

fn source_id(path: &Path) -> String {
    sha1_hex(path.to_string_lossy().as_bytes())[..10].to_string()
}

pub fn scan(c: &Candidate) -> Option<Source> {
    let d = &c.dir;
    if !d.is_dir() || !(d.join("saves").is_dir() || d.join("versions").is_dir() || d.join("mods").is_dir()) {
        return None;
    }
    let read = |p: PathBuf| fs::read_to_string(p).ok();
    let mut profiles = Vec::new();
    for f in ["launcher_profiles.json", "TlauncherProfiles.json"] {
        if let Some(t) = read(d.join(f)) {
            profiles.extend(parse_launcher_profiles(&t));
        }
    }
    profiles.dedup_by(|a, b| a.version_id == b.version_id);
    if let Some(m) = &c.meta_dir {
        let nm = m.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(p) = read(m.join("mmc-pack.json")).and_then(|t| parse_mmc_pack(&t, &nm)) {
            profiles.push(p);
        } else if let Some(p) = read(m.join("minecraftinstance.json")).and_then(|t| parse_curseforge_instance(&t, &nm)) {
            profiles.push(p);
        }
    }
    Some(Source {
        id: source_id(d),
        launcher: c.launcher.clone(),
        path: d.to_string_lossy().into_owned(),
        worlds: list_worlds(d),
        profiles,
        mods: count(&d.join("mods"), Some(".jar")),
        resourcepacks: count(&d.join("resourcepacks"), None),
        shaderpacks: count(&d.join("shaderpacks"), None),
        has_options: d.join("options.txt").is_file(),
        has_servers: d.join("servers.dat").is_file(),
    })
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    fs::read_dir(p).map(|r| r.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect()).unwrap_or_default()
}

/// Bilinen konumlar (Windows) + kullanıcının eklediği özel klasörler.
pub fn default_candidates(custom: &[String]) -> Vec<Candidate> {
    let var = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    let (appdata, home) = (var("APPDATA"), var("USERPROFILE").or_else(|| var("HOME")));
    let mut out = Vec::new();
    if let Some(a) = &appdata {
        let mc = a.join(".minecraft");
        let mut label = Vec::new();
        if mc.join("launcher_accounts.json").is_file() {
            label.push("Resmi Minecraft Launcher");
        }
        if mc.join("TlauncherProfiles.json").is_file() {
            label.push("TLauncher");
        }
        let label = if label.is_empty() { ".minecraft (SKLauncher ve benzerleri)".to_string() } else { label.join(" / ") };
        out.push(Candidate { launcher: label, dir: mc, meta_dir: None });
        for (folder, sub, name) in [("PrismLauncher/instances", "", "Prism"), ("MultiMC/instances", "", "MultiMC"), ("ATLauncher/instances", "", "ATLauncher"), ("gdlauncher_next/instances", "", "GDLauncher"), ("ModrinthApp/profiles", "", "Modrinth")] {
            for inst in subdirs(&a.join(folder)) {
                let game = ["minecraft", ".minecraft"].iter().map(|s| inst.join(s)).find(|p| p.is_dir()).unwrap_or_else(|| inst.join(sub));
                out.push(Candidate { launcher: format!("{name}: {}", inst.file_name().unwrap_or_default().to_string_lossy()), dir: game, meta_dir: Some(inst) });
            }
        }
    }
    if let Some(h) = &home {
        for inst in subdirs(&h.join("curseforge/minecraft/Instances")) {
            out.push(Candidate { launcher: format!("CurseForge: {}", inst.file_name().unwrap_or_default().to_string_lossy()), dir: inst.clone(), meta_dir: Some(inst) });
        }
    }
    for c in custom {
        out.push(Candidate { launcher: "Özel klasör".into(), dir: PathBuf::from(c), meta_dir: None });
    }
    out
}

pub fn scan_all(custom: &[String]) -> Vec<Source> {
    let mut v: Vec<Source> = default_candidates(custom).iter().filter_map(scan).collect();
    v.dedup_by(|a, b| a.id == b.id);
    v
}

// ---------- hesap adları (belirteçler okunmaz) ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FoundAccount {
    pub launcher: String,
    pub name: String,
    /// tire ile yazılmış küçük harf UUID
    pub uuid: String,
    /// premium | çevrimdışı
    pub kind: String,
}

pub fn dash_uuid(raw: &str) -> Option<String> {
    let h: String = raw.chars().filter(|c| *c != '-').collect::<String>().to_ascii_lowercase();
    (h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn acct(launcher: &str, name: &str, uuid: &str, premium: bool) -> Option<FoundAccount> {
    validate_player_name(name).ok()?;
    Some(FoundAccount { launcher: launcher.into(), name: name.into(), uuid: dash_uuid(uuid)?, kind: if premium { "premium" } else { "çevrimdışı" }.into() })
}

/// Resmi launcher: `launcher_accounts.json`.
pub fn parse_official_accounts(json: &str) -> Vec<FoundAccount> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    v.get("accounts").and_then(Value::as_object).map(|o| {
        o.values().filter_map(|a| acct("Resmi launcher", a.pointer("/minecraftProfile/name")?.as_str()?, a.pointer("/minecraftProfile/id")?.as_str()?, true)).collect()
    }).unwrap_or_default()
}

/// TLauncher: `TlauncherProfiles.json` içindeki `accounts`.
pub fn parse_tlauncher_accounts(json: &str) -> Vec<FoundAccount> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    let list: Vec<&Value> = match v.get("accounts") {
        Some(Value::Object(o)) => o.values().collect(),
        Some(Value::Array(a)) => a.iter().collect(),
        _ => return vec![],
    };
    list.into_iter().filter_map(|a| {
        let name = a.get("username").or_else(|| a.get("displayName")).and_then(Value::as_str)?;
        let uuid = a.get("uuid").and_then(Value::as_str)?;
        let kind = a.get("type").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        acct("TLauncher", name, uuid, !(kind == "free" || kind == "offline"))
    }).collect()
}

/// Prism/MultiMC: `accounts.json`.
pub fn parse_prism_accounts(json: &str) -> Vec<FoundAccount> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    v.get("accounts").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(|x| acct("Prism", x.pointer("/profile/name")?.as_str()?, x.pointer("/profile/id")?.as_str()?, x.get("type").and_then(Value::as_str) != Some("Offline"))).collect()
    }).unwrap_or_default()
}

pub fn scan_accounts(custom: &[String]) -> Vec<FoundAccount> {
    let mut out = Vec::new();
    let read = |p: PathBuf| fs::read_to_string(p).ok();
    for c in default_candidates(custom) {
        if let Some(t) = read(c.dir.join("launcher_accounts.json")) {
            out.extend(parse_official_accounts(&t));
        }
        if let Some(t) = read(c.dir.join("TlauncherProfiles.json")) {
            out.extend(parse_tlauncher_accounts(&t));
        }
    }
    if let Ok(a) = std::env::var("APPDATA") {
        if let Some(t) = read(PathBuf::from(a).join("PrismLauncher/accounts.json")) {
            out.extend(parse_prism_accounts(&t));
        }
    }
    out.sort_by(|a, b| (&a.name, &a.uuid).cmp(&(&b.name, &b.uuid)));
    out.dedup_by(|a, b| a.name == b.name && a.uuid == b.uuid);
    out
}

// ---------- kopyalama ----------

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Extras {
    #[serde(default)]
    pub options: bool,
    #[serde(default)]
    pub servers: bool,
    #[serde(default)]
    pub resourcepacks: bool,
    #[serde(default)]
    pub shaderpacks: bool,
    #[serde(default)]
    pub mods: bool,
}

/// Dizini kopyalar. Sembolik bağlar izlenmez (kaynak dışına çıkılmaz), `session.lock` atlanır. Dönen: kopyalanan dosya sayısı.
fn copy_dir(src: &Path, dst: &Path, skip_existing: bool) -> R<u64> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut n = 0;
    for e in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?.flatten() {
        let name = e.file_name();
        if name == "session.lock" {
            continue;
        }
        let t = e.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(&name);
        if t.is_symlink() {
            continue;
        }
        if t.is_dir() {
            n += copy_dir(&e.path(), &to, skip_existing)?;
        } else if t.is_file() {
            if skip_existing && to.exists() {
                continue;
            }
            fs::copy(e.path(), &to).map_err(|er| format!("{}: {er}", to.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let mut p = parent.join(name);
    let mut i = 2;
    while p.exists() {
        p = parent.join(format!("{name} ({i})"));
        i += 1;
    }
    p
}

fn safe_world(n: &str) -> bool {
    !n.is_empty() && !n.contains(['/', '\\', ':']) && n != "." && n != ".." && !n.chars().any(|c| c.is_control())
}

/// Seçili dünyaları `dst_dir/saves` altına kopyalar. Aynı adlı dünya varsa yenisi "Ad (2)" olarak eklenir.
pub fn import_worlds(src_dir: &Path, dst_dir: &Path, worlds: &[String], rep: &dyn Reporter) -> R<Vec<String>> {
    let saves = dst_dir.join("saves");
    let mut done = Vec::new();
    for (i, w) in worlds.iter().enumerate() {
        if !safe_world(w) {
            return Err(format!("dünya adı geçersiz: {w}"));
        }
        let from = src_dir.join("saves").join(w);
        if !from.join("level.dat").is_file() {
            return Err(format!("{w} bir Minecraft dünyası değil ya da bulunamadı"));
        }
        rep.stage(&format!("Dünya kopyalanıyor: {w} ({}/{})", i + 1, worlds.len()));
        fs::create_dir_all(&saves).map_err(|e| e.to_string())?;
        let to = unique_dir(&saves, w);
        copy_dir(&from, &to, false)?;
        done.push(to.file_name().unwrap_or_default().to_string_lossy().into_owned());
    }
    Ok(done)
}

/// Ayar dosyalarını ve paket/mod klasörlerini kopyalar. Hedefte zaten olan dosyaların üzerine yazılmaz.
pub fn import_extras(src_dir: &Path, dst_dir: &Path, x: &Extras, rep: &dyn Reporter) -> R<Vec<String>> {
    let mut notes = Vec::new();
    fs::create_dir_all(dst_dir).map_err(|e| e.to_string())?;
    for (on, file) in [(x.options, "options.txt"), (x.servers, "servers.dat")] {
        if !on {
            continue;
        }
        let (from, to) = (src_dir.join(file), dst_dir.join(file));
        if !from.is_file() {
            notes.push(format!("{file} kaynakta yok"));
        } else if to.exists() {
            notes.push(format!("{file} hedefte zaten var, değiştirilmedi"));
        } else {
            fs::copy(&from, &to).map_err(|e| format!("{file}: {e}"))?;
            notes.push(format!("{file} kopyalandı"));
        }
    }
    for (on, folder) in [(x.resourcepacks, "resourcepacks"), (x.shaderpacks, "shaderpacks"), (x.mods, "mods")] {
        if !on || !src_dir.join(folder).is_dir() {
            continue;
        }
        rep.stage(&format!("{folder} kopyalanıyor"));
        let n = copy_dir(&src_dir.join(folder), &dst_dir.join(folder), true)?;
        notes.push(format!("{folder}: {n} dosya kopyalandı"));
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-imp-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    fn world(dir: &Path, name: &str, extra: &str) {
        let w = dir.join("saves").join(name);
        fs::create_dir_all(w.join("region")).unwrap();
        fs::write(w.join("level.dat"), b"nbt").unwrap();
        fs::write(w.join("session.lock"), b"x").unwrap();
        fs::write(w.join("region/r.0.0.mca"), extra).unwrap();
    }

    #[test]
    fn version_ids_roundtrip_with_instance_ids() {
        use kazma_core::instance::from_selection;
        for (id, want) in [
            ("1.21.1", ("1.21.1", "vanilla", "")),
            ("fabric-loader-0.16.9-1.21.1", ("1.21.1", "fabric", "0.16.9")),
            ("quilt-loader-0.26.1-1.20.4", ("1.20.4", "quilt", "0.26.1")),
            ("1.20.1-forge-47.3.0", ("1.20.1", "forge", "47.3.0")),
            ("neoforge-21.1.172", ("1.21.1", "neoforge", "21.1.172")),
            ("neoforge-21.0.5", ("1.21", "neoforge", "21.0.5")),
            ("24w14a", ("24w14a", "vanilla", "")),
        ] {
            let p = parse_version_id(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!((p.0.as_str(), p.1.as_str(), p.2.as_str()), want, "{id}");
            if p.1 != "vanilla" {
                assert_eq!(from_selection(&p.0, &p.1, &p.2).unwrap().version_id(), id, "kimlik geri üretilebilmeli");
            }
        }
        for bad in ["latest-release", "OptiFine_1.8.9", "ForgeOptiFine 1.12.2", "", "fabric-loader-1.21.1", "neoforge-19.1.2"] {
            assert!(parse_version_id(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn reads_profiles_prism_and_curseforge() {
        let p = parse_launcher_profiles(r#"{"profiles":{"a":{"name":"Modlu","lastVersionId":"fabric-loader-0.16.9-1.21.1"},"b":{"lastVersionId":"latest-release"},"c":{"name":"","lastVersionId":"1.20.1"},"d":{"name":"Garip","lastVersionId":"MyCustom"}}}"#);
        assert_eq!(p.len(), 3);
        assert_eq!(p.iter().find(|x| x.name == "Modlu").unwrap().loader.as_deref(), Some("fabric"));
        assert_eq!(p.iter().find(|x| x.name == "c").unwrap().game.as_deref(), Some("1.20.1"), "adı boşsa anahtar kullanılır");
        assert_eq!(p.iter().find(|x| x.name == "Garip").unwrap().game, None, "tanınmayan sürüm kimliği");
        let m = parse_mmc_pack(r#"{"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"net.minecraftforge","version":"47.3.0"}]}"#, "Kule").unwrap();
        assert_eq!((m.game.as_deref(), m.loader.as_deref(), m.loader_version.as_deref()), (Some("1.20.1"), Some("forge"), Some("47.3.0")));
        let c = parse_curseforge_instance(r#"{"gameVersion":"1.21.1","baseModLoader":{"name":"neoforge-21.1.172"}}"#, "Pack").unwrap();
        assert_eq!((c.loader.as_deref(), c.loader_version.as_deref()), (Some("neoforge"), Some("21.1.172")));
        assert!(parse_launcher_profiles("{").is_empty());
    }

    #[test]
    fn scans_a_game_folder() {
        let d = tmp("scan");
        world(&d, "Yaşam", "aaaa");
        world(&d, "Eski", "b");
        fs::create_dir_all(d.join("saves/BosKlasor")).unwrap();
        fs::create_dir_all(d.join("mods")).unwrap();
        fs::write(d.join("mods/a.jar"), b"x").unwrap();
        fs::write(d.join("options.txt"), b"o").unwrap();
        fs::write(d.join("launcher_profiles.json"), r#"{"profiles":{"x":{"name":"P","lastVersionId":"1.21.1"}}}"#).unwrap();
        let s = scan(&Candidate { launcher: "Test".into(), dir: d.clone(), meta_dir: None }).unwrap();
        assert_eq!(s.worlds.len(), 2, "level.dat olmayan klasör dünya sayılmaz");
        assert_eq!((s.mods, s.has_options, s.has_servers, s.profiles.len()), (1, true, false, 1));
        assert_eq!(s.id.len(), 10);
        assert!(scan(&Candidate { launcher: "x".into(), dir: d.join("yok"), meta_dir: None }).is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn imports_worlds_without_touching_the_source_and_never_overwrites() {
        let (src, dst) = (tmp("src"), tmp("dst"));
        world(&src, "Dunyam", "veri");
        let first = import_worlds(&src, &dst, &["Dunyam".into()], &NullReporter).unwrap();
        assert_eq!(first, ["Dunyam"]);
        assert!(dst.join("saves/Dunyam/region/r.0.0.mca").is_file());
        assert!(!dst.join("saves/Dunyam/session.lock").exists(), "kilit dosyası kopyalanmaz");
        assert!(src.join("saves/Dunyam/session.lock").is_file(), "kaynak olduğu gibi kalır");
        let second = import_worlds(&src, &dst, &["Dunyam".into()], &NullReporter).unwrap();
        assert_eq!(second, ["Dunyam (2)"], "var olan dünya ezilmez");
        for bad in ["../x", "a/b", "..", "C:\\x"] {
            assert!(import_worlds(&src, &dst, &[bad.into()], &NullReporter).is_err(), "{bad}");
        }
        assert!(import_worlds(&src, &dst, &["Yok".into()], &NullReporter).is_err());
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn extras_are_copied_but_existing_files_win() {
        let (src, dst) = (tmp("xs"), tmp("xd"));
        fs::write(src.join("options.txt"), b"yeni").unwrap();
        fs::write(src.join("servers.dat"), b"srv").unwrap();
        fs::create_dir_all(src.join("resourcepacks")).unwrap();
        fs::write(src.join("resourcepacks/a.zip"), b"z").unwrap();
        fs::create_dir_all(src.join("mods")).unwrap();
        fs::write(src.join("mods/m.jar"), b"m").unwrap();
        fs::write(dst.join("options.txt"), b"eski").unwrap();
        let x = Extras { options: true, servers: true, resourcepacks: true, shaderpacks: true, mods: false };
        let notes = import_extras(&src, &dst, &x, &NullReporter).unwrap();
        assert_eq!(fs::read(dst.join("options.txt")).unwrap(), b"eski", "hedefteki ayar korunur");
        assert!(dst.join("servers.dat").is_file() && dst.join("resourcepacks/a.zip").is_file());
        assert!(!dst.join("mods").exists(), "istenmeyen klasör kopyalanmaz");
        assert!(notes.iter().any(|n| n.contains("options.txt") && n.contains("zaten var")));
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let (src, dst) = (tmp("ls"), tmp("ld"));
        let secret = tmp("secret");
        fs::write(secret.join("gizli.txt"), b"sifre").unwrap();
        world(&src, "W", "x");
        std::os::unix::fs::symlink(&secret, src.join("saves/W/link")).unwrap();
        import_worlds(&src, &dst, &["W".into()], &NullReporter).unwrap();
        assert!(!dst.join("saves/W/link").exists(), "sembolik bağ üzerinden dışarıdaki dosyalar alınmaz");
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
        let _ = fs::remove_dir_all(&secret);
    }

    #[test]
    fn account_parsers_only_expose_names_and_ids() {
        let off = parse_official_accounts(r#"{"accounts":{"k1":{"accessToken":"GIZLI-TOKEN","type":"Xbox","minecraftProfile":{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}}}}"#);
        assert_eq!(off, vec![FoundAccount { launcher: "Resmi launcher".into(), name: "Notch".into(), uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(), kind: "premium".into() }]);
        assert!(!format!("{off:?}").contains("GIZLI"), "belirteç yapıya girmez");
        let tl = parse_tlauncher_accounts(r#"{"accounts":{"a":{"username":"Efe_Kral","uuid":"f0180f747e2235a189533717dee31846","type":"free"},"b":{"username":"x@mail.com","uuid":"069a79f444e94726a5befca90e38aaf5","type":"microsoft"}}}"#);
        assert_eq!(tl.len(), 1, "geçersiz (e-posta) ad atılır");
        assert_eq!((tl[0].name.as_str(), tl[0].kind.as_str()), ("Efe_Kral", "çevrimdışı"));
        let pr = parse_prism_accounts(r#"{"accounts":[{"type":"MSA","profile":{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}},{"type":"Offline","profile":{"id":"f0180f747e2235a189533717dee31846","name":"Efe_Kral"}}]}"#);
        assert_eq!(pr.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(), ["premium", "çevrimdışı"]);
        assert!(parse_official_accounts("{").is_empty());
        assert_eq!(dash_uuid("ZZ"), None);
        assert_eq!(dash_uuid("069A79F4-44E9-4726-A5BE-FCA90E38AAF5").as_deref(), Some("069a79f4-44e9-4726-a5be-fca90e38aaf5"));
    }
}
