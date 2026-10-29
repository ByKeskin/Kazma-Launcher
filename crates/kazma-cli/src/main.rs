//! `kazma-cli launch 1.21.1 Mustafa [--fabric] [--ram 4096] [--join sunucu[:port]] [--java YOL] [--root KLASÖR]`
//!
//! Sürüm dosyalarını, kütüphaneleri, asset'leri ve (Windows'ta) Java'yı indirir, doğrular ve oyunu
//! çevrimdışı hesapla başlatır. Mantığın tamamı `kazma-core`'dadır; burada yalnızca ağ ve dosya işi var.

use kazma_core::launch::{Account, Dirs, LaunchOptions};
use kazma_core::plan::{asset_items, build_plan, game_url_trusted, DownloadItem, NativeJar, Plan};
use kazma_core::version::{self, Env, Os};
use kazma_core::{invite, java, mojang};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type R<T> = Result<T, String>;

// ---------- komut satırı ----------

#[derive(Debug, PartialEq)]
struct Cli {
    version: String,
    player: String,
    fabric: bool,
    ram: u32,
    join: Option<(String, u16)>,
    java: Option<PathBuf>,
    root: PathBuf,
}

const USAGE: &str = "Kullanım: kazma-cli launch <sürüm> <oyuncu_adı> [--fabric] [--ram MB] [--join sunucu[:port]] [--java YOL] [--root KLASÖR]\nÖrnek:    kazma-cli launch 1.21.1 Mustafa --fabric --ram 4096";

fn default_root() -> PathBuf {
    if let Ok(a) = std::env::var("APPDATA") {
        return PathBuf::from(a).join(".kazma");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".kazma")
}

fn parse_cli(args: &[String]) -> R<Cli> {
    if args.first().map(String::as_str) != Some("launch") || args.len() < 3 {
        return Err(USAGE.into());
    }
    let mut cli = Cli { version: args[1].clone(), player: args[2].clone(), fabric: false, ram: 4096, join: None, java: None, root: default_root() };
    let mut i = 3;
    while i < args.len() {
        let need = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} için değer gerekli\n{USAGE}", args[i]));
        match args[i].as_str() {
            "--fabric" => cli.fabric = true,
            "--ram" => {
                cli.ram = need(i)?.parse().map_err(|_| "--ram sayı olmalı (MB)".to_string())?;
                if !(1024..=65536).contains(&cli.ram) {
                    return Err("--ram 1024 ile 65536 MB arasında olmalı".into());
                }
                i += 1;
            }
            "--join" => {
                let inv = invite::parse(&need(i)?).map_err(|e| e.to_string())?;
                cli.join = Some((inv.host, inv.port));
                i += 1;
            }
            "--java" => {
                cli.java = Some(PathBuf::from(need(i)?));
                i += 1;
            }
            "--root" => {
                cli.root = PathBuf::from(need(i)?);
                i += 1;
            }
            other => return Err(format!("bilinmeyen seçenek: {other}\n{USAGE}")),
        }
        i += 1;
    }
    Ok(cli)
}

// ---------- ağ ----------

fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .user_agent("kazma-cli/0.1")
            .build()
    })
}

fn get_bytes(url: &str, max: u64) -> R<Vec<u8>> {
    let mut last = String::new();
    for attempt in 0..3 {
        match agent().get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                match resp.into_reader().take(max + 1).read_to_end(&mut buf) {
                    Ok(_) if buf.len() as u64 > max => return Err(format!("{url}: yanıt beklenenden büyük")),
                    Ok(_) => return Ok(buf),
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(500 * (attempt + 1)));
    }
    Err(format!("{url}: indirilemedi ({last})"))
}

fn get_text(url: &str) -> R<String> {
    String::from_utf8(get_bytes(url, 20 << 20)?).map_err(|_| format!("{url}: geçerli UTF-8 değil"))
}

fn sha1_hex(b: &[u8]) -> String {
    hex::encode(Sha1::digest(b))
}

// ---------- dosya indirme ----------

fn file_ok(path: &Path, it: &DownloadItem) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    if let Some(sz) = it.size {
        return meta.len() == sz;
    }
    match (&it.sha1, fs::read(path)) {
        (Some(h), Ok(b)) => sha1_hex(&b).eq_ignore_ascii_case(h),
        _ => true,
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> R<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let tmp = path.with_extension("part");
    fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn download_item(root: &Path, it: &DownloadItem) -> R<()> {
    let dest = root.join(&it.rel_path);
    if file_ok(&dest, it) {
        return Ok(());
    }
    let data = get_bytes(&it.url, 400 << 20)?;
    if let Some(h) = &it.sha1 {
        if !sha1_hex(&data).eq_ignore_ascii_case(h) {
            return Err(format!("{}: SHA-1 uyuşmuyor, dosya atıldı", it.rel_path));
        }
    }
    write_atomic(&dest, &data)
}

fn download_all(root: &Path, items: Vec<DownloadItem>, label: &str) -> R<()> {
    let total = items.len();
    if total == 0 {
        return Ok(());
    }
    println!("{label}: {total} dosya kontrol ediliyor");
    let queue = Mutex::new(VecDeque::from(items));
    let errors = Mutex::new(Vec::<String>::new());
    let done = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..12.min(total) {
            s.spawn(|| loop {
                let next = queue.lock().unwrap().pop_front();
                let Some(it) = next else { break };
                if let Err(e) = download_item(root, &it) {
                    errors.lock().unwrap().push(e);
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 250 == 0 || n == total {
                    println!("  {label}: {n}/{total}");
                }
            });
        }
    });
    let errs = errors.into_inner().unwrap();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("{} dosya indirilemedi, ilki: {}", errs.len(), errs[0]))
    }
}

// ---------- sürüm dosyaları ----------

fn fetch_version_json(root: &Path, id: &str, manifest: &mojang::VersionManifest) -> R<version::VersionJson> {
    let v = manifest.find(id).ok_or_else(|| format!("{id} sürümü Mojang listesinde yok"))?;
    if !game_url_trusted(&v.url) {
        return Err(format!("güvenilmeyen adres: {}", v.url));
    }
    let path = root.join("versions").join(id).join(format!("{id}.json"));
    let text = match fs::read_to_string(&path) {
        Ok(t) if sha1_hex(t.as_bytes()).eq_ignore_ascii_case(&v.sha1) => t,
        _ => {
            let bytes = get_bytes(&v.url, 20 << 20)?;
            if !sha1_hex(&bytes).eq_ignore_ascii_case(&v.sha1) {
                return Err(format!("{id}.json SHA-1 uyuşmuyor"));
            }
            write_atomic(&path, &bytes)?;
            String::from_utf8(bytes).map_err(|_| "sürüm dosyası UTF-8 değil".to_string())?
        }
    };
    version::parse(&text).map_err(|e| e.to_string())
}

fn fetch_fabric(root: &Path, game: &str) -> R<version::VersionJson> {
    let list = get_text(&format!("https://meta.fabricmc.net/v2/versions/loader/{game}"))?;
    let v: serde_json::Value = serde_json::from_str(&list).map_err(|e| e.to_string())?;
    let loader = v
        .as_array()
        .and_then(|a| a.iter().find(|e| e.pointer("/loader/stable").and_then(|s| s.as_bool()) == Some(true)))
        .and_then(|e| e.pointer("/loader/version").and_then(|s| s.as_str()))
        .ok_or_else(|| format!("{game} için kararlı Fabric sürümü bulunamadı"))?
        .to_string();
    let url = mojang::fabric_profile_url(game, &loader).map_err(|e| e.to_string())?;
    let text = get_text(&url)?;
    let parsed = version::parse(&text).map_err(|e| e.to_string())?;
    write_atomic(&root.join("versions").join(&parsed.id).join(format!("{}.json", parsed.id)), text.as_bytes())?;
    println!("Fabric yükleyici {loader}");
    Ok(parsed)
}

// ---------- Java ----------

fn find_java(dir: &Path, depth: u32) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let direct = dir.join("bin").join(exe);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_java(&e.path(), depth - 1))
}

fn unzip(zip_bytes: &[u8], dest: &Path) -> R<()> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue }; // yol kaçışı olan girişler atlanır
        let out = dest.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = out.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut w = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn ensure_java(root: &Path, major: u32, env: &Env, custom: &Option<PathBuf>) -> R<PathBuf> {
    if let Some(p) = custom {
        return Ok(p.clone());
    }
    let dir = root.join(java::runtime_dir(major));
    if let Some(j) = find_java(&dir, 3) {
        return Ok(j);
    }
    if env.os != Os::Windows {
        return Err(format!("Java {major} bulunamadı. Bu sistemde otomatik kurulum yok; --java ile yolunu ver."));
    }
    println!("Java {major} indiriliyor (Eclipse Temurin)");
    let pkg = java::parse_adoptium(&get_text(&java::adoptium_query_url(major, env))?).map_err(|e| e.to_string())?;
    let bytes = get_bytes(&pkg.link, 400 << 20)?;
    if !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&pkg.sha256) {
        return Err("Java paketinin SHA-256 özeti uyuşmuyor, kurulmadı".into());
    }
    unzip(&bytes, &dir)?;
    find_java(&dir, 3).ok_or_else(|| "Java kuruldu ama java.exe bulunamadı".to_string())
}

// ---------- yerel kütüphaneler ----------

fn extract_natives(root: &Path, natives: &[NativeJar], dest: &Path) -> R<()> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for n in natives {
        let data = fs::read(root.join(&n.rel_path)).map_err(|e| format!("{}: {e}", n.rel_path))?;
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| e.to_string())?;
            if f.is_dir() || n.exclude.iter().any(|x| f.name().starts_with(x.as_str())) {
                continue;
            }
            // Yalnızca dosya adı kullanılır: arşiv içindeki yol hedefin dışına çıkamaz.
            let Some(name) = Path::new(f.name()).file_name().map(|s| s.to_owned()) else { continue };
            let out = dest.join(name);
            if out.is_file() {
                continue;
            }
            let mut w = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ---------- ana akış ----------

fn run(cli: Cli) -> R<i32> {
    let env = Env::current();
    let root = cli.root.clone();
    fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;

    println!("Mojang sürüm listesi alınıyor");
    let manifest = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL)?).map_err(|e| e.to_string())?;
    let base = fetch_version_json(&root, &cli.version, &manifest)?;
    let resolved = if cli.fabric {
        let fab = fetch_fabric(&root, &cli.version)?;
        version::resolve(fab, Some(base))
    } else {
        version::resolve(base, None)
    }
    .map_err(|e| e.to_string())?;

    let plan: Plan = build_plan(&resolved, &env).map_err(|e| e.to_string())?;
    download_all(&root, plan.downloads.clone(), "kütüphaneler")?;

    let index_text = fs::read_to_string(root.join(&plan.asset_index_path)).map_err(|e| e.to_string())?;
    download_all(&root, asset_items(&index_text).map_err(|e| e.to_string())?, "asset'ler")?;

    let java_exe = ensure_java(&root, resolved.java_major, &env, &cli.java)?;
    let natives_dir = root.join("natives").join(&resolved.id);
    extract_natives(&root, &plan.natives, &natives_dir)?;

    let game_dir = root.join("instances").join(format!("{}-{}", cli.version, if cli.fabric { "fabric" } else { "vanilla" }));
    fs::create_dir_all(&game_dir).map_err(|e| e.to_string())?;
    let dirs = Dirs { root: root.clone(), game_dir: game_dir.clone(), natives_dir };
    let acct = Account::offline(&cli.player).map_err(|e| e.to_string())?;
    let opts = LaunchOptions { max_mb: cli.ram, join_server: cli.join.clone(), ..Default::default() };
    let args = kazma_core::launch::build_args(&resolved, &plan, &env, &dirs, &acct, &opts).map_err(|e| e.to_string())?;

    println!("Oyun başlatılıyor: {} (kimlik {})", acct.name, acct.uuid);
    let status = Command::new(&java_exe).args(&args).current_dir(&game_dir).status().map_err(|e| format!("{}: {e}", java_exe.display()))?;
    Ok(status.code().unwrap_or(-1))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli(&args).and_then(run) {
        Ok(0) => {}
        Ok(code) => {
            eprintln!("Oyun {code} koduyla kapandı");
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("Hata: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_full_command() {
        let c = parse_cli(&a("launch 1.21.1 Mustafa --fabric --ram 6144 --join play.example.com:25570 --root C:\\k")).unwrap();
        assert_eq!(c.version, "1.21.1");
        assert!(c.fabric);
        assert_eq!(c.ram, 6144);
        assert_eq!(c.join, Some(("play.example.com".into(), 25570)));
        assert_eq!(c.root, PathBuf::from("C:\\k"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_cli(&a("launch 1.21.1")).is_err());
        assert!(parse_cli(&a("play 1.21.1 X")).is_err());
        assert!(parse_cli(&a("launch 1.21.1 X --ram abc")).is_err());
        assert!(parse_cli(&a("launch 1.21.1 X --ram 10")).is_err());
        assert!(parse_cli(&a("launch 1.21.1 X --join -bad")).is_err());
        assert!(parse_cli(&a("launch 1.21.1 X --nope")).is_err());
        assert!(parse_cli(&a("launch 1.21.1 X --ram")).is_err());
    }

    #[test]
    fn unzip_blocks_path_traversal() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("ok/a.txt", o).unwrap();
            w.write_all(b"a").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("kazma-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        unzip(&buf, &dir).unwrap();
        assert!(dir.join("ok/a.txt").is_file());
        assert!(!dir.parent().unwrap().join("evil.txt").exists(), "yol kaçışı engellenmeli");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn natives_extract_flat_and_skip_meta_inf() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("kazma-nat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("libraries")).unwrap();
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("windows/x64/org/lwjgl/lwjgl.dll", o).unwrap();
            w.write_all(b"dll").unwrap();
            w.start_file("META-INF/MANIFEST.MF", o).unwrap();
            w.write_all(b"m").unwrap();
            w.finish().unwrap();
        }
        fs::write(root.join("libraries/n.jar"), &buf).unwrap();
        let dest = root.join("natives");
        extract_natives(&root, &[NativeJar { rel_path: "libraries/n.jar".into(), exclude: vec!["META-INF/".into()] }], &dest).unwrap();
        assert!(dest.join("lwjgl.dll").is_file());
        assert!(!dest.join("MANIFEST.MF").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
