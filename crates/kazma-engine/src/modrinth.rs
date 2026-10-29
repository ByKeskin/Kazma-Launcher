//! Modrinth: mod arama ve kurma (gerekli bağımlılıklarla). Ağ işleri dışarıdan verilir, böylece testlenebilir.
//! Dosyalar yalnızca `cdn.modrinth.com` adresinden, SHA-512 doğrulamasıyla indirilir.

use crate::net::*;
use crate::report::Reporter;
use kazma_core::manifest::{safe_file_name, url_trusted};
use kazma_core::verify::verify_sha512;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use url::Url;

#[derive(Debug, Serialize, PartialEq)]
pub struct Hit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub categories: Vec<String>,
    pub client_side: String,
    pub server_side: String,
    pub icon_url: Option<String>,
}

fn token_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'))
}

pub fn search_url(query: &str, game: &str, loader: &str, category: Option<&str>) -> R<String> {
    if !token_ok(game) || !token_ok(loader) || category.map_or(false, |c| !token_ok(c)) {
        return Err("arama ölçütü geçersiz karakter içeriyor".into());
    }
    let mut facets = vec![
        r#"["project_type:mod"]"#.to_string(),
        format!(r#"["versions:{game}"]"#),
        format!(r#"["categories:{loader}"]"#),
    ];
    if let Some(c) = category {
        facets.push(format!(r#"["categories:{c}"]"#));
    }
    let facets = format!("[{}]", facets.join(","));
    Url::parse_with_params("https://api.modrinth.com/v2/search", [("query", query.trim()), ("facets", facets.as_str()), ("limit", "24")])
        .map(|u| u.to_string())
        .map_err(|e| e.to_string())
}

pub fn parse_search(json: &str) -> R<Vec<Hit>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Modrinth yanıtı okunamadı: {e}"))?;
    let hits = v.get("hits").and_then(Value::as_array).ok_or("Modrinth yanıtında 'hits' yok")?;
    let s = |h: &Value, k: &str| h.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Ok(hits
        .iter()
        .filter_map(|h| {
            let id = s(h, "project_id");
            if !token_ok(&id) {
                return None;
            }
            let icon = h.get("icon_url").and_then(Value::as_str).filter(|u| url_trusted(u, None)).map(String::from);
            Some(Hit {
                slug: s(h, "slug"),
                title: s(h, "title"),
                description: s(h, "description"),
                categories: h.get("categories").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default(),
                client_side: s(h, "client_side"),
                server_side: s(h, "server_side"),
                icon_url: icon,
                project_id: id,
            })
        })
        .collect())
}

#[derive(Debug, PartialEq)]
pub struct ModFileInfo {
    pub project_id: String,
    pub version_number: String,
    pub filename: String,
    pub url: String,
    pub sha512: String,
    pub deps: Vec<String>,
}

pub fn versions_url(project: &str, game: &str, loader: &str) -> R<String> {
    if !token_ok(project) || !token_ok(game) || !token_ok(loader) {
        return Err("geçersiz karakter".into());
    }
    Url::parse_with_params(
        &format!("https://api.modrinth.com/v2/project/{project}/version"),
        [("loaders", format!(r#"["{loader}"]"#)), ("game_versions", format!(r#"["{game}"]"#))],
    )
    .map(|u| u.to_string())
    .map_err(|e| e.to_string())
}

/// Sürüm listesinin (en yeni önce) ilk elemanının birincil dosyası. Uygun sürüm yoksa None.
pub fn parse_best_version(project: &str, json: &str) -> R<Option<ModFileInfo>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Modrinth yanıtı okunamadı: {e}"))?;
    let Some(first) = v.as_array().and_then(|a| a.first()) else { return Ok(None) };
    let files = first.get("files").and_then(Value::as_array).ok_or("sürümde dosya yok")?;
    let f = files.iter().find(|f| f.get("primary").and_then(Value::as_bool) == Some(true)).or_else(|| files.first()).ok_or("sürümde dosya yok")?;
    let get = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (filename, url) = (get("filename"), get("url"));
    let sha512 = f.pointer("/hashes/sha512").and_then(Value::as_str).unwrap_or("").to_string();
    if !safe_file_name(&filename) {
        return Err(format!("güvensiz dosya adı: {filename}"));
    }
    if !url_trusted(&url, None) {
        return Err(format!("güvenilmeyen indirme adresi: {url}"));
    }
    if sha512.len() != 128 || !sha512.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Modrinth dosyasında SHA-512 özeti yok".into());
    }
    let deps = first
        .get("dependencies")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|d| d.get("dependency_type").and_then(Value::as_str) == Some("required"))
                .filter_map(|d| d.get("project_id").and_then(Value::as_str).map(String::from))
                .filter(|p| token_ok(p))
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(ModFileInfo {
        project_id: project.to_string(),
        version_number: first.get("version_number").and_then(Value::as_str).unwrap_or("").to_string(),
        filename,
        url,
        sha512: sha512.to_ascii_lowercase(),
        deps,
    }))
}

/// Bir modu ve gerekli bağımlılıklarını `mods_dir`'e kurar. Dönen liste yeni yazılan dosyalardır.
pub fn install_mod(
    mods_dir: &Path,
    project: &str,
    game: &str,
    loader: &str,
    get_json: &dyn Fn(&str) -> R<String>,
    get_file: &dyn Fn(&str) -> R<Vec<u8>>,
    rep: &dyn Reporter,
) -> R<Vec<String>> {
    let mut seen = HashSet::new();
    let mut written = Vec::new();
    install_rec(mods_dir, project, game, loader, get_json, get_file, rep, &mut seen, &mut written, 0)?;
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
fn install_rec(
    dir: &Path,
    project: &str,
    game: &str,
    loader: &str,
    get_json: &dyn Fn(&str) -> R<String>,
    get_file: &dyn Fn(&str) -> R<Vec<u8>>,
    rep: &dyn Reporter,
    seen: &mut HashSet<String>,
    written: &mut Vec<String>,
    depth: u32,
) -> R<()> {
    if depth > 3 || !seen.insert(project.to_string()) {
        return Ok(());
    }
    let json = get_json(&versions_url(project, game, loader)?)?;
    let Some(info) = parse_best_version(project, &json)? else {
        return Err(format!("{project}: {game} / {loader} için uygun sürüm yok"));
    };
    let dest = dir.join(&info.filename);
    let have = std::fs::read(&dest).map(|b| verify_sha512(&b, &info.sha512).is_ok()).unwrap_or(false);
    if !have {
        rep.stage(&format!("{} indiriliyor", info.filename));
        let data = get_file(&info.url)?;
        verify_sha512(&data, &info.sha512).map_err(|_| format!("{}: SHA-512 uyuşmuyor, dosya kullanılmadı", info.filename))?;
        write_atomic(&dest, &data)?;
        written.push(info.filename.clone());
    }
    for d in &info.deps {
        install_rec(dir, d, game, loader, get_json, get_file, rep, seen, written, depth + 1)?;
    }
    Ok(())
}

pub fn search_live(query: &str, game: &str, loader: &str, category: Option<&str>) -> R<Vec<Hit>> {
    parse_search(&get_text(&search_url(query, game, loader, category)?, 4 << 20)?)
}

pub fn install_live(mods_dir: &Path, project: &str, game: &str, loader: &str, rep: &dyn Reporter) -> R<Vec<String>> {
    install_mod(mods_dir, project, game, loader, &|u| get_text(u, 4 << 20), &|u| get_bytes(u, 200 << 20), rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;
    use kazma_core::verify::sha512_hex;
    use std::collections::HashMap;
    use std::fs;

    fn version_json(file: &str, data: &[u8], deps: &[(&str, &str)]) -> String {
        let deps: Vec<String> = deps.iter().map(|(p, t)| format!(r#"{{"project_id":"{p}","version_id":null,"dependency_type":"{t}"}}"#)).collect();
        format!(
            r#"[{{"id":"v1","version_number":"1.2.3","dependencies":[{}],"files":[{{"hashes":{{"sha512":"{}","sha1":"x"}},"url":"https://cdn.modrinth.com/data/P/versions/v1/{file}","filename":"{file}","primary":true,"size":{}}}]}}]"#,
            deps.join(","), sha512_hex(data), data.len()
        )
    }

    #[test]
    fn urls_are_encoded_and_validated() {
        let u = search_url("fabric api & co", "1.21.1", "fabric", Some("optimization")).unwrap();
        assert!(u.starts_with("https://api.modrinth.com/v2/search?query=fabric+api+%26+co&facets="));
        assert!(u.contains("versions%3A1.21.1") && u.contains("categories%3Afabric"));
        assert!(search_url("x", "1.21.1\"]", "fabric", None).is_err());
        assert!(search_url("x", "1.21.1", "fabric", Some("a/b")).is_err());
        let v = versions_url("AANobbMI", "1.21.1", "fabric").unwrap();
        assert!(v.contains("loaders=%5B%22fabric%22%5D") && v.contains("game_versions=%5B%221.21.1%22%5D"));
        assert!(versions_url("../x", "1.21.1", "fabric").is_err());
    }

    #[test]
    fn parses_search_hits_and_drops_bad_ones() {
        let j = r#"{"hits":[
          {"project_id":"AANobbMI","slug":"sodium","title":"Sodium","description":"Hızlı","categories":["optimization","fabric"],"client_side":"required","server_side":"unsupported","icon_url":"https://cdn.modrinth.com/data/AANobbMI/icon.png"},
          {"project_id":"../x","slug":"evil","title":"Evil"},
          {"project_id":"abc123","slug":"nologo","title":"B","icon_url":"https://evil.example/i.png"}
        ]}"#;
        let h = parse_search(j).unwrap();
        assert_eq!(h.len(), 2, "geçersiz kimlikli sonuç atılır");
        assert_eq!(h[0].title, "Sodium");
        assert!(h[0].icon_url.is_some());
        assert_eq!(h[1].icon_url, None, "güvenilmeyen simge adresi atılır");
        assert!(parse_search("{}").is_err());
    }

    #[test]
    fn best_version_validation() {
        let ok = version_json("sodium-0.6.0.jar", b"x", &[("P7dR8mSH", "required"), ("other", "optional")]);
        let i = parse_best_version("AANobbMI", &ok).unwrap().unwrap();
        assert_eq!((i.filename.as_str(), i.version_number.as_str()), ("sodium-0.6.0.jar", "1.2.3"));
        assert_eq!(i.deps, ["P7dR8mSH"], "yalnızca gerekli bağımlılıklar");
        assert_eq!(parse_best_version("p", "[]").unwrap(), None);
        let evil_url = ok.replace("cdn.modrinth.com", "evil.example");
        assert!(parse_best_version("p", &evil_url).is_err());
        let evil_name = ok.replace("sodium-0.6.0.jar", "../x.jar");
        assert!(parse_best_version("p", &evil_name).is_err());
        let no_hash = ok.replace(&sha512_hex(b"x"), "zz");
        assert!(parse_best_version("p", &no_hash).is_err());
    }

    #[test]
    fn installs_with_required_dependency_once() {
        let dir = std::env::temp_dir().join(format!("kazma-mr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (main, api) = (b"main-jar".to_vec(), b"api-jar".to_vec());
        let mut pages: HashMap<String, String> = HashMap::new();
        pages.insert(versions_url("MAIN", "1.21.1", "fabric").unwrap(), version_json("main.jar", &main, &[("FAPI", "required"), ("MAIN", "required")]));
        pages.insert(versions_url("FAPI", "1.21.1", "fabric").unwrap(), version_json("fabric-api.jar", &api, &[]));
        let gj = |u: &str| pages.get(u).cloned().ok_or(format!("beklenmeyen {u}"));
        let gf = |u: &str| if u.ends_with("main.jar") { Ok(main.clone()) } else { Ok(api.clone()) };
        let w = install_mod(&dir, "MAIN", "1.21.1", "fabric", &gj, &gf, &NullReporter).unwrap();
        assert_eq!(w, ["main.jar", "fabric-api.jar"]);
        // ikinci kurulum aynı dosyaları yeniden yazmaz
        assert!(install_mod(&dir, "MAIN", "1.21.1", "fabric", &gj, &gf, &NullReporter).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_file_and_missing_version() {
        let dir = std::env::temp_dir().join(format!("kazma-mr2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let good = b"good".to_vec();
        let page = version_json("m.jar", &good, &[]);
        let gj = |_: &str| Ok(page.clone());
        let evil = |_: &str| Ok(b"EVIL".to_vec());
        assert!(install_mod(&dir, "MAIN", "1.21.1", "fabric", &gj, &evil, &NullReporter).unwrap_err().contains("SHA-512"));
        assert!(!dir.join("m.jar").exists());
        let none = |_: &str| Ok("[]".to_string());
        assert!(install_mod(&dir, "MAIN", "1.21.1", "fabric", &none, &evil, &NullReporter).unwrap_err().contains("uygun sürüm yok"));
        let _ = fs::remove_dir_all(&dir);
    }
}
