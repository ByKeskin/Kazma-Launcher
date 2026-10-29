//! Bir instance'ın oyun dosyalarını kurar ve oyunu başlatır. Sürüm ve yükleyici bir `InstanceSpec`'ten gelir.

use crate::net::*;
use crate::report::Reporter;
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

/// Belirli bir Fabric yükleyici sürümü değişmez; bir kez indirilince diskten okunur.
fn fabric_profile(root: &Path, spec: &InstanceSpec, loader: &str) -> R<VersionJson> {
    let id = spec.version_id();
    let path = root.join("versions").join(&id).join(format!("{id}.json"));
    if let Ok(t) = fs::read_to_string(&path) {
        if let Ok(v) = version::parse(&t) {
            if v.id == id {
                return Ok(v);
            }
        }
    }
    let url = mojang::fabric_profile_url(&spec.game, loader).map_err(|e| e.to_string())?;
    let text = get_text(&url, 5 << 20).map_err(|e| format!("{e}\nBu Minecraft sürümü için Fabric {loader} yok olabilir"))?;
    let parsed = version::parse(&text).map_err(|e| e.to_string())?;
    if parsed.id != id {
        return Err(format!("Fabric profili beklenen kimlikte değil ({} ≠ {id})", parsed.id));
    }
    write_atomic(&path, text.as_bytes())?;
    Ok(parsed)
}

pub fn latest_stable_fabric(game: &str) -> R<String> {
    let url = format!("https://meta.fabricmc.net/v2/versions/loader/{game}");
    if mojang::fabric_profile_url(game, "0").is_err() {
        return Err("sürüm adı geçersiz".into());
    }
    let v: serde_json::Value = serde_json::from_str(&get_text(&url, 5 << 20)?).map_err(|e| e.to_string())?;
    v.as_array()
        .and_then(|a| a.iter().find(|e| e.pointer("/loader/stable").and_then(|s| s.as_bool()) == Some(true)))
        .and_then(|e| e.pointer("/loader/version").and_then(|s| s.as_str()))
        .map(String::from)
        .ok_or_else(|| format!("{game} için kararlı Fabric sürümü bulunamadı"))
}

pub fn list_versions(snapshots: bool) -> R<Vec<String>> {
    let m = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL, 20 << 20)?).map_err(|e| e.to_string())?;
    Ok(m.versions(snapshots).iter().map(|v| v.id.clone()).collect())
}

pub fn ensure_installed(root: &Path, spec: &InstanceSpec, env: &Env, rep: &dyn Reporter) -> R<Installed> {
    rep.stage("Mojang sürüm listesi alınıyor");
    let manifest = mojang::parse_manifest(&get_text(mojang::VERSION_MANIFEST_URL, 20 << 20)?).map_err(|e| e.to_string())?;
    let base = fetch_version_json(root, &spec.game, &manifest)?;
    let resolved = match &spec.loader {
        LoaderSpec::Vanilla => version::resolve(base, None),
        LoaderSpec::Fabric { version } => version::resolve(fabric_profile(root, spec, version)?, Some(base)),
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
