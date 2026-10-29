//! Server sahibi araçları: imza anahtarı üretme ve `mods/` klasöründen imzalı manifest oluşturma.

use crate::net::*;
use crate::sync::read_mod_meta;
use kazma_core::manifest::{Loader, ModEntry, PackManifest, Side};
use kazma_core::sign;
use kazma_core::verify::sha512_hex;
use std::fs;
use std::path::Path;

pub fn make_key(out: &Path) -> R<String> {
    if out.exists() {
        return Err(format!("{} zaten var; üzerine yazılmadı", out.display()));
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
    fs::write(out, hex::encode(seed)).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(sign::public_key_hex(&seed))
}

pub fn read_seed(path: &Path) -> R<[u8; 32]> {
    let t = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    hex::decode(t.trim()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| "anahtar dosyası geçersiz (64 onaltılık karakter olmalı)".to_string())
}

#[derive(Debug)]
pub struct ModFile {
    pub file: String,
    pub sha512: String,
    pub size: u64,
    pub id: String,
    pub version: String,
    pub side: Side,
}

pub fn scan_for_manifest(dir: &Path) -> R<Vec<ModFile>> {
    let mut files: Vec<_> = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().filter(|e| e.path().is_file()).collect();
    files.sort_by_key(|e| e.file_name());
    let (mut out, mut bad) = (Vec::new(), Vec::new());
    for e in files {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        let bytes = fs::read(e.path()).map_err(|er| format!("{name}: {er}"))?;
        match read_mod_meta(&bytes) {
            Some(m) => out.push(ModFile { sha512: sha512_hex(&bytes), size: bytes.len() as u64, id: m.id, version: m.version.unwrap_or_default(), side: m.side, file: name }),
            None => bad.push(name),
        }
    }
    if !bad.is_empty() {
        return Err(format!("fabric.mod.json olmayan jar'lar var: {}. Bunlar mod değil ya da Fabric modu değil.", bad.join(", ")));
    }
    Ok(out)
}

/// `lookup`: SHA-512'den indirme adresi bulur (Modrinth). Bulunamayanlar için `base_url` + dosya adı kullanılır.
pub fn build_manifest(
    name: &str,
    game: &str,
    loader: Loader,
    mods: &[ModFile],
    base_url: Option<&str>,
    lookup: &dyn Fn(&str) -> R<Option<String>>,
) -> R<PackManifest> {
    let (mut entries, mut missing) = (Vec::new(), Vec::new());
    for m in mods {
        let url = if m.side == Side::Server {
            String::new() // yalnızca server'da çalışır, istemci indirmez
        } else if let Some(u) = lookup(&m.sha512)? {
            u
        } else if let Some(base) = base_url {
            let enc: String = url::form_urlencoded::byte_serialize(m.file.as_bytes()).collect::<String>().replace('+', "%20");
            format!("{}/{}", base.trim_end_matches('/'), enc)
        } else {
            missing.push(m.file.clone());
            continue;
        };
        entries.push(ModEntry { id: m.id.clone(), version: m.version.clone(), file: m.file.clone(), sha512: m.sha512.clone(), url, side: m.side, size: m.size });
    }
    if !missing.is_empty() {
        return Err(format!(
            "Modrinth'te bulunamayan modlar: {}\nBunları kendi sunucunda barındırıp --base-url https://adresin/mods ile ver.",
            missing.join(", ")
        ));
    }
    Ok(PackManifest { name: name.into(), game: game.into(), loader, mods: entries, signer: None, signature: None })
}

pub fn modrinth_lookup(sha512: &str) -> R<Option<String>> {
    let url = format!("https://api.modrinth.com/v2/version_file/{sha512}?algorithm=sha512");
    match agent().get(&url).call() {
        Ok(r) => {
            let v: serde_json::Value = serde_json::from_reader(r.into_reader().take_limit()).map_err(|e| e.to_string())?;
            Ok(v.get("files")
                .and_then(|f| f.as_array())
                .and_then(|a| a.iter().find(|f| f.pointer("/hashes/sha512").and_then(|h| h.as_str()).map_or(false, |h| h.eq_ignore_ascii_case(sha512))))
                .and_then(|f| f.get("url").and_then(|u| u.as_str()).map(String::from)))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("Modrinth sorgusu başarısız: {e}")),
    }
}

trait TakeLimit: std::io::Read + Sized {
    fn take_limit(self) -> std::io::Take<Self> {
        self.take(2 << 20)
    }
}
impl<T: std::io::Read> TakeLimit for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use kazma_core::trust::TrustStore;

    fn mf(id: &str, side: Side) -> ModFile {
        ModFile { file: format!("{id} mod.jar"), sha512: "a".repeat(128), size: 5, id: id.into(), version: "1.0".into(), side }
    }
    fn loader() -> Loader {
        Loader { kind: "fabric".into(), version: "0.16.9".into() }
    }

    #[test]
    fn builds_and_signs_valid_manifest() {
        let mods = vec![mf("sodium", Side::Both), mf("serveronly", Side::Server)];
        let lookup = |_: &str| -> R<Option<String>> { Ok(Some("https://cdn.modrinth.com/data/x/versions/1/x.jar".into())) };
        // dosya adında boşluk var: güvenli ad kuralına takılmamalı
        let mut m = build_manifest("Dostlar SMP", "1.21.1", loader(), &mods, None, &lookup).unwrap();
        assert_eq!(m.mods[1].url, "", "yalnızca server modunun adresi yok");
        sign::sign(&mut m, &[7u8; 32]);
        m.validate(None).unwrap();
        let key = sign::verify(&m, None).unwrap();
        let mut t = TrustStore::default();
        t.pin("a.com:25565", &key);
        assert!(sign::verify(&m, t.servers.get("a.com:25565").map(String::as_str)).is_ok());
    }

    #[test]
    fn missing_mods_need_base_url() {
        let mods = vec![mf("custom", Side::Both)];
        let none = |_: &str| -> R<Option<String>> { Ok(None) };
        let e = build_manifest("S", "1.21.1", loader(), &mods, None, &none).unwrap_err();
        assert!(e.contains("custom mod.jar") && e.contains("--base-url"));
        let m = build_manifest("S", "1.21.1", loader(), &mods, Some("https://play.example.com/mods/"), &none).unwrap();
        assert_eq!(m.mods[0].url, "https://play.example.com/mods/custom%20mod.jar");
        assert!(m.validate_with(&["play.example.com"]).is_ok());
        assert!(m.validate(None).is_err(), "izinli adres olmadan reddedilir");
    }

    #[test]
    fn key_file_is_not_overwritten() {
        let p = std::env::temp_dir().join(format!("kazma-key-{}.txt", std::process::id()));
        let _ = fs::remove_file(&p);
        let pk = make_key(&p).unwrap();
        assert_eq!(pk.len(), 64);
        assert_eq!(sign::public_key_hex(&read_seed(&p).unwrap()), pk);
        assert!(make_key(&p).is_err(), "var olan anahtar ezilmez");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn scan_rejects_non_mod_jars() {
        let d = std::env::temp_dir().join(format!("kazma-sk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("lib.jar"), b"x").unwrap();
        assert!(scan_for_manifest(&d).unwrap_err().contains("lib.jar"));
        let _ = fs::remove_dir_all(&d);
    }
}
