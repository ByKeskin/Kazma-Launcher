//! Mod eşitleme: yerel `mods/` klasörünü tarar, server manifestiyle karşılaştırılan planı uygular.
//! Silmez: fazla modlar `.disabled` uzantısıyla kenara alınır, geri alınabilir.

use crate::net::*;
use kazma_core::manifest::{safe_file_name, LocalMod, ModEntry, PackManifest, SyncPlan};
use kazma_core::modjar::{parse_fabric_mod_json, to_local, ModMeta};
use kazma_core::trust::TrustStore;
use kazma_core::verify::{sha512_hex, verify_sha512};
use std::fs;
use std::io::Read;
use std::path::Path;

/// Jar içindeki `fabric.mod.json`'ı okur; yoksa ya da bozuksa None.
pub fn read_mod_meta(jar: &[u8]) -> Option<ModMeta> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(jar)).ok()?;
    let f = z.by_name("fabric.mod.json").ok()?;
    let mut s = String::new();
    f.take(2 << 20).read_to_string(&mut s).ok()?;
    parse_fabric_mod_json(&s).ok()
}

pub fn scan_mods(mods_dir: &Path) -> R<Vec<LocalMod>> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(mods_dir) else { return Ok(out) };
    let mut files: Vec<_> = rd.flatten().filter(|e| e.path().is_file()).collect();
    files.sort_by_key(|e| e.file_name());
    for e in files {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        let bytes = fs::read(e.path()).map_err(|er| format!("{name}: {er}"))?;
        let sha = sha512_hex(&bytes);
        out.push(match read_mod_meta(&bytes) {
            Some(meta) => to_local(&name, &sha, &meta),
            // Tanınmayan jar: manifestte olamaz, eşitlemede devre dışı bırakılır.
            None => LocalMod { id: format!("unknown:{name}"), file: name, sha512: sha, client_only: false },
        });
    }
    Ok(out)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub downloaded: Vec<String>,
    pub disabled: Vec<String>,
}

/// Önce eski/fazla dosyaları kenara alır, sonra eksikleri indirip SHA-512 ile doğrular.
/// `fetch` indirme işlevidir (testte sahtesi verilir).
pub fn apply_plan(mods_dir: &Path, plan: &SyncPlan, fetch: &dyn Fn(&str) -> R<Vec<u8>>) -> R<Applied> {
    fs::create_dir_all(mods_dir).map_err(|e| format!("{}: {e}", mods_dir.display()))?;
    let mut done = Applied::default();
    for l in plan.disable.iter().chain(plan.update.iter().map(|(l, _)| l)) {
        if !safe_file_name(&l.file) {
            return Err(format!("güvensiz dosya adı: {}", l.file));
        }
        let src = mods_dir.join(&l.file);
        if src.is_file() {
            let dst = mods_dir.join(format!("{}.disabled", l.file));
            let _ = fs::remove_file(&dst);
            fs::rename(&src, &dst).map_err(|e| format!("{}: {e}", l.file))?;
            done.disabled.push(l.file.clone());
        }
    }
    let wanted: Vec<&ModEntry> = plan.download.iter().chain(plan.update.iter().map(|(_, e)| e)).collect();
    for e in wanted {
        if !safe_file_name(&e.file) {
            return Err(format!("güvensiz dosya adı: {}", e.file));
        }
        let data = fetch(&e.url)?;
        verify_sha512(&data, &e.sha512).map_err(|_| format!("{}: SHA-512 uyuşmuyor, dosya kullanılmadı", e.file))?;
        write_atomic(&mods_dir.join(&e.file), &data)?;
        done.downloaded.push(e.file.clone());
    }
    Ok(done)
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}

/// Kullanıcıya gösterilen onay özeti.
pub fn summarize(plan: &SyncPlan) -> String {
    let mut s = String::new();
    s += &format!("  hazır: {}   indirilecek: {}   güncellenecek: {}   devre dışı kalacak: {}   yerelde kalacak: {}\n",
        plan.ready.len(), plan.download.len(), plan.update.len(), plan.disable.len(), plan.keep.len());
    for e in &plan.download {
        s += &format!("  + {} {} ({})\n", e.id, e.version, mb(e.size));
    }
    for (l, e) in &plan.update {
        s += &format!("  ~ {} {} → {} ({})\n", e.id, l.file, e.version, mb(e.size));
    }
    for l in &plan.disable {
        s += &format!("  - {} ({}) .disabled olarak kenara alınır\n", l.id, l.file);
    }
    if plan.download_bytes() > 0 {
        s += &format!("  toplam indirme: {}\n", mb(plan.download_bytes()));
    }
    s
}

pub fn fetch_manifest(url: &str) -> R<PackManifest> {
    kazma_core::manifest::parse(&get_text(url, 5 << 20)?).map_err(|e| e.to_string())
}

pub fn load_trust(root: &Path) -> TrustStore {
    fs::read_to_string(root.join("trust.json")).ok().and_then(|t| TrustStore::parse(&t).ok()).unwrap_or_default()
}

pub fn save_trust(root: &Path, t: &TrustStore) -> R<()> {
    write_atomic(&root.join("trust.json"), t.to_json().as_bytes())
}

#[cfg(test)]
pub fn tests_jar(id: &str, env: &str, extra: &str) -> Vec<u8> {
    tests::jar(id, env, extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kazma_core::manifest::{plan, Loader, Side};
    use std::io::Write;

    pub fn jar(id: &str, env: &str, extra: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("fabric.mod.json", o).unwrap();
            w.write_all(format!(r#"{{"schemaVersion":1,"id":"{id}","version":"1.0","environment":"{env}"}}"#).as_bytes()).unwrap();
            w.start_file("data.txt", o).unwrap();
            w.write_all(extra.as_bytes()).unwrap();
            w.finish().unwrap();
        }
        buf
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(id: &str, data: &[u8], side: Side) -> ModEntry {
        ModEntry { id: id.into(), version: "2.0".into(), file: format!("{id}.jar"), sha512: sha512_hex(data), url: format!("https://cdn.modrinth.com/data/{id}.jar"), side, size: data.len() as u64 }
    }

    fn manifest(mods: Vec<ModEntry>) -> PackManifest {
        PackManifest { name: "S".into(), game: "1.21.1".into(), loader: Loader { kind: "fabric".into(), version: "0.16.9".into() }, mods, signer: None, signature: None }
    }

    #[test]
    fn scan_reads_ids_sides_and_unknown() {
        let d = tmp("scan");
        fs::write(d.join("sodium.jar"), jar("sodium", "client", "a")).unwrap();
        fs::write(d.join("create.jar"), jar("create", "*", "b")).unwrap();
        fs::write(d.join("lib.jar"), b"not a zip").unwrap();
        fs::write(d.join("old.jar.disabled"), jar("old", "*", "c")).unwrap();
        fs::write(d.join("notes.txt"), "x").unwrap();
        let l = scan_mods(&d).unwrap();
        let ids: Vec<_> = l.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["create", "unknown:lib.jar", "sodium"], ".disabled ve .txt taranmaz");
        assert!(l.iter().find(|m| m.id == "sodium").unwrap().client_only);
        assert!(!l.iter().find(|m| m.id == "create").unwrap().client_only);
        assert_eq!(scan_mods(&d.join("yok")).unwrap().len(), 0, "klasör yoksa boş liste");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn full_sync_cycle() {
        let d = tmp("cycle");
        // yerel: sodium güncel, lithium eski sürüm, iris istemciye özel, optifine uyumsuz
        let sodium = jar("sodium", "client", "s1");
        fs::write(d.join("sodium.jar"), &sodium).unwrap();
        fs::write(d.join("lithium.jar"), jar("lithium", "*", "old")).unwrap();
        fs::write(d.join("iris.jar"), jar("iris", "client", "i")).unwrap();
        fs::write(d.join("optifine.jar"), jar("optifine", "*", "o")).unwrap();

        let new_lithium = jar("lithium", "*", "new");
        let create = jar("create", "*", "c");
        let m = manifest(vec![entry("sodium", &sodium, Side::Both), entry("lithium", &new_lithium, Side::Both), entry("create", &create, Side::Both)]);
        let p = plan(&m, &scan_mods(&d).unwrap());
        assert_eq!(p.ready, ["sodium"]);
        assert_eq!(p.download.len(), 1);
        assert_eq!(p.update.len(), 1);
        assert_eq!(p.keep.len(), 1);
        assert_eq!(p.disable.len(), 1);
        assert!(summarize(&p).contains("+ create"));

        let fetch = |url: &str| -> R<Vec<u8>> {
            if url.ends_with("create.jar") { Ok(create.clone()) } else if url.ends_with("lithium.jar") { Ok(new_lithium.clone()) } else { Err(format!("beklenmeyen {url}")) }
        };
        let a = apply_plan(&d, &p, &fetch).unwrap();
        assert_eq!(a.downloaded.len(), 2);
        assert!(d.join("optifine.jar.disabled").is_file(), "silinmez, kenara alınır");
        assert!(!d.join("optifine.jar").exists());
        assert!(d.join("iris.jar").is_file(), "istemciye özel mod yerinde");
        assert!(d.join("create.jar").is_file());
        // eşitleme sonrası plan boş olmalı
        assert!(plan(&m, &scan_mods(&d).unwrap()).is_noop());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tampered_download_is_rejected_and_not_written() {
        let d = tmp("tamper");
        let good = jar("create", "*", "good");
        let m = manifest(vec![entry("create", &good, Side::Both)]);
        let p = plan(&m, &[]);
        let fetch = |_: &str| -> R<Vec<u8>> { Ok(jar("create", "*", "EVIL")) };
        let err = apply_plan(&d, &p, &fetch).unwrap_err();
        assert!(err.contains("SHA-512 uyuşmuyor"));
        assert!(!d.join("create.jar").exists(), "doğrulanamayan dosya yazılmaz");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unsafe_file_names_never_touch_disk() {
        let d = tmp("unsafe");
        let data = jar("x", "*", "x");
        let mut e = entry("x", &data, Side::Both);
        e.file = "../evil.jar".into();
        let m = manifest(vec![e]);
        let p = plan(&m, &[]);
        let fetch = |_: &str| -> R<Vec<u8>> { Ok(data.clone()) };
        assert!(apply_plan(&d, &p, &fetch).is_err());
        assert!(!d.parent().unwrap().join("evil.jar").exists());
        let _ = fs::remove_dir_all(&d);
    }
}
