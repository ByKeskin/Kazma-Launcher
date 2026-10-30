//! Kendi bilgisayarımızda Minecraft server'ı kurma ve yönetme: jar indirme (Vanilla, Paper, Fabric, Forge, NeoForge),
//! başlat / durdur / yeniden başlat, konsol (stdin/stdout), server.properties ayarları, başka makineye taşınabilir paket.

use crate::net::*;
use crate::report::Reporter;
use kazma_core::mojang::{self, plain};
use kazma_core::version::Env;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const KINDS: [&str; 5] = ["vanilla", "paper", "fabric", "forge", "neoforge"];
const LOG_KEEP: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerSpec {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub game: String,
    #[serde(default)]
    pub loader_version: String,
    pub port: u16,
    pub ram: u32,
    /// "local" (bu bilgisayar) ya da "remote" (başka makine için paket)
    pub mode: String,
    #[serde(default)]
    pub upnp: bool,
    #[serde(default)]
    pub created: u64,
    /// Kurulumda belirlenen Java ana sürümü (0 = sürüme göre tahmin).
    #[serde(default)]
    pub java_major: u32,
    /// Oyuncuların bağlanacağı adres (UPnP dış IP'si ya da tünel adresi); boşsa bilinmiyor.
    #[serde(default)]
    pub public_address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRequest {
    pub name: String,
    pub kind: String,
    pub game: String,
    pub port: Option<u16>,
    pub ram: Option<u32>,
    pub mode: String,
    #[serde(default)]
    pub upnp: bool,
    /// Mojang EULA kabul edildi mi? Kabul edilmeden server kurulmaz.
    pub eula: bool,
    /// Karma (çevrimdışı) girişe izin ver: online-mode=false
    #[serde(default)]
    pub cracked: bool,
}

// ---------- yollar ve doğrulama ----------

pub fn servers_root(root: &Path) -> PathBuf {
    root.join("servers")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn server_dir(root: &Path, id: &str) -> R<PathBuf> {
    if !valid_id(id) {
        return Err("geçersiz server kimliği".into());
    }
    Ok(servers_root(root).join(id))
}

pub fn data_dir(root: &Path, id: &str) -> R<PathBuf> {
    Ok(server_dir(root, id)?.join("data"))
}

fn slug(name: &str) -> String {
    let mut s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "server".into() } else { s.chars().take(30).collect() }
}

fn clean_name(n: &str) -> R<String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 40 || n.chars().any(|c| c.is_control()) {
        return Err("Server adı 1-40 karakter olmalı.".into());
    }
    Ok(n.to_string())
}

pub fn read_spec(root: &Path, id: &str) -> R<ServerSpec> {
    let t = fs::read_to_string(server_dir(root, id)?.join("kazma-server.json")).map_err(|_| "Server bulunamadı.".to_string())?;
    serde_json::from_str(&t).map_err(|e| format!("server kaydı okunamadı: {e}"))
}

fn write_spec(root: &Path, s: &ServerSpec) -> R<()> {
    write_atomic(&server_dir(root, &s.id)?.join("kazma-server.json"), serde_json::to_string_pretty(s).map_err(|e| e.to_string())?.as_bytes())
}

pub fn list_specs(root: &Path) -> Vec<ServerSpec> {
    let mut v: Vec<ServerSpec> = fs::read_dir(servers_root(root))
        .map(|r| r.flatten().filter_map(|e| read_spec(root, &e.file_name().to_string_lossy()).ok()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.created.cmp(&a.created));
    v
}

pub fn delete_server(root: &Path, id: &str) -> R<()> {
    let d = server_dir(root, id)?;
    if d.is_dir() {
        fs::remove_dir_all(&d).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Server'ın çalışması için gereken Java ana sürümü (Mojang'ın bildirdiği; bulunamazsa sürüme göre tahmin).
pub fn java_major_guess(game: &str) -> u32 {
    let mut it = game.split('.').skip(1);
    let minor: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(21);
    let patch: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    if minor >= 21 || (minor == 20 && patch >= 5) {
        21
    } else if minor >= 18 {
        17
    } else if minor == 17 {
        16
    } else {
        8
    }
}

// ---------- jar adresleri (saf ayrıştırma) ----------

pub struct JarSource {
    pub url: String,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

fn host_ok(url: &str, hosts: &[&str]) -> bool {
    url::Url::parse(url).ok().map_or(false, |u| u.scheme() == "https" && u.host_str().map_or(false, |h| hosts.contains(&h)))
}

/// Paper: `fill.papermc.io/v3/projects/paper/versions/<v>/builds` — en yüksek numaralı STABLE yapı.
pub fn paper_pick(builds_json: &str) -> R<JarSource> {
    let v: Value = serde_json::from_str(builds_json).map_err(|e| format!("Paper listesi okunamadı: {e}"))?;
    let arr = v.as_array().ok_or("Paper listesi beklenen biçimde değil")?;
    let best = arr
        .iter()
        .filter(|b| b.get("channel").and_then(Value::as_str) == Some("STABLE"))
        .max_by_key(|b| b.get("id").and_then(Value::as_u64).unwrap_or(0))
        .ok_or("Bu sürüm için kararlı Paper yapısı yok.")?;
    let d = best.pointer("/downloads/server:default").ok_or("Paper indirme bilgisi yok")?;
    let url = d.get("url").and_then(Value::as_str).ok_or("Paper indirme adresi yok")?.to_string();
    if !host_ok(&url, &["fill-data.papermc.io", "fill.papermc.io", "api.papermc.io"]) {
        return Err("Paper adresi güvenilir değil".into());
    }
    Ok(JarSource { url, sha1: None, sha256: d.pointer("/checksums/sha256").and_then(Value::as_str).map(String::from) })
}

pub fn fabric_server_url(game: &str, loader: &str, installer: &str) -> R<String> {
    if !plain(game) || !plain(loader) || !plain(installer) {
        return Err("sürüm adı geçersiz karakter içeriyor".into());
    }
    Ok(format!("https://meta.fabricmc.net/v2/versions/loader/{game}/{loader}/{installer}/server/jar"))
}

pub fn fabric_installer_pick(json: &str) -> R<String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    v.as_array()
        .and_then(|a| a.iter().find(|e| e.get("stable").and_then(Value::as_bool) == Some(true)))
        .and_then(|e| e.get("version").and_then(Value::as_str))
        .map(String::from)
        .ok_or_else(|| "kararlı Fabric kurucusu bulunamadı".into())
}

// ---------- kurulum ----------

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(b))
}

fn download_jar(src: &JarSource, dest: &Path, max: u64) -> R<()> {
    let bytes = get_bytes(&src.url, max)?;
    if let Some(h) = &src.sha1 {
        if !sha1_hex(&bytes).eq_ignore_ascii_case(h) {
            return Err("indirilen server dosyasının SHA-1 özeti uyuşmuyor".into());
        }
    }
    if let Some(h) = &src.sha256 {
        if !sha256_hex(&bytes).eq_ignore_ascii_case(h) {
            return Err("indirilen server dosyasının SHA-256 özeti uyuşmuyor".into());
        }
    }
    write_atomic(dest, &bytes)
}

fn initial_properties(spec: &ServerSpec, cracked: bool) -> String {
    format!("# Kazma tarafından oluşturuldu\nserver-port={}\nonline-mode={}\nmotd={}\nenable-query=false\n", spec.port, !cracked, spec.name.replace(['\n', '\r', '\\'], " "))
}

/// Sürücüyü ve jar'ı hazırlar. `cracked` doğruysa online-mode=false (çevrimdışı hesaplar girebilir).
pub fn create(root: &Path, req: &CreateRequest, env: &Env, rep: &dyn Reporter) -> R<ServerSpec> {
    if !req.eula {
        return Err("Server kurmak için Minecraft EULA'sını kabul etmelisin.".into());
    }
    let name = clean_name(&req.name)?;
    if !KINDS.contains(&req.kind.as_str()) {
        return Err("Bilinmeyen server türü.".into());
    }
    if !plain(&req.game) {
        return Err("Sürüm adı geçersiz.".into());
    }
    if !["local", "remote"].contains(&req.mode.as_str()) {
        return Err("Kurulum yeri geçersiz.".into());
    }
    let port = req.port.unwrap_or(25565);
    if port < 1024 {
        return Err("Port 1024-65535 arasında olmalı.".into());
    }
    let ram = req.ram.unwrap_or(4096);
    if !(512..=65536).contains(&ram) {
        return Err("RAM 512-65536 MB arasında olmalı.".into());
    }
    let id = {
        let base = slug(&name);
        let mut id = base.clone();
        let mut i = 2;
        while server_dir(root, &id)?.exists() {
            id = format!("{base}-{i}");
            i += 1;
        }
        id
    };
    let mut spec = ServerSpec { id: id.clone(), name, kind: req.kind.clone(), game: req.game.clone(), loader_version: String::new(), port, ram, mode: req.mode.clone(), upnp: req.upnp && req.mode == "local", created: now(), java_major: 0, public_address: None };
    let data = data_dir(root, &id)?;
    fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    let res = install_files(root, &mut spec, &data, req.cracked, env, rep);
    if let Err(e) = res {
        let _ = fs::remove_dir_all(server_dir(root, &id)?);
        return Err(e);
    }
    write_spec(root, &spec)?;
    Ok(spec)
}

fn install_files(root: &Path, spec: &mut ServerSpec, data: &Path, cracked: bool, env: &Env, rep: &dyn Reporter) -> R<()> {
    let jar = data.join("server.jar");
    rep.stage("Sürüm bilgisi alınıyor");
    let manifest = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL, 20 << 20)?).map_err(|e| e.to_string())?;
    let vj = crate::install::fetch_version_json(root, &spec.game, &manifest)?;
    let java_major = vj.java_version.as_ref().map(|j| j.major_version).unwrap_or_else(|| java_major_guess(&spec.game));
    spec.java_major = java_major;
    match spec.kind.as_str() {
        "vanilla" => {
            let a = vj.downloads.as_ref().and_then(|d| d.server.clone()).ok_or("Bu sürümün server dosyası yok (çok eski ya da yalnızca istemci).")?;
            if !kazma_core::plan::game_url_trusted(&a.url) {
                return Err(format!("güvenilmeyen adres: {}", a.url));
            }
            rep.stage("Vanilla server indiriliyor");
            download_jar(&JarSource { url: a.url, sha1: a.sha1, sha256: None }, &jar, 200 << 20)?;
        }
        "paper" => {
            rep.stage("Paper yapısı aranıyor");
            let src = paper_pick(&get_text(&format!("https://fill.papermc.io/v3/projects/paper/versions/{}/builds", spec.game), 20 << 20)?)?;
            rep.stage("Paper indiriliyor");
            download_jar(&src, &jar, 200 << 20)?;
        }
        "fabric" => {
            rep.stage("Fabric sürümü aranıyor");
            let lv = crate::install::latest_loader("fabric", &spec.game)?;
            let inst = fabric_installer_pick(&get_text("https://meta.fabricmc.net/v2/versions/installer", 5 << 20)?)?;
            spec.loader_version = lv.clone();
            rep.stage("Fabric server indiriliyor");
            download_jar(&JarSource { url: fabric_server_url(&spec.game, &lv, &inst)?, sha1: None, sha256: None }, &jar, 100 << 20)?;
        }
        k @ ("forge" | "neoforge") => {
            rep.stage(&format!("{k} sürümü aranıyor"));
            let lv = crate::install::latest_loader(k, &spec.game)?;
            spec.loader_version = lv.clone();
            let (url, sha_url) = crate::loaders::installer_urls(k, &spec.game, &lv)?;
            rep.stage(&format!("{k} kurucusu indiriliyor"));
            let bytes = get_bytes(&url, 200 << 20)?;
            let want = get_text(&sha_url, 4096)?.split_whitespace().next().unwrap_or("").to_string();
            if !sha1_hex(&bytes).eq_ignore_ascii_case(&want) {
                return Err("kurucunun SHA-1 özeti uyuşmuyor, çalıştırılmadı".into());
            }
            let installer = data.join("installer.jar");
            fs::write(&installer, &bytes).map_err(|e| e.to_string())?;
            let java_exe = crate::install::ensure_java(root, java_major, env, &None, rep)?;
            rep.stage(&format!("{k} server dosyaları kuruluyor (birkaç dakika sürebilir)"));
            let log = fs::File::create(data.join("installer.log")).map_err(|e| e.to_string())?;
            let err = log.try_clone().map_err(|e| e.to_string())?;
            let mut cmd = Command::new(&java_exe);
            cmd.arg("-jar").arg("installer.jar").arg("--installServer").current_dir(data).stdout(log).stderr(err);
            no_window(&mut cmd);
            let st = cmd.status().map_err(|e| format!("{}: {e}", java_exe.display()))?;
            if !st.success() {
                return Err(format!("{k} server kurulumu başarısız (kod {}). Günlük: {}", st.code().unwrap_or(-1), data.join("installer.log").display()));
            }
            let _ = fs::remove_file(&installer);
        }
        _ => unreachable!(),
    }
    fs::write(data.join("eula.txt"), "# Kullanıcı Kazma içinde kabul etti: https://aka.ms/MinecraftEULA\neula=true\n").map_err(|e| e.to_string())?;
    fs::write(data.join("server.properties"), initial_properties(spec, cracked)).map_err(|e| e.to_string())?;
    fs::write(data.join("user_jvm_args.txt"), format!("-Xms512M\n-Xmx{}M\n", spec.ram)).map_err(|e| e.to_string())?;
    rep.stage("Java denetleniyor");
    crate::install::ensure_java(root, java_major, env, &None, rep)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Konsol renk kodlarını (ESC [ ... harf) temizler.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if !c.is_control() || c == '\t' {
            out.push(c);
        }
    }
    out
}

fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

// ---------- başlatma argümanları ----------

/// Forge/NeoForge kurucusunun ürettiği `@args` dosyası (örn. libraries/net/neoforged/neoforge/21.1.1/win_args.txt).
fn find_args_file(data: &Path) -> Option<String> {
    let name = if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" };
    for base in ["libraries/net/neoforged/neoforge", "libraries/net/minecraftforge/forge"] {
        if let Ok(rd) = fs::read_dir(data.join(base)) {
            for e in rd.flatten() {
                if e.path().join(name).is_file() {
                    return Some(format!("{base}/{}/{name}", e.file_name().to_string_lossy()));
                }
            }
        }
    }
    None
}

/// Java'ya verilecek argümanlar (java yolu hariç).
pub fn launch_args(data: &Path, spec: &ServerSpec) -> R<Vec<String>> {
    let xmx = format!("-Xmx{}M", spec.ram);
    match spec.kind.as_str() {
        "forge" | "neoforge" => {
            if let Some(a) = find_args_file(data) {
                return Ok(vec!["@user_jvm_args.txt".into(), format!("@{a}"), "nogui".into()]);
            }
            let jar = fs::read_dir(data)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.starts_with("forge-") && n.ends_with(".jar") && !n.contains("installer"))
                .ok_or("Forge server dosyaları bulunamadı; server'ı yeniden kur.")?;
            Ok(vec!["-Xms512M".into(), xmx, "-jar".into(), jar, "nogui".into()])
        }
        _ => Ok(vec!["-Xms512M".into(), xmx, "-jar".into(), "server.jar".into(), "nogui".into()]),
    }
}

fn quote(a: &str) -> String {
    if a.contains(' ') { format!("\"{a}\"") } else { a.to_string() }
}

/// Başka bir makinede çalıştırmak için paket: tüm dosyalar + start.bat / start.sh. Java ayrıca kurulu olmalı.
pub fn export_package(root: &Path, id: &str) -> R<PathBuf> {
    let spec = read_spec(root, id)?;
    let data = data_dir(root, id)?;
    let args = launch_args(&data, &spec)?;
    let java = java_major_guess(&spec.game);
    let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    let bat = format!("@echo off\r\ncd /d \"%~dp0\"\r\nREM Gerekli Java: {java} veya üstü (java -version ile kontrol et)\r\njava {line}\r\npause\r\n");
    let sh = format!("#!/bin/sh\ncd \"$(dirname \"$0\")\"\n# Gerekli Java: {java} veya üstü\nexec java {line}\n");
    let readme = format!(
        "Kazma server paketi: {name}\r\nSürüm: {kind} {game}\r\n\r\n1. Bu klasörü server makinesine kopyala ve zip'ten çıkar.\r\n2. Makinede Java {java} (ya da üstü) kurulu olmalı.\r\n3. Windows'ta start.bat, Linux'ta start.sh çalıştır.\r\n4. Server portu: {port} (TCP). Router'dan bu portu server makinesine yönlendir ya da güvenlik duvarında aç.\r\n5. Oyuncular makinenin genel IP'siyle ya da alan adıyla bağlanır.\r\n",
        name = spec.name,
        kind = spec.kind,
        game = spec.game,
        port = spec.port,
    );
    let out_dir = root.join("exports");
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out = out_dir.join(format!("{id}.zip"));
    let f = fs::File::create(&out).map_err(|e| e.to_string())?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, bytes: &[u8], exec: bool| -> R<()> {
        z.start_file(name, if exec { opt.unix_permissions(0o755) } else { opt }).map_err(|e| e.to_string())?;
        z.write_all(bytes).map_err(|e| e.to_string())
    };
    add("start.bat", bat.as_bytes(), false)?;
    add("start.sh", sh.as_bytes(), true)?;
    add("OKU-BENI.txt", readme.as_bytes(), false)?;
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let t = match e.file_type() { Ok(t) => t, Err(_) => continue };
                if t.is_symlink() {
                    continue;
                }
                if t.is_dir() {
                    if p.file_name().map_or(false, |n| n == "logs") {
                        continue;
                    }
                    walk(base, &p, out);
                } else if t.is_file() && p.file_name().map_or(true, |n| n != "installer.log") {
                    out.push(p);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(&data, &data, &mut files);
    for p in files {
        let rel = p.strip_prefix(&data).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        add(&rel, &fs::read(&p).map_err(|e| e.to_string())?, false)?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Server'a mod (Fabric/Forge/NeoForge) ya da eklenti (Paper) eklemek için klasör.
pub fn content_dir(root: &Path, spec: &ServerSpec) -> R<PathBuf> {
    let sub = match spec.kind.as_str() {
        "vanilla" => return Err("Vanilla server mod ya da eklenti desteklemez; Paper, Fabric, Forge veya NeoForge seç.".into()),
        "paper" => "plugins",
        _ => "mods",
    };
    let d = data_dir(root, &spec.id)?.join(sub);
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn modrinth_loader(spec: &ServerSpec) -> &str {
    spec.kind.as_str()
}

pub fn search_content(root: &Path, id: &str, query: &str, category: Option<&str>) -> R<Vec<crate::modrinth::Hit>> {
    let spec = read_spec(root, id)?;
    content_dir(root, &spec)?;
    crate::modrinth::search_live(query, &spec.game, Some(modrinth_loader(&spec)), category, crate::modrinth::Kind::Mod)
}

/// Server'da kurulu mod/eklenti dosyaları.
pub fn list_content(root: &Path, id: &str) -> R<Vec<String>> {
    let spec = read_spec(root, id)?;
    let dir = content_dir(root, &spec)?;
    let mut v: Vec<String> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_ascii_lowercase().ends_with(".jar")).collect();
    v.sort();
    Ok(v)
}

pub fn remove_content(root: &Path, id: &str, file: &str) -> R<()> {
    let spec = read_spec(root, id)?;
    let dir = content_dir(root, &spec)?;
    if file.is_empty() || file.contains(['/', '\\', ':']) || file == ".." || !file.to_ascii_lowercase().ends_with(".jar") {
        return Err("Geçersiz dosya adı.".into());
    }
    fs::remove_file(dir.join(file)).map_err(|e| format!("Silinemedi: {e}"))
}

/// Server'ın kullandığı (kurulmuş) Java'nın yolu.
pub fn java_path(root: &Path, id: &str) -> R<PathBuf> {
    let spec = read_spec(root, id)?;
    let major = if spec.java_major > 0 { spec.java_major } else { java_major_guess(&spec.game) };
    crate::install::find_java(&root.join(kazma_core::java::runtime_dir(major)), 3).ok_or_else(|| "Server'ın Java'sı bulunamadı; server'ı bir kez başlat.".to_string())
}

/// Bu bilgisayarın yerel ağ adresi (paket göndermeden, yönlendirme tablosundan).
pub fn lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(a) if a.is_private() => Some(a.to_string()),
        _ => None,
    }
}

/// Dış IP'yi öğrenir (yalnızca kullanıcı isteyince; api.ipify.org'a tek bir istek).
pub fn public_ip() -> R<String> {
    let t = get_text("https://api.ipify.org", 64)?;
    let t = t.trim();
    t.parse::<std::net::Ipv4Addr>().map(|a| a.to_string()).map_err(|_| "Dış IP öğrenilemedi.".to_string())
}

pub fn install_content(root: &Path, id: &str, project: &str, rep: &dyn Reporter) -> R<Vec<String>> {
    let spec = read_spec(root, id)?;
    let dir = content_dir(root, &spec)?;
    crate::modrinth::install_live(&dir, project, &spec.game, Some(modrinth_loader(&spec)), crate::modrinth::Kind::Mod, rep)
}

/// Oyuncuların modları otomatik eşitlemesi için paket kodu (server'ın mod klasöründen).
pub fn pack_code(root: &Path, id: &str, address: Option<String>, rep: &dyn Reporter) -> R<crate::serverkit::PackBuild> {
    let spec = read_spec(root, id)?;
    let addr = address.filter(|a| !a.trim().is_empty()).or_else(|| spec.public_address.clone()).ok_or("Önce bir adres seç ya da gir (dış adres, tünel adresi ya da yerel ağ adresi).")?;
    if !["fabric", "forge", "neoforge"].contains(&spec.kind.as_str()) {
        return Err("Paket kodu yalnızca Fabric, Forge ve NeoForge server'lar için (Paper eklentileri oyuncuya kurulmaz).".into());
    }
    let lv = if spec.loader_version.is_empty() { None } else { Some(spec.loader_version.clone()) };
    crate::app::make_pack(&data_dir(root, id)?.join("mods"), &spec.name, &addr, &spec.game, &spec.kind, lv, rep)
}

pub fn set_public_address(root: &Path, id: &str, address: &str) -> R<ServerSpec> {
    let mut spec = read_spec(root, id)?;
    let a = address.trim();
    spec.public_address = if a.is_empty() { None } else { Some(kazma_core::invite::parse(a).map_err(|e| e.to_string())?.address()) };
    write_spec(root, &spec)?;
    Ok(spec)
}

pub fn set_upnp(root: &Path, id: &str, on: bool) -> R<ServerSpec> {
    let mut spec = read_spec(root, id)?;
    spec.upnp = on;
    write_spec(root, &spec)?;
    Ok(spec)
}

// ---------- server.properties ----------

pub fn read_properties(data: &Path) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(t) = fs::read_to_string(data.join("server.properties")) {
        for l in t.lines() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

fn bool_ok(v: &str) -> bool {
    v == "true" || v == "false"
}
fn int_in(v: &str, lo: i64, hi: i64) -> bool {
    v.parse::<i64>().map_or(false, |n| (lo..=hi).contains(&n))
}

/// Yalnızca izin verilen anahtarlar ve doğrulanmış değerler yazılır.
pub fn property_valid(key: &str, v: &str) -> bool {
    if v.len() > 200 || v.chars().any(|c| c.is_control()) {
        return false;
    }
    match key {
        "motd" | "level-seed" => !v.contains('\\'),
        "level-name" => !v.is_empty() && v.len() <= 60 && !v.contains(['/', '\\', ':']) && v != "." && v != "..",
        "max-players" => int_in(v, 1, 1000),
        "view-distance" | "simulation-distance" => int_in(v, 3, 32),
        "spawn-protection" => int_in(v, 0, 1000),
        "difficulty" => ["peaceful", "easy", "normal", "hard"].contains(&v),
        "gamemode" => ["survival", "creative", "adventure", "spectator"].contains(&v),
        "online-mode" | "pvp" | "white-list" | "allow-flight" | "enable-command-block" | "hardcore" | "allow-nether" | "spawn-monsters" | "spawn-animals" => bool_ok(v),
        _ => false,
    }
}

pub fn set_properties(root: &Path, id: &str, updates: &BTreeMap<String, String>) -> R<()> {
    for (k, v) in updates {
        if !property_valid(k, v) {
            return Err(format!("Geçersiz ayar: {k}"));
        }
    }
    let data = data_dir(root, id)?;
    let path = data.join("server.properties");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut left: BTreeMap<&String, &String> = updates.iter().collect();
    let mut out: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if !l.trim_start().starts_with('#') && left.contains_key(&k.trim().to_string()) => {
                let key = k.trim().to_string();
                let v = left.remove(&key).unwrap();
                format!("{key}={v}")
            }
            _ => l.to_string(),
        })
        .collect();
    for (k, v) in left {
        out.push(format!("{k}={v}"));
    }
    write_atomic(&path, (out.join("\n") + "\n").as_bytes())
}

// ---------- süreç yönetimi ----------

pub trait Sink: Send + Sync + 'static {
    fn line(&self, id: &str, text: &str);
    /// "starting" | "running" | "stopping" | "stopped"
    fn state(&self, id: &str, state: &str);
}

struct Running {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    log: Mutex<VecDeque<String>>,
    state: Mutex<String>,
}

type RunMap = Arc<Mutex<HashMap<String, Arc<Running>>>>;

#[derive(Default)]
pub struct Manager {
    run: RunMap,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerView {
    #[serde(flatten)]
    pub spec: ServerSpec,
    pub state: String,
}

struct StageToSink<'a>(&'a str, &'a Arc<dyn Sink>);
impl Reporter for StageToSink<'_> {
    fn stage(&self, text: &str) {
        self.1.line(self.0, &format!("[Kazma] {text}"));
    }
    fn progress(&self, text: &str, done: usize, total: usize) {
        if done == total {
            self.1.line(self.0, &format!("[Kazma] {text}: {done}/{total}"));
        }
    }
}

impl Manager {
    pub fn new() -> Self {
        Manager::default()
    }

    pub fn state_of(&self, id: &str) -> String {
        self.run.lock().unwrap().get(id).map(|r| r.state.lock().unwrap().clone()).unwrap_or_else(|| "stopped".into())
    }

    pub fn views(&self, root: &Path) -> Vec<ServerView> {
        list_specs(root).into_iter().map(|s| ServerView { state: self.state_of(&s.id), spec: s }).collect()
    }

    pub fn log(&self, id: &str) -> Vec<String> {
        self.run.lock().unwrap().get(id).map(|r| r.log.lock().unwrap().iter().cloned().collect()).unwrap_or_default()
    }

    pub fn start(&self, root: &Path, id: &str, env: &Env, sink: Arc<dyn Sink>) -> R<()> {
        let spec = read_spec(root, id)?;
        if spec.mode != "local" {
            return Err("Bu server başka bir makine için; paketi oradan çalıştır.".into());
        }
        if self.run.lock().unwrap().contains_key(id) {
            return Err("Server zaten çalışıyor.".into());
        }
        let data = data_dir(root, id)?;
        sink.state(id, "starting");
        let prep = (|| -> R<Command> {
            let major = if spec.java_major > 0 { spec.java_major } else { java_major_guess(&spec.game) };
            let java = crate::install::ensure_java(root, major, env, &None, &StageToSink(id, &sink))?;
            let mut cmd = Command::new(java);
            cmd.args(launch_args(&data, &spec)?).current_dir(&data);
            Ok(cmd)
        })();
        match prep {
            Ok(cmd) => self.run_command(id, cmd, sink),
            Err(e) => {
                sink.state(id, "stopped");
                Err(e)
            }
        }
    }

    /// Hazırlanmış komutu çalıştırır, çıktıyı satır satır `sink`e iletir.
    pub fn run_command(&self, id: &str, mut cmd: Command, sink: Arc<dyn Sink>) -> R<()> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        no_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            sink.state(id, "stopped");
            format!("Server başlatılamadı: {e}")
        })?;
        let stdin = child.stdin.take().ok_or("stdin alınamadı")?;
        let out = child.stdout.take().ok_or("stdout alınamadı")?;
        let err = child.stderr.take().ok_or("stderr alınamadı")?;
        let r = Arc::new(Running { stdin: Mutex::new(stdin), child: Mutex::new(child), log: Mutex::new(VecDeque::new()), state: Mutex::new("starting".into()) });
        self.run.lock().unwrap().insert(id.to_string(), r.clone());
        sink.state(id, "starting");
        let mut readers = Vec::new();
        for stream in [Box::new(out) as Box<dyn std::io::Read + Send>, Box::new(err)] {
            let (r, sink, id) = (r.clone(), sink.clone(), id.to_string());
            readers.push(std::thread::spawn(move || {
                for l in BufReader::new(stream).split(b'\n').flatten() {
                    let line = strip_ansi(&String::from_utf8_lossy(&l));
                    {
                        let mut log = r.log.lock().unwrap();
                        if log.len() >= LOG_KEEP {
                            log.pop_front();
                        }
                        log.push_back(line.clone());
                    }
                    if line.contains("Done (") && line.contains("For help") {
                        let mut s = r.state.lock().unwrap();
                        if *s == "starting" {
                            *s = "running".into();
                            sink.state(&id, "running");
                        }
                    }
                    sink.line(&id, &line);
                }
            }));
        }
        let (sink2, id2, r2, map) = (sink, id.to_string(), r, self.run.clone());
        std::thread::spawn(move || {
            while r2.child.lock().unwrap().try_wait().ok().flatten().is_none() {
                std::thread::sleep(Duration::from_millis(200));
            }
            for h in readers {
                let _ = h.join();
            }
            map.lock().unwrap().remove(&id2);
            sink2.state(&id2, "stopped");
        });
        Ok(())
    }

    pub fn send(&self, id: &str, command: &str) -> R<()> {
        let c = command.trim();
        if c.is_empty() || c.len() > 500 || c.chars().any(|ch| ch.is_control()) {
            return Err("Geçersiz komut.".into());
        }
        let r = self.run.lock().unwrap().get(id).cloned().ok_or("Server çalışmıyor.")?;
        let mut w = r.stdin.lock().unwrap();
        writeln!(w, "{c}").and_then(|_| w.flush()).map_err(|e| format!("Komut gönderilemedi: {e}"))
    }

    /// Düzgünce kapatır ("stop"); `grace` içinde kapanmazsa zorla sonlandırır.
    pub fn stop(&self, id: &str, grace: Duration, sink: &Arc<dyn Sink>) -> R<()> {
        let r = self.run.lock().unwrap().get(id).cloned().ok_or("Server çalışmıyor.")?;
        *r.state.lock().unwrap() = "stopping".into();
        sink.state(id, "stopping");
        let _ = self.send(id, "stop");
        let t0 = Instant::now();
        while t0.elapsed() < grace {
            if r.child.lock().unwrap().try_wait().ok().flatten().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = r.child.lock().unwrap().kill();
        Ok(())
    }

    pub fn wait_stopped(&self, id: &str, max: Duration) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < max {
            if !self.run.lock().unwrap().contains_key(id) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        false
    }

    pub fn stop_all(&self, sink: &Arc<dyn Sink>) {
        let ids: Vec<String> = self.run.lock().unwrap().keys().cloned().collect();
        for id in &ids {
            let _ = self.stop(id, Duration::from_secs(15), sink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-sh-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[derive(Default)]
    struct Rec {
        lines: Mutex<Vec<String>>,
        states: Mutex<Vec<String>>,
    }
    impl Sink for Rec {
        fn line(&self, _: &str, t: &str) {
            self.lines.lock().unwrap().push(t.to_string());
        }
        fn state(&self, _: &str, s: &str) {
            self.states.lock().unwrap().push(s.to_string());
        }
    }

    fn spec(kind: &str) -> ServerSpec {
        ServerSpec { id: "s1".into(), name: "Deneme".into(), kind: kind.into(), game: "1.21.1".into(), loader_version: String::new(), port: 25565, ram: 2048, mode: "local".into(), upnp: false, created: 1, java_major: 21, public_address: None }
    }

    #[test]
    fn ids_and_names_are_safe() {
        let r = tmp("ids");
        for bad in ["", "../x", "A", "a/b", "a b", &"x".repeat(65)] {
            assert!(server_dir(&r, bad).is_err(), "{bad}");
        }
        assert!(server_dir(&r, "kule-2").is_ok());
        assert_eq!(slug("Dostlar SMP!!"), "dostlar-smp");
        assert_eq!(slug("???"), "server");
        assert!(clean_name("  ").is_err() && clean_name("a\nb").is_err());
        let req = |eula: bool, port: u16, kind: &str| CreateRequest { name: "x".into(), kind: kind.into(), game: "1.21.1".into(), port: Some(port), ram: None, mode: "local".into(), upnp: false, eula, cracked: false };
        let env = Env::current();
        assert!(create(&r, &req(false, 25565, "vanilla"), &env, &NullReporter).unwrap_err().contains("EULA"));
        assert!(create(&r, &req(true, 80, "vanilla"), &env, &NullReporter).unwrap_err().contains("Port"));
        assert!(create(&r, &req(true, 25565, "bukkit"), &env, &NullReporter).unwrap_err().contains("tür"));
        assert!(!servers_root(&r).exists() || list_specs(&r).is_empty(), "hatalı istek dosya bırakmaz");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mHata\u{1b}[0m: x\r"), "Hata: x");
        assert_eq!(strip_ansi("düz"), "düz");
    }

    #[test]
    fn content_files_listed_and_removed_safely() {
        let r = tmp("cont");
        write_spec(&r, &spec("fabric")).unwrap();
        let d = content_dir(&r, &spec("fabric")).unwrap();
        fs::write(d.join("a.jar"), b"x").unwrap();
        fs::write(d.join("b.txt"), b"x").unwrap();
        assert_eq!(list_content(&r, "s1").unwrap(), ["a.jar"]);
        for bad in ["../a.jar", "a/b.jar", "b.txt", "", ".."] {
            assert!(remove_content(&r, "s1", bad).is_err(), "{bad}");
        }
        remove_content(&r, "s1", "a.jar").unwrap();
        assert!(list_content(&r, "s1").unwrap().is_empty());
        assert!(content_dir(&r, &spec("vanilla")).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn java_majors() {
        assert_eq!([java_major_guess("1.21.1"), java_major_guess("1.20.4"), java_major_guess("1.20.5"), java_major_guess("1.18.2"), java_major_guess("1.17.1"), java_major_guess("1.12.2")], [21, 17, 21, 17, 16, 8]);
    }

    #[test]
    fn paper_picks_highest_stable_and_checks_host() {
        let j = r#"[{"id":3,"channel":"ALPHA","downloads":{"server:default":{"url":"https://fill-data.papermc.io/a.jar"}}},
          {"id":2,"channel":"STABLE","downloads":{"server:default":{"url":"https://fill-data.papermc.io/b.jar","checksums":{"sha256":"ab"}}}},
          {"id":1,"channel":"STABLE","downloads":{"server:default":{"url":"https://fill-data.papermc.io/c.jar"}}}]"#;
        let s = paper_pick(j).unwrap();
        assert_eq!((s.url.as_str(), s.sha256.as_deref()), ("https://fill-data.papermc.io/b.jar", Some("ab")));
        assert!(paper_pick(&j.replace("fill-data.papermc.io", "evil.example.com")).is_err());
        assert!(paper_pick(r#"[{"id":1,"channel":"ALPHA","downloads":{}}]"#).is_err());
        assert!(paper_pick("{").is_err());
    }

    #[test]
    fn fabric_urls() {
        assert_eq!(fabric_server_url("1.21.1", "0.16.9", "1.0.1").unwrap(), "https://meta.fabricmc.net/v2/versions/loader/1.21.1/0.16.9/1.0.1/server/jar");
        assert!(fabric_server_url("1.21/..", "0.16.9", "1").is_err());
        assert_eq!(fabric_installer_pick(r#"[{"version":"1.1.0","stable":false},{"version":"1.0.1","stable":true}]"#).unwrap(), "1.0.1");
    }

    #[test]
    fn launch_args_for_each_kind() {
        let d = tmp("args");
        assert_eq!(launch_args(&d, &spec("paper")).unwrap(), ["-Xms512M", "-Xmx2048M", "-jar", "server.jar", "nogui"]);
        assert!(launch_args(&d, &spec("neoforge")).is_err(), "kurulum yoksa hata");
        let name = if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" };
        let ad = d.join("libraries/net/neoforged/neoforge/21.1.5");
        fs::create_dir_all(&ad).unwrap();
        fs::write(ad.join(name), "x").unwrap();
        let a = launch_args(&d, &spec("neoforge")).unwrap();
        assert_eq!(a[0], "@user_jvm_args.txt");
        assert_eq!(a[1], format!("@libraries/net/neoforged/neoforge/21.1.5/{name}"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn properties_are_validated_and_preserved() {
        let r = tmp("props");
        let d = data_dir(&r, "s1").unwrap();
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("server.properties"), "#kom\nmotd=eski\nserver-port=25565\nlevel-name=world\n").unwrap();
        let mut u = BTreeMap::new();
        u.insert("motd".to_string(), "Merhaba".to_string());
        u.insert("difficulty".to_string(), "hard".to_string());
        set_properties(&r, "s1", &u).unwrap();
        let p = read_properties(&d);
        assert_eq!((p["motd"].as_str(), p["difficulty"].as_str(), p["server-port"].as_str()), ("Merhaba", "hard", "25565"));
        assert!(fs::read_to_string(d.join("server.properties")).unwrap().starts_with("#kom"));
        for (k, v) in [("server-port", "1"), ("rcon.password", "x"), ("difficulty", "cok"), ("motd", "a\nenable-rcon=true"), ("level-name", "../x"), ("max-players", "0"), ("pvp", "yes")] {
            let mut b = BTreeMap::new();
            b.insert(k.to_string(), v.to_string());
            assert!(set_properties(&r, "s1", &b).is_err(), "{k}={v}");
        }
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn export_zip_has_scripts_and_data() {
        let r = tmp("exp");
        let s = spec("paper");
        fs::create_dir_all(data_dir(&r, "s1").unwrap().join("logs")).unwrap();
        write_spec(&r, &s).unwrap();
        let d = data_dir(&r, "s1").unwrap();
        fs::write(d.join("server.jar"), b"jar").unwrap();
        fs::write(d.join("eula.txt"), b"eula=true").unwrap();
        fs::write(d.join("logs/latest.log"), b"x").unwrap();
        let z = export_package(&r, "s1").unwrap();
        let mut a = zip::ZipArchive::new(fs::File::open(&z).unwrap()).unwrap();
        let names: Vec<String> = (0..a.len()).map(|i| a.by_index(i).unwrap().name().to_string()).collect();
        for n in ["start.bat", "start.sh", "OKU-BENI.txt", "server.jar", "eula.txt"] {
            assert!(names.contains(&n.to_string()), "{n}: {names:?}");
        }
        assert!(!names.iter().any(|n| n.starts_with("logs")));
        let mut bat = String::new();
        std::io::Read::read_to_string(&mut a.by_name("start.bat").unwrap(), &mut bat).unwrap();
        assert!(bat.contains("java -Xms512M -Xmx2048M -jar server.jar nogui"));
        let _ = fs::remove_dir_all(&r);
    }

    #[cfg(unix)]
    #[test]
    fn manager_streams_console_and_stops() {
        let m = Manager::new();
        let rec = Arc::new(Rec::default());
        let sink: Arc<dyn Sink> = rec.clone();
        let mut c = Command::new("sh");
        c.arg("-c").arg("echo 'Done (1.2s)! For help, type \"help\"'; while read l; do echo \"got:$l\"; [ \"$l\" = stop ] && exit 0; done");
        m.run_command("t1", c, sink.clone()).unwrap();
        assert!(m.run.lock().unwrap().contains_key("t1"));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(m.state_of("t1"), "running");
        m.send("t1", "say merhaba").unwrap();
        assert!(m.send("t1", "a\nb").is_err() && m.send("yok", "x").is_err());
        std::thread::sleep(Duration::from_millis(300));
        assert!(m.log("t1").iter().any(|l| l == "got:say merhaba"));
        m.stop("t1", Duration::from_secs(5), &sink).unwrap();
        assert!(m.wait_stopped("t1", Duration::from_secs(5)));
        assert_eq!(m.state_of("t1"), "stopped");
        let st = rec.states.lock().unwrap().clone();
        assert!(st.contains(&"running".to_string()) && st.contains(&"stopping".to_string()) && st.last().map(String::as_str) == Some("stopped"), "{st:?}");
        assert!(rec.lines.lock().unwrap().iter().any(|l| l == "got:stop"));
    }

    #[cfg(unix)]
    #[test]
    fn stuck_server_is_killed_after_grace() {
        let m = Manager::new();
        let sink: Arc<dyn Sink> = Arc::new(Rec::default());
        let mut c = Command::new("sh");
        c.arg("-c").arg("while true; do sleep 1; done");
        m.run_command("t2", c, sink.clone()).unwrap();
        m.stop("t2", Duration::from_millis(600), &sink).unwrap();
        assert!(m.wait_stopped("t2", Duration::from_secs(5)));
    }
}
