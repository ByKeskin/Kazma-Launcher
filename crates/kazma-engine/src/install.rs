//! Bir instance'ın oyun dosyalarını kurar ve oyunu başlatır. Sürüm ve yükleyici bir `InstanceSpec`'ten gelir.

use crate::net::*;
use crate::report::Reporter;
use crate::loaders;
use kazma_core::instance::{InstanceSpec, LoaderSpec};
use kazma_core::launch::{build_args, Account, Dirs, LaunchOptions};
use kazma_core::plan::{asset_items, build_plan, game_url_trusted, NativeJar, Plan};
use kazma_core::version::{self, Env, Os, Resolved, VersionJson};
use kazma_core::{java, mojang};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Installed {
    pub resolved: Resolved,
    pub plan: Plan,
}

fn fetch_version_json(root: &Path, id: &str, manifest: &mojang::VersionManifest) -> R<VersionJson> {
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

/// Belirli bir Fabric/Quilt yükleyici sürümünün profili değişmez; bir kez indirilince diskten okunur.
fn profile_json(root: &Path, spec: &InstanceSpec) -> R<VersionJson> {
    let id = spec.version_id();
    let path = root.join("versions").join(&id).join(format!("{id}.json"));
    if let Ok(t) = fs::read_to_string(&path) {
        if let Ok(v) = version::parse(&t) {
            if v.id == id {
                return Ok(v);
            }
        }
    }
    let url = match &spec.loader {
        LoaderSpec::Fabric { version } => mojang::fabric_profile_url(&spec.game, version),
        LoaderSpec::Quilt { version } => mojang::quilt_profile_url(&spec.game, version),
        _ => return Err("bu yükleyici profil dosyasıyla kurulmaz".into()),
    }
    .map_err(|e| e.to_string())?;
    let text = get_text(&url, 5 << 20).map_err(|e| format!("{e}\nBu Minecraft sürümü için {} {} yok olabilir", spec.loader.kind(), spec.loader.version()))?;
    let parsed = version::parse(&text).map_err(|e| e.to_string())?;
    if parsed.id != id {
        return Err(format!("yükleyici profili beklenen kimlikte değil ({} ≠ {id})", parsed.id));
    }
    write_atomic(&path, text.as_bytes())?;
    Ok(parsed)
}

/// Seçilen yükleyicinin, verilen Minecraft sürümü için en uygun sürümü (ağdan).
pub fn latest_loader(kind: &str, game: &str) -> R<String> {
    loaders::latest_version(kind, game, &|u| get_text(u, 8 << 20))
}

pub fn list_versions(snapshots: bool) -> R<Vec<String>> {
    let m = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL, 20 << 20)?).map_err(|e| e.to_string())?;
    Ok(m.versions(snapshots).iter().map(|v| v.id.clone()).collect())
}

fn sha1_from_text(t: &str) -> Option<String> {
    let w = t.split_whitespace().next()?;
    (w.len() == 40 && w.bytes().all(|b| b.is_ascii_hexdigit())).then(|| w.to_ascii_lowercase())
}

/// Forge ve NeoForge: resmi kurucuyu indirir (SHA-1 ile doğrular) ve `--installClient` ile çalıştırır.
/// Kurucu `versions/<kimlik>/<kimlik>.json` dosyasını ve kütüphaneleri üretir; sonra normal akış devam eder.
fn run_loader_installer(root: &Path, spec: &InstanceSpec, base: &VersionJson, env: &Env, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<String> {
    let expected = spec.version_id();
    let json_of = |id: &str| root.join("versions").join(id).join(format!("{id}.json"));
    if json_of(&expected).is_file() {
        return Ok(expected);
    }
    let kind = spec.loader.kind();
    let (jar_url, sha_url) = loaders::installer_urls(kind, &spec.game, spec.loader.version())?;
    rep.stage(&format!("{kind} kurucusu indiriliyor"));
    let jar = get_bytes(&jar_url, 200 << 20)?;
    let want = sha1_from_text(&get_text(&sha_url, 4096)?).ok_or("kurucunun SHA-1 özeti okunamadı")?;
    if !sha1_hex(&jar).eq_ignore_ascii_case(&want) {
        return Err("kurucunun SHA-1 özeti uyuşmuyor, çalıştırılmadı".into());
    }
    let dir = root.join("installers");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let jar_path = dir.join(format!("{kind}-{}-{}.jar", spec.game, spec.loader.version()));
    fs::write(&jar_path, &jar).map_err(|e| e.to_string())?;
    let profiles = root.join("launcher_profiles.json");
    if !profiles.is_file() {
        write_atomic(&profiles, br#"{"profiles":{},"settings":{},"version":3}"#)?;
    }
    let java_exe = ensure_java(root, base.java_version.as_ref().map(|j| j.major_version).unwrap_or(17), env, java, rep)?;
    let before: std::collections::HashSet<String> = fs::read_dir(root.join("versions")).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    rep.stage(&format!("{kind} kuruluyor (birkaç dakika sürebilir)"));
    let log = dir.join("last.log");
    let out = fs::File::create(&log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&java_exe);
    cmd.arg("-jar").arg(&jar_path).arg("--installClient").arg(root).current_dir(root).stdout(out).stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd.status().map_err(|e| format!("{}: {e}", java_exe.display()))?;
    if json_of(&expected).is_file() {
        return Ok(expected);
    }
    // Kurucu beklenenden farklı bir kimlikle kurmuş olabilir: yeni oluşan sürüm klasörünü bul.
    let after: Vec<String> = fs::read_dir(root.join("versions")).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    if let Some(id) = after.into_iter().find(|n| !before.contains(n) && json_of(n).is_file() && n.contains(spec.loader.version())) {
        return Ok(id);
    }
    Err(format!("{kind} kurulamadı (çıkış kodu {}). Günlük: {}", status.code().unwrap_or(-1), log.display()))
}

pub fn ensure_installed(root: &Path, spec: &InstanceSpec, env: &Env, java: &Option<PathBuf>, rep: &dyn Reporter) -> R<Installed> {
    rep.stage("Mojang sürüm listesi alınıyor");
    let manifest = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL, 20 << 20)?).map_err(|e| e.to_string())?;
    let base = fetch_version_json(root, &spec.game, &manifest)?;
    let resolved = match &spec.loader {
        LoaderSpec::Vanilla => version::resolve(base, None),
        LoaderSpec::Fabric { .. } | LoaderSpec::Quilt { .. } => version::resolve(profile_json(root, spec)?, Some(base)),
        LoaderSpec::Forge { .. } | LoaderSpec::NeoForge { .. } => {
            let id = run_loader_installer(root, spec, &base, env, java, rep)?;
            let t = fs::read_to_string(root.join("versions").join(&id).join(format!("{id}.json"))).map_err(|e| e.to_string())?;
            version::resolve(version::parse(&t).map_err(|e| e.to_string())?, Some(base))
        }
    }
    .map_err(|e| e.to_string())?;
    let plan = build_plan(&resolved, env).map_err(|e| e.to_string())?;
    download_all(root, plan.downloads.clone(), "kütüphaneler", rep)?;
    let index_text = fs::read_to_string(root.join(&plan.asset_index_path)).map_err(|e| e.to_string())?;
    download_all(root, asset_items(&index_text).map_err(|e| e.to_string())?, "asset'ler", rep)?;
    Ok(Installed { resolved, plan })
}

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

fn ensure_java(root: &Path, major: u32, env: &Env, custom: &Option<PathBuf>, rep: &dyn Reporter) -> R<PathBuf> {
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
    rep.stage(&format!("Java {major} indiriliyor (Eclipse Temurin)"));
    let pkg = java::parse_adoptium(&get_text(&java::adoptium_query_url(major, env), 5 << 20)?).map_err(|e| e.to_string())?;
    let bytes = get_bytes(&pkg.link, 400 << 20)?;
    if !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&pkg.sha256) {
        return Err("Java paketinin SHA-256 özeti uyuşmuyor, kurulmadı".into());
    }
    unzip(&bytes, &dir)?;
    find_java(&dir, 3).ok_or_else(|| "Java kuruldu ama java.exe bulunamadı".to_string())
}

pub fn extract_natives(root: &Path, natives: &[NativeJar], dest: &Path) -> R<()> {
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

pub struct GameExit {
    pub code: i32,
    /// Oyunun çıktısının yazıldığı günlük dosyası.
    pub log: PathBuf,
}

/// Oyunu başlatır ve kapanana kadar bekler. Çıktı `kazma-last.log` dosyasına yazılır;
/// Windows'ta konsol penceresi açılmaz.
pub fn launch_game(
    root: &Path,
    spec: &InstanceSpec,
    inst: &Installed,
    env: &Env,
    acct: &Account,
    opts: &LaunchOptions,
    java_override: &Option<PathBuf>,
    rep: &dyn Reporter,
) -> R<GameExit> {
    let java_exe = ensure_java(root, inst.resolved.java_major, env, java_override, rep)?;
    rep.stage("Yerel kütüphaneler hazırlanıyor");
    let natives_dir = root.join("natives").join(&inst.resolved.id);
    extract_natives(root, &inst.plan.natives, &natives_dir)?;
    let game_dir = root.join(spec.dir_rel());
    fs::create_dir_all(&game_dir).map_err(|e| e.to_string())?;
    let dirs = Dirs { root: root.to_path_buf(), game_dir: game_dir.clone(), natives_dir };
    let args = build_args(&inst.resolved, &inst.plan, env, &dirs, acct, opts).map_err(|e| e.to_string())?;
    let log = game_dir.join("kazma-last.log");
    let out = fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    rep.stage(&format!("Oyun açılıyor: {}", acct.name));
    let mut cmd = Command::new(&java_exe);
    cmd.args(&args).current_dir(&game_dir).stdout(out).stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let status = cmd.status().map_err(|e| format!("{}: {e}", java_exe.display()))?;
    Ok(GameExit { code: status.code().unwrap_or(-1), log })
}

#[cfg(test)]
mod tests {
    use super::*;

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
