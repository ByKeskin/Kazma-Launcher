//! "Kazma Tema modu"nu (Fabric, Minecraft 26+) bir instance'a kurar / kaldırır. Mod, bu deponun `mod-latest` sürümünden indirilir;
//! SHA-256 özeti ve jar içeriği (fabric.mod.json, id=kazma_theme) doğrulanmadan yazılmaz. Mod'un gerektirdiği Fabric API de eklenir.

use crate::net::*;
use crate::report::Reporter;
use std::fs;
use std::io::Read;
use std::path::Path;

pub const FILE: &str = "kazma-theme.jar";
pub const JAR_URL: &str = "https://github.com/ByKeskin/Kazma-Launcher/releases/download/mod-latest/kazma-theme.jar";
pub const SHA_URL: &str = "https://github.com/ByKeskin/Kazma-Launcher/releases/download/mod-latest/kazma-theme.jar.sha256";

/// Mod yalnızca Fabric + Minecraft 26.x ve sonrası için yazıldı.
pub fn eligible(loader: &str, game: &str) -> bool {
    loader == "fabric" && game.split('.').next().and_then(|m| m.parse::<u32>().ok()).map_or(false, |m| m >= 26)
}

pub fn installed(mods_dir: &Path) -> bool {
    mods_dir.join(FILE).is_file()
}

fn has_fabric_api(mods_dir: &Path) -> bool {
    fs::read_dir(mods_dir).map(|r| r.flatten().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("fabric-api"))).unwrap_or(false)
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(b))
}

/// Jar gerçekten Kazma Tema modu mu?
pub fn verify_jar(bytes: &[u8], want_sha256: &str) -> R<()> {
    if bytes.len() < 100 || bytes.len() > 8 << 20 {
        return Err("Mod dosyasının boyutu beklenmedik.".into());
    }
    if !sha256_hex(bytes).eq_ignore_ascii_case(want_sha256.trim()) {
        return Err("Mod dosyasının SHA-256 özeti uyuşmuyor, kurulmadı.".into());
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "Mod dosyası bir jar değil.".to_string())?;
    let mut s = String::new();
    z.by_name("fabric.mod.json").map_err(|_| "Jar içinde fabric.mod.json yok.".to_string())?.take(64 * 1024).read_to_string(&mut s).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&s).map_err(|_| "fabric.mod.json okunamadı.".to_string())?;
    if v.get("id").and_then(|x| x.as_str()) != Some("kazma_theme") {
        return Err("Bu jar Kazma Tema modu değil.".into());
    }
    Ok(())
}

/// Ağ erişimi dışarıdan verilir (testlenebilir): `fetch` → (jar baytları, beklenen SHA-256), `fabric_api` → mods klasörüne Fabric API ekler.
pub fn install_with(
    instance_dir: &Path,
    loader: &str,
    game: &str,
    fetch: &dyn Fn() -> R<(Vec<u8>, String)>,
    fabric_api: &dyn Fn(&Path) -> R<()>,
    rep: &dyn Reporter,
) -> R<()> {
    if !eligible(loader, game) {
        return Err("Kazma Tema modu şimdilik yalnızca Fabric + Minecraft 26 ve sonrası için.".into());
    }
    let mods = instance_dir.join("mods");
    fs::create_dir_all(&mods).map_err(|e| e.to_string())?;
    rep.stage("Kazma Tema modu indiriliyor");
    let (bytes, sha) = fetch()?;
    verify_jar(&bytes, &sha)?;
    if !has_fabric_api(&mods) {
        rep.stage("Fabric API ekleniyor (mod için gerekli)");
        fabric_api(&mods)?;
    }
    write_atomic(&mods.join(FILE), &bytes)
}

pub fn fetch_live() -> R<(Vec<u8>, String)> {
    let sha = get_text(SHA_URL, 256).map_err(|_| "Tema modu henüz yayınlanmamış (GitHub'da 'Kazma Tema modu' iş akışı bir kez çalışmalı).".to_string())?;
    let jar = get_bytes(JAR_URL, 8 << 20).map_err(|_| "Tema modu indirilemedi.".to_string())?;
    Ok((jar, sha.split_whitespace().next().unwrap_or("").to_string()))
}

pub fn remove(instance_dir: &Path) -> R<()> {
    let p = instance_dir.join("mods").join(FILE);
    if p.is_file() {
        fs::remove_file(&p).map_err(|e| format!("Silinemedi ({e}). Oyun açıksa önce kapat."))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;
    use std::io::Write;

    fn jar(id: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(format!(r#"{{"schemaVersion":1,"id":"{id}","version":"1","padding":"{}"}}"#, "x".repeat(1200)).as_bytes()).unwrap();
            z.finish().unwrap();
        }
        buf
    }

    fn tmp(n: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-mt-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn eligibility() {
        for (l, g, ok) in [("fabric", "26.3", true), ("fabric", "26.1.2", true), ("fabric", "1.21.1", false), ("forge", "26.3", false), ("fabric", "x", false), ("fabric", "27.0", true)] {
            assert_eq!(eligible(l, g), ok, "{l} {g}");
        }
    }

    #[test]
    fn jar_verification_rejects_wrong_hash_and_wrong_mod() {
        let j = jar("kazma_theme");
        let sha = sha256_hex(&j);
        assert!(verify_jar(&j, &sha).is_ok());
        assert!(verify_jar(&j, &sha.to_uppercase()).is_ok());
        assert!(verify_jar(&j, &"0".repeat(64)).unwrap_err().contains("SHA-256"));
        let other = jar("baska_mod");
        assert!(verify_jar(&other, &sha256_hex(&other)).unwrap_err().contains("değil"));
        assert!(verify_jar(b"kisa", "x").is_err());
        let junk = vec![7u8; 5000];
        assert!(verify_jar(&junk, &sha256_hex(&junk)).unwrap_err().contains("jar"));
    }

    #[test]
    fn install_adds_jar_and_fabric_api_once_then_removes() {
        let d = tmp("inst");
        let j = jar("kazma_theme");
        let sha = sha256_hex(&j);
        let api_calls = std::cell::Cell::new(0);
        let fetch = || Ok((j.clone(), sha.clone()));
        let api = |m: &Path| {
            api_calls.set(api_calls.get() + 1);
            fs::write(m.join("fabric-api-0.165.0+26.3.jar"), b"x").map_err(|e| e.to_string())
        };
        install_with(&d, "fabric", "26.3", &fetch, &api, &NullReporter).unwrap();
        assert!(installed(&d.join("mods")) && api_calls.get() == 1);
        install_with(&d, "fabric", "26.3", &fetch, &api, &NullReporter).unwrap();
        assert_eq!(api_calls.get(), 1, "Fabric API zaten varsa tekrar eklenmez");
        assert!(install_with(&d, "fabric", "1.21.1", &fetch, &api, &NullReporter).is_err());
        let bad = || Ok((j.clone(), "0".repeat(64)));
        let d2 = tmp("inst2");
        assert!(install_with(&d2, "fabric", "26.3", &bad, &api, &NullReporter).is_err());
        assert!(!installed(&d2.join("mods")), "doğrulanmayan dosya yazılmaz");
        remove(&d).unwrap();
        assert!(!installed(&d.join("mods")));
        remove(&d).unwrap();
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&d2);
    }
}
