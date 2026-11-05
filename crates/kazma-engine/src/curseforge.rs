//! CurseForge: mod, kaynak paketi ve shader arama/kurma (Forge modlarının çoğu burada).
//! API anahtarı gerekir (ücretsiz, console.curseforge.com). Dosyalar yalnızca forgecdn adreslerinden inilir ve
//! SHA-1 ile doğrulanır. Yazarı üçüncü taraf indirmeyi kapatmış modlar (downloadUrl boş) kurulmaz; kullanıcı sitesine yönlendirilir.

use crate::modrinth::{Hit, Kind, Page, PAGE_SIZE};
use crate::net::*;
use crate::report::Reporter;
use kazma_core::manifest::url_trusted;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use url::Url;

const API: &str = "https://api.curseforge.com/v1";
const GAME_ID: &str = "432";

/// Derleme sırasında gömülen anahtar (`KAZMA_CF_KEY` ortam değişkeni); yoksa kullanıcı ayarlardan girer.
pub fn builtin_key() -> Option<&'static str> {
    option_env!("KAZMA_CF_KEY").filter(|k| key_ok(k))
}

/// CurseForge anahtarları `$2a$10$...` biçimindedir. Başlığa girecek değer, satır sonu ve boşluk içeremez.
pub fn key_ok(k: &str) -> bool {
    (16..=120).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'.' | b'/' | b'_' | b'-'))
}

fn class_id(k: Kind) -> &'static str {
    match k {
        Kind::Mod => "6",
        Kind::ResourcePack => "12",
        Kind::Shader => "6552",
    }
}

fn loader_type(l: &str) -> Option<&'static str> {
    match l {
        "forge" => Some("1"),
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        _ => None,
    }
}

fn sort_field(sort: &str) -> &'static str {
    match sort {
        "downloads" => "6",
        "updated" => "3",
        "newest" => "11",
        "follows" => "2",
        _ => "1",
    }
}

fn token_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'))
}

fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn search_url(query: &str, game: &str, loader: Option<&str>, kind: Kind, sort: &str, offset: u64) -> R<String> {
    if !token_ok(game) || loader.map_or(false, |l| !token_ok(l)) {
        return Err("arama ölçütü geçersiz karakter içeriyor".into());
    }
    let mut p: Vec<(&str, String)> = vec![
        ("gameId", GAME_ID.into()),
        ("classId", class_id(kind).into()),
        ("gameVersion", game.into()),
        ("searchFilter", query.trim().into()),
        ("sortField", sort_field(sort).into()),
        ("sortOrder", "desc".into()),
        ("pageSize", PAGE_SIZE.to_string()),
        ("index", offset.min(9_000).to_string()),
    ];
    if let (Kind::Mod, Some(t)) = (kind, loader.and_then(loader_type)) {
        p.push(("modLoaderType", t.into()));
    }
    Url::parse_with_params(&format!("{API}/mods/search"), p).map(|u| u.to_string()).map_err(|e| e.to_string())
}

fn icon_ok(u: &str) -> bool {
    Url::parse(u).map(|u| u.scheme() == "https" && u.host_str() == Some("media.forgecdn.net")).unwrap_or(false)
}

/// Tarayıcıda açılabilecek CurseForge adresleri: geliştirici konsolu ve Minecraft proje sayfaları.
pub fn page_url_ok(u: &str) -> bool {
    u == "https://console.curseforge.com/" || (site_ok(u) && Url::parse(u).map(|p| p.path().starts_with("/minecraft/")).unwrap_or(false))
}

fn site_ok(u: &str) -> bool {
    Url::parse(u).map(|u| u.scheme() == "https" && u.host_str() == Some("www.curseforge.com")).unwrap_or(false)
}

pub fn parse_page(json: &str) -> R<Page> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("CurseForge yanıtı okunamadı: {e}"))?;
    let data = v.get("data").and_then(Value::as_array).ok_or("CurseForge yanıtında 'data' yok")?;
    let s = |h: &Value, k: &str| h.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let hits = data
        .iter()
        .filter_map(|h| {
            let id = h.get("id").and_then(Value::as_u64)?.to_string();
            let icon = h.pointer("/logo/thumbnailUrl").and_then(Value::as_str).filter(|u| icon_ok(u)).map(String::from);
            let page = h.pointer("/links/websiteUrl").and_then(Value::as_str).filter(|u| site_ok(u)).map(String::from);
            Some(Hit {
                slug: s(h, "slug"),
                title: s(h, "name"),
                description: s(h, "summary"),
                categories: h.get("categories").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.get("slug").and_then(Value::as_str).map(String::from)).collect()).unwrap_or_default(),
                client_side: "optional".into(),
                server_side: "optional".into(),
                icon_url: icon,
                downloads: h.get("downloadCount").and_then(Value::as_f64).map(|d| d as u64).unwrap_or(0),
                source: "curseforge".into(),
                page_url: page,
                project_id: id,
            })
        })
        .collect();
    let total = v.pointer("/pagination/totalCount").and_then(Value::as_u64).unwrap_or(0);
    let offset = v.pointer("/pagination/index").and_then(Value::as_u64).unwrap_or(0);
    Ok(Page { hits, total, offset })
}

pub fn files_url(mod_id: &str, game: &str, loader: Option<&str>, kind: Kind) -> R<String> {
    if !id_ok(mod_id) || !token_ok(game) {
        return Err("geçersiz mod kimliği ya da sürüm".into());
    }
    let mut p: Vec<(&str, String)> = vec![("gameVersion", game.into()), ("pageSize", "50".into())];
    if let (Kind::Mod, Some(t)) = (kind, loader.and_then(loader_type)) {
        p.push(("modLoaderType", t.into()));
    }
    Url::parse_with_params(&format!("{API}/mods/{mod_id}/files"), p).map(|u| u.to_string()).map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq)]
pub struct FileInfo {
    pub filename: String,
    pub url: String,
    pub sha1: String,
    pub deps: Vec<String>,
}

fn safe_name(f: &str, exts: &[&str]) -> bool {
    let l = f.to_ascii_lowercase();
    !f.is_empty() && f.len() <= 128 && !f.contains("..") && !f.contains(['/', '\\', ':']) && !f.chars().any(|c| c.is_control()) && exts.iter().any(|e| l.ends_with(&format!(".{e}")))
}

/// En uygun dosya: önce kararlı (release) sürümler, sonra en yeni tarih. Uygun dosya yoksa Ok(None).
pub fn parse_best_file(json: &str, exts: &[&str]) -> R<Option<FileInfo>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("CurseForge yanıtı okunamadı: {e}"))?;
    let data = v.get("data").and_then(Value::as_array).ok_or("CurseForge yanıtında 'data' yok")?;
    let mut files: Vec<&Value> = data.iter().filter(|f| f.get("isServerPack").and_then(Value::as_bool) != Some(true)).collect();
    // (kararlı mı, tarih) büyükten küçüğe; tarih ISO-8601 olduğu için metin sırası yeterli.
    files.sort_by(|a, b| {
        let key = |f: &Value| (f.get("releaseType").and_then(Value::as_u64) == Some(1), f.get("fileDate").and_then(Value::as_str).unwrap_or("").to_string());
        key(b).cmp(&key(a))
    });
    let Some(f) = files.first() else { return Ok(None) };
    let filename = f.get("fileName").and_then(Value::as_str).unwrap_or("").to_string();
    if !safe_name(&filename, exts) {
        return Err(format!("güvensiz ya da beklenmeyen dosya adı: {filename}"));
    }
    let Some(url) = f.get("downloadUrl").and_then(Value::as_str).filter(|u| !u.is_empty()) else {
        return Err(format!("{filename}: yazar bu dosyanın üçüncü taraf launcher'lardan indirilmesine izin vermiyor. CurseForge sayfasından elle indirip mods klasörüne atabilirsin."));
    };
    if !url_trusted(url, None) {
        return Err(format!("güvenilmeyen indirme adresi: {url}"));
    }
    let sha1 = f
        .get("hashes")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|h| h.get("algo").and_then(Value::as_u64) == Some(1)))
        .and_then(|h| h.get("value").and_then(Value::as_str))
        .unwrap_or("")
        .to_ascii_lowercase();
    if sha1.len() != 40 || !sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{filename}: CurseForge dosyasında SHA-1 özeti yok"));
    }
    let deps = f
        .get("dependencies")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|d| d.get("relationType").and_then(Value::as_u64) == Some(3)).filter_map(|d| d.get("modId").and_then(Value::as_u64).map(|i| i.to_string())).collect())
        .unwrap_or_default();
    Ok(Some(FileInfo { filename, url: url.to_string(), sha1, deps }))
}

pub fn search_live(key: &str, query: &str, game: &str, loader: Option<&str>, kind: Kind, sort: &str, offset: u64) -> R<Page> {
    if !key_ok(key) {
        return Err("CurseForge API anahtarı geçersiz.".into());
    }
    let url = search_url(query, game, loader, kind, sort, offset)?;
    parse_page(&get_text_with(&url, 4 << 20, &[("x-api-key", key), ("Accept", "application/json")]).map_err(|e| format!("CurseForge: {e}"))?)
}

#[allow(clippy::too_many_arguments)]
pub fn install_project(
    dir: &Path,
    mod_id: &str,
    game: &str,
    loader: Option<&str>,
    kind: Kind,
    get_json: &dyn Fn(&str) -> R<String>,
    get_file: &dyn Fn(&str) -> R<Vec<u8>>,
    rep: &dyn Reporter,
) -> R<Vec<String>> {
    let mut seen = HashSet::new();
    let mut written = Vec::new();
    rec(dir, mod_id, game, loader, kind, get_json, get_file, rep, &mut seen, &mut written, 0)?;
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
fn rec(
    dir: &Path,
    mod_id: &str,
    game: &str,
    loader: Option<&str>,
    kind: Kind,
    get_json: &dyn Fn(&str) -> R<String>,
    get_file: &dyn Fn(&str) -> R<Vec<u8>>,
    rep: &dyn Reporter,
    seen: &mut HashSet<String>,
    written: &mut Vec<String>,
    depth: u32,
) -> R<()> {
    if depth > 3 || !seen.insert(mod_id.to_string()) {
        return Ok(());
    }
    let json = get_json(&files_url(mod_id, game, loader, kind)?)?;
    let Some(info) = parse_best_file(&json, kind.exts())? else {
        return Err(format!("CurseForge {mod_id}: {game} sürümü için uygun dosya yok"));
    };
    let dest = dir.join(&info.filename);
    let have = std::fs::read(&dest).map(|b| sha1_hex(&b) == info.sha1).unwrap_or(false);
    if !have {
        rep.stage(&format!("{} indiriliyor", info.filename));
        let data = get_file(&info.url)?;
        if sha1_hex(&data) != info.sha1 {
            return Err(format!("{}: SHA-1 uyuşmuyor, dosya kullanılmadı", info.filename));
        }
        write_atomic(&dest, &data)?;
        written.push(info.filename.clone());
    }
    if kind == Kind::Mod {
        for d in &info.deps {
            rec(dir, d, game, loader, kind, get_json, get_file, rep, seen, written, depth + 1).map_err(|e| format!("{} kuruldu ama gerekli bağımlılık kurulamadı: {e}", info.filename))?;
        }
    }
    Ok(())
}

pub fn install_live(key: &str, dir: &Path, mod_id: &str, game: &str, loader: Option<&str>, kind: Kind, rep: &dyn Reporter) -> R<Vec<String>> {
    if !key_ok(key) {
        return Err("CurseForge API anahtarı geçersiz.".into());
    }
    let h = [("x-api-key", key), ("Accept", "application/json")];
    install_project(dir, mod_id, game, loader, kind, &|u| get_text_with(u, 4 << 20, &h).map_err(|e| format!("CurseForge: {e}")), &|u| get_bytes(u, 300 << 20), rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;
    use std::cell::RefCell;
    use std::fs;

    fn file_json(id: u64, name: &str, data: &[u8], url: Option<&str>, rel: u64, date: &str, deps: &[u64]) -> String {
        let u = url.map(|u| format!("\"{u}\"")).unwrap_or("null".into());
        let d: Vec<String> = deps.iter().map(|m| format!(r#"{{"modId":{m},"relationType":3}}"#)).collect();
        format!(
            r#"{{"id":{id},"fileName":"{name}","downloadUrl":{u},"releaseType":{rel},"fileDate":"{date}","hashes":[{{"value":"{}","algo":1}},{{"value":"abc","algo":2}}],"dependencies":[{}]}}"#,
            sha1_hex(data), d.join(",")
        )
    }
    fn files(list: &[String]) -> String {
        format!(r#"{{"data":[{}]}}"#, list.join(","))
    }
    fn tmp(n: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-cf-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn url_has_loader_sort_and_paging() {
        let u = search_url("jei", "1.20.1", Some("forge"), Kind::Mod, "downloads", 48).unwrap();
        for part in ["classId=6", "gameVersion=1.20.1", "searchFilter=jei", "modLoaderType=1", "sortField=6", "index=48", "pageSize=24"] {
            assert!(u.contains(part), "{part} yok: {u}");
        }
        let r = search_url("x", "1.20.1", Some("forge"), Kind::Shader, "relevance", 0).unwrap();
        assert!(r.contains("classId=6552") && !r.contains("modLoaderType"), "shader'da yükleyici süzgeci olmaz");
        assert!(search_url("x", "1.20.1&evil=1", None, Kind::Mod, "", 0).is_err());
    }

    #[test]
    fn key_validation() {
        assert!(key_ok("$2a$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234"));
        assert!(!key_ok("kisa"));
        assert!(!key_ok("abcdefghijklmnopqrstuvwxyz\nX-Evil: 1"));
        assert!(!key_ok("abcdefghijklmnop qrstuvwxyz"));
    }

    #[test]
    fn parses_hits_and_filters_urls() {
        let j = r#"{"data":[
          {"id":238222,"slug":"jei","name":"JEI","summary":"Tarifler","downloadCount":1.5e8,"categories":[{"slug":"utility-qol"}],
           "logo":{"thumbnailUrl":"https://media.forgecdn.net/avatars/thumbnails/1/1/64/64/a.png"},"links":{"websiteUrl":"https://www.curseforge.com/minecraft/mc-mods/jei"}},
          {"id":5,"slug":"x","name":"X","summary":"","logo":{"thumbnailUrl":"https://evil.example/a.png"},"links":{"websiteUrl":"https://evil.example/"}},
          {"slug":"noid","name":"N"}],
          "pagination":{"index":24,"pageSize":24,"resultCount":2,"totalCount":900}}"#;
        let p = parse_page(j).unwrap();
        assert_eq!((p.total, p.offset, p.hits.len()), (900, 24, 2), "kimliksiz kayıt atılır");
        assert_eq!(p.hits[0].project_id, "238222");
        assert_eq!(p.hits[0].downloads, 150_000_000);
        assert!(p.hits[0].icon_url.is_some() && p.hits[0].page_url.is_some());
        assert!(p.hits[1].icon_url.is_none() && p.hits[1].page_url.is_none(), "yabancı adresler atılır");
        assert_eq!(p.hits[0].source, "curseforge");
        assert!(parse_page("{}").is_err());
    }

    #[test]
    fn picks_newest_release_and_refuses_blocked_files() {
        let d = b"jar";
        let url = "https://edge.forgecdn.net/files/1/2/a.jar";
        let j = files(&[
            file_json(1, "old-1.0.jar", d, Some(url), 1, "2024-01-01T00:00:00Z", &[]),
            file_json(2, "new-2.0.jar", d, Some(url), 1, "2025-01-01T00:00:00Z", &[7]),
            file_json(3, "beta-3.0.jar", d, Some(url), 2, "2026-01-01T00:00:00Z", &[]),
        ]);
        let f = parse_best_file(&j, &["jar"]).unwrap().unwrap();
        assert_eq!((f.filename.as_str(), f.deps.as_slice()), ("new-2.0.jar", &["7".to_string()][..]), "beta yerine en yeni kararlı");
        let blocked = files(&[file_json(1, "a.jar", d, None, 1, "2025-01-01T00:00:00Z", &[])]);
        assert!(parse_best_file(&blocked, &["jar"]).unwrap_err().contains("izin vermiyor"));
        let evil = files(&[file_json(1, "a.jar", d, Some("https://evil.example/a.jar"), 1, "2025-01-01T00:00:00Z", &[])]);
        assert!(parse_best_file(&evil, &["jar"]).unwrap_err().contains("güvenilmeyen"));
        let bad = files(&[file_json(1, "../a.jar", d, Some(url), 1, "2025-01-01T00:00:00Z", &[])]);
        assert!(parse_best_file(&bad, &["jar"]).is_err());
        assert_eq!(parse_best_file(r#"{"data":[]}"#, &["jar"]).unwrap(), None);
    }

    #[test]
    fn installs_with_dependency_and_checks_hash() {
        let dir = tmp("ok");
        let (main, dep) = (b"main-bytes".to_vec(), b"dep-bytes".to_vec());
        let url = "https://edge.forgecdn.net/files/9/9/x.jar";
        let calls = RefCell::new(Vec::new());
        let get_json = |u: &str| -> R<String> {
            calls.borrow_mut().push(u.to_string());
            if u.contains("/mods/100/") {
                Ok(files(&[file_json(1, "main.jar", &main, Some(url), 1, "2025-01-01T00:00:00Z", &[200])]))
            } else {
                Ok(files(&[file_json(2, "dep.jar", &dep, Some(&format!("{url}?d")), 1, "2025-01-01T00:00:00Z", &[100])]))
            }
        };
        let get_file = |u: &str| -> R<Vec<u8>> { Ok(if u.ends_with("?d") { dep.clone() } else { main.clone() }) };
        let w = install_project(&dir, "100", "1.20.1", Some("forge"), Kind::Mod, &get_json, &get_file, &NullReporter).unwrap();
        assert_eq!(w, vec!["main.jar", "dep.jar"], "döngüsel bağımlılık bir kez");
        assert!(dir.join("main.jar").is_file() && dir.join("dep.jar").is_file());
        // bozuk dosya
        let bad = |_: &str| -> R<Vec<u8>> { Ok(b"bozuk".to_vec()) };
        let dir2 = tmp("bad");
        let e = install_project(&dir2, "100", "1.20.1", Some("forge"), Kind::Mod, &get_json, &bad, &NullReporter).unwrap_err();
        assert!(e.contains("SHA-1"));
        assert!(!dir2.join("main.jar").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn only_curseforge_pages_open() {
        assert!(page_url_ok("https://www.curseforge.com/minecraft/mc-mods/jei"));
        assert!(page_url_ok("https://console.curseforge.com/"));
        assert!(!page_url_ok("https://www.curseforge.com/download?x=1"));
        assert!(!page_url_ok("https://evil.example/minecraft/x"));
        assert!(!page_url_ok("file:///C:/Windows/System32/calc.exe"));
    }

    #[test]
    fn bad_ids_rejected() {
        assert!(files_url("12; drop", "1.20.1", None, Kind::Mod).is_err());
        assert!(files_url("123", "1.20.1", Some("forge"), Kind::Mod).unwrap().contains("modLoaderType=1"));
    }
}
