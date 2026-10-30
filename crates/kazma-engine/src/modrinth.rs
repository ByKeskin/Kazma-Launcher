//! Modrinth: mod, kaynak paketi ve shader arama/kurma. Modlar gerekli bağımlılıklarıyla kurulur.
//! Ağ işleri dışarıdan verilir, böylece testlenebilir. Dosyalar yalnızca `cdn.modrinth.com` adresinden,
//! SHA-512 doğrulamasıyla indirilir.

use crate::net::*;
use crate::report::Reporter;
use kazma_core::manifest::{url_trusted, ModEntry, Side};
use kazma_core::verify::verify_sha512;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use url::Url;

/// Aranabilen içerik türleri. Her birinin Modrinth proje türü, instance içindeki klasörü ve dosya uzantısı farklıdır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mod,
    ResourcePack,
    Shader,
}

impl Kind {
    pub fn parse(s: &str) -> R<Kind> {
        match s {
            "mod" => Ok(Kind::Mod),
            "resourcepack" => Ok(Kind::ResourcePack),
            "shader" => Ok(Kind::Shader),
            o => Err(format!("bilinmeyen içerik türü: {o}")),
        }
    }
    pub fn project_type(self) -> &'static str {
        match self {
            Kind::Mod => "mod",
            Kind::ResourcePack => "resourcepack",
            Kind::Shader => "shader",
        }
    }
    /// Instance klasörü altındaki hedef klasör.
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Mod => "mods",
            Kind::ResourcePack => "resourcepacks",
            Kind::Shader => "shaderpacks",
        }
    }
    pub fn exts(self) -> &'static [&'static str] {
        match self {
            Kind::Mod => &["jar"],
            _ => &["zip"],
        }
    }
}

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

/// Güvenli dosya adı: yol ayracı, `..`, kontrol karakteri yok ve uzantı izinli.
fn safe_name(f: &str, exts: &[&str]) -> bool {
    let lower = f.to_ascii_lowercase();
    !f.is_empty()
        && f.len() <= 128
        && !f.contains("..")
        && !f.contains(['/', '\\', ':'])
        && exts.iter().any(|e| lower.ends_with(&format!(".{e}")))
        && !f.chars().any(|c| c.is_control())
}

/// `loader`: yalnızca modlarda anlamlı (fabric, forge...). Kaynak paketi ve shader aramasında verilmez.
pub fn search_url(query: &str, game: &str, loader: Option<&str>, category: Option<&str>, kind: Kind) -> R<String> {
    if !token_ok(game) || loader.map_or(false, |l| !token_ok(l)) || category.map_or(false, |c| !token_ok(c)) {
        return Err("arama ölçütü geçersiz karakter içeriyor".into());
    }
    let mut facets = vec![format!(r#"["project_type:{}"]"#, kind.project_type()), format!(r#"["versions:{game}"]"#)];
    if let (Kind::Mod, Some(l)) = (kind, loader) {
        facets.push(format!(r#"["categories:{l}"]"#));
    }
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

pub fn versions_url(project: &str, game: &str, loader: Option<&str>) -> R<String> {
    if !token_ok(project) || !token_ok(game) || loader.map_or(false, |l| !token_ok(l)) {
        return Err("geçersiz karakter".into());
    }
    let mut params = vec![("game_versions", format!(r#"["{game}"]"#))];
    if let Some(l) = loader {
        params.insert(0, ("loaders", format!(r#"["{l}"]"#)));
    }
    Url::parse_with_params(&format!("https://api.modrinth.com/v2/project/{project}/version"), params).map(|u| u.to_string()).map_err(|e| e.to_string())
}

/// Bir sürüm nesnesinin birincil dosyası: (dosya adı, adres, sha512, boyut). Her alan doğrulanır.
fn file_from_version(v: &Value, exts: &[&str]) -> R<(String, String, String, u64)> {
    let files = v.get("files").and_then(Value::as_array).ok_or("sürümde dosya yok")?;
    let f = files.iter().find(|f| f.get("primary").and_then(Value::as_bool) == Some(true)).or_else(|| files.first()).ok_or("sürümde dosya yok")?;
    let get = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (filename, url) = (get("filename"), get("url"));
    let sha512 = f.pointer("/hashes/sha512").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    if !safe_name(&filename, exts) {
        return Err(format!("güvensiz ya da beklenmeyen dosya adı: {filename}"));
    }
    if !url_trusted(&url, None) {
        return Err(format!("güvenilmeyen indirme adresi: {url}"));
    }
    if sha512.len() != 128 || !sha512.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Modrinth dosyasında SHA-512 özeti yok".into());
    }
    Ok((filename, url, sha512, f.get("size").and_then(Value::as_u64).unwrap_or(0)))
}

/// Sürüm listesinin (en yeni önce) ilk elemanının birincil dosyası. Uygun sürüm yoksa None.
pub fn parse_best_version(project: &str, json: &str, exts: &[&str]) -> R<Option<ModFileInfo>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Modrinth yanıtı okunamadı: {e}"))?;
    let Some(first) = v.as_array().and_then(|a| a.first()) else { return Ok(None) };
    let (filename, url, sha512, _) = file_from_version(first, exts)?;
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
    Ok(Some(ModFileInfo { project_id: project.to_string(), version_number: first.get("version_number").and_then(Value::as_str).unwrap_or("").to_string(), filename, url, sha512, deps }))
}

// ---------- paket kodu için toplu sorgular ----------

pub fn versions_batch_url(ids: &[String]) -> R<String> {
    if ids.is_empty() || ids.iter().any(|i| !token_ok(i)) {
        return Err("sürüm kimliği geçersiz".into());
    }
    let list = serde_json::to_string(ids).map_err(|e| e.to_string())?;
    Url::parse_with_params("https://api.modrinth.com/v2/versions", [("ids", list)]).map(|u| u.to_string()).map_err(|e| e.to_string())
}

/// `wanted` sırasıyla mod kayıtları. İstenen sürümlerden biri bulunamazsa hepsi listelenerek hata verilir.
pub fn parse_versions_batch(json: &str, wanted: &[String]) -> R<Vec<ModEntry>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Modrinth yanıtı okunamadı: {e}"))?;
    let arr = v.as_array().ok_or("Modrinth yanıtı beklenen biçimde değil")?;
    let by_id: HashMap<&str, &Value> = arr.iter().filter_map(|x| x.get("id").and_then(Value::as_str).map(|i| (i, x))).collect();
    let (mut out, mut missing) = (Vec::new(), Vec::new());
    for id in wanted {
        let Some(ver) = by_id.get(id.as_str()) else {
            missing.push(id.clone());
            continue;
        };
        let (file, url, sha512, size) = file_from_version(ver, &["jar"])?;
        let project = ver.get("project_id").and_then(Value::as_str).filter(|p| token_ok(p)).ok_or("sürümde proje kimliği yok")?;
        out.push(ModEntry { id: project.into(), version: ver.get("version_number").and_then(Value::as_str).unwrap_or("").into(), file, sha512, url, side: Side::Both, size });
    }
    if !missing.is_empty() {
        return Err(format!("Modrinth'te bulunamayan sürümler: {}. Paket kodu eskimiş ya da bozuk olabilir.", missing.join(", ")));
    }
    Ok(out)
}

/// Dosya özetlerinden Modrinth sürüm kimliklerini bulma: yanıt `{ özet: sürüm nesnesi }`.
pub fn parse_version_files(json: &str) -> R<HashMap<String, String>> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Modrinth yanıtı okunamadı: {e}"))?;
    let o = v.as_object().ok_or("Modrinth yanıtı beklenen biçimde değil")?;
    Ok(o.iter().filter_map(|(h, ver)| ver.get("id").and_then(Value::as_str).filter(|i| token_ok(i)).map(|i| (h.to_ascii_lowercase(), i.to_string()))).collect())
}

pub fn lookup_hashes_live(hashes: &[String]) -> R<HashMap<String, String>> {
    let mut out = HashMap::new();
    for chunk in hashes.chunks(100) {
        let body = serde_json::json!({ "hashes": chunk, "algorithm": "sha512" }).to_string();
        let resp = agent().post("https://api.modrinth.com/v2/version_files").set("Content-Type", "application/json").send_string(&body).map_err(|e| format!("Modrinth sorgusu başarısız: {e}"))?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::Read::take(resp.into_reader(), 8 << 20), &mut text).map_err(|e| e.to_string())?;
        out.extend(parse_version_files(&text)?);
    }
    Ok(out)
}

pub fn resolve_pack_live(ids: &[String]) -> R<Vec<ModEntry>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(50) {
        out.extend(parse_versions_batch(&get_text(&versions_batch_url(chunk)?, 8 << 20)?, chunk)?);
    }
    Ok(out)
}

// ---------- kurma ----------

/// Bir projeyi `dir`'e kurar. Modlarda gerekli bağımlılıklar da kurulur. Dönen liste yeni yazılan dosyalardır.
#[allow(clippy::too_many_arguments)]
pub fn install_project(
    dir: &Path,
    project: &str,
    game: &str,
    loader: Option<&str>,
    kind: Kind,
    get_json: &dyn Fn(&str) -> R<String>,
    get_file: &dyn Fn(&str) -> R<Vec<u8>>,
    rep: &dyn Reporter,
) -> R<Vec<String>> {
    let mut seen = HashSet::new();
    let mut written = Vec::new();
    install_rec(dir, project, game, loader, kind, get_json, get_file, rep, &mut seen, &mut written, 0)?;
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
fn install_rec(
    dir: &Path,
    project: &str,
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
    if depth > 3 || !seen.insert(project.to_string()) {
        return Ok(());
    }
    let json = get_json(&versions_url(project, game, if kind == Kind::Mod { loader } else { None })?)?;
    let Some(info) = parse_best_version(project, &json, kind.exts())? else {
        return Err(format!("{project}: {game} sürümü için uygun dosya yok"));
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
    if kind == Kind::Mod {
        for d in &info.deps {
            install_rec(dir, d, game, loader, kind, get_json, get_file, rep, seen, written, depth + 1)?;
        }
    }
    Ok(())
}

pub fn search_live(query: &str, game: &str, loader: Option<&str>, category: Option<&str>, kind: Kind) -> R<Vec<Hit>> {
    parse_search(&get_text(&search_url(query, game, loader, category, kind)?, 4 << 20)?)
}

pub fn install_live(dir: &Path, project: &str, game: &str, loader: Option<&str>, kind: Kind, rep: &dyn Reporter) -> R<Vec<String>> {
    install_project(dir, project, game, loader, kind, &|u| get_text(u, 4 << 20), &|u| get_bytes(u, 300 << 20), rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::NullReporter;
    use kazma_core::verify::sha512_hex;
    use std::fs;

    fn version_json_ext(file: &str, data: &[u8], deps: &[(&str, &str)]) -> String {
        let deps: Vec<String> = deps.iter().map(|(p, t)| format!(r#"{{"project_id":"{p}","version_id":null,"dependency_type":"{t}"}}"#)).collect();
        format!(
            r#"[{{"id":"v1","version_number":"1.2.3","dependencies":[{}],"files":[{{"hashes":{{"sha512":"{}","sha1":"x"}},"url":"https://cdn.modrinth.com/data/P/versions/v1/{file}","filename":"{file}","primary":true,"size":{}}}]}}]"#,
            deps.join(","), sha512_hex(data), data.len()
        )
    }
    fn version_json(file: &str, data: &[u8], deps: &[(&str, &str)]) -> String {
        version_json_ext(file, data, deps)
    }

    #[test]
    fn kinds_map_to_folders_and_types() {
        assert_eq!(Kind::parse("resourcepack").unwrap().dir(), "resourcepacks");
        assert_eq!(Kind::parse("shader").unwrap().dir(), "shaderpacks");
        assert_eq!((Kind::Mod.dir(), Kind::Mod.exts()), ("mods", &["jar"][..]));
        assert_eq!(Kind::Shader.project_type(), "shader");
        assert!(Kind::parse("modpack").is_err());
    }

    #[test]
    fn urls_are_encoded_and_validated() {
        let u = search_url("fabric api & co", "1.21.1", Some("fabric"), Some("optimization"), Kind::Mod).unwrap();
        assert!(u.starts_with("https://api.modrinth.com/v2/search?query=fabric+api+%26+co&facets="));
        assert!(u.contains("versions%3A1.21.1") && u.contains("categories%3Afabric") && u.contains("project_type%3Amod"));
        let rp = search_url("", "1.21.1", Some("fabric"), Some("realistic"), Kind::ResourcePack).unwrap();
        assert!(rp.contains("project_type%3Aresourcepack") && !rp.contains("categories%3Afabric"), "kaynak paketinde yükleyici filtresi yok");
        assert!(rp.contains("categories%3Arealistic"));
        let sh = search_url("bsl", "1.21.1", None, None, Kind::Shader).unwrap();
        assert!(sh.contains("project_type%3Ashader"));
        assert!(search_url("x", "1.21.1\"]", Some("fabric"), None, Kind::Mod).is_err());
        assert!(search_url("x", "1.21.1", Some("fabric"), Some("a/b"), Kind::Mod).is_err());
        let v = versions_url("AANobbMI", "1.21.1", Some("fabric")).unwrap();
        assert!(v.contains("loaders=%5B%22fabric%22%5D") && v.contains("game_versions=%5B%221.21.1%22%5D"));
        let nv = versions_url("AANobbMI", "1.21.1", None).unwrap();
        assert!(!nv.contains("loaders"));
        assert!(versions_url("../x", "1.21.1", Some("fabric")).is_err());
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
        let i = parse_best_version("AANobbMI", &ok, &["jar"]).unwrap().unwrap();
        assert_eq!((i.filename.as_str(), i.version_number.as_str()), ("sodium-0.6.0.jar", "1.2.3"));
        assert_eq!(i.deps, ["P7dR8mSH"], "yalnızca gerekli bağımlılıklar");
        assert_eq!(parse_best_version("p", "[]", &["jar"]).unwrap(), None);
        assert!(parse_best_version("p", &ok.replace("cdn.modrinth.com", "evil.example"), &["jar"]).is_err());
        assert!(parse_best_version("p", &ok.replace("sodium-0.6.0.jar", "../x.jar"), &["jar"]).is_err());
        assert!(parse_best_version("p", &ok.replace(&sha512_hex(b"x"), "zz"), &["jar"]).is_err());
        // uzantı denetimi: mod kutusuna zip, paket klasörüne jar giremez
        let zip = version_json_ext("pack.zip", b"z", &[]);
        assert!(parse_best_version("p", &zip, &["jar"]).is_err());
        assert!(parse_best_version("p", &zip, &["zip"]).unwrap().is_some());
        assert!(parse_best_version("p", &ok, &["zip"]).is_err());
    }

    #[test]
    fn batch_versions_become_entries() {
        let v = |id: &str, proj: &str, file: &str, data: &[u8]| format!(r#"{{"id":"{id}","project_id":"{proj}","version_number":"1.0","files":[{{"hashes":{{"sha512":"{}"}},"url":"https://cdn.modrinth.com/data/{proj}/{file}","filename":"{file}","primary":true,"size":{}}}]}}"#, sha512_hex(data), data.len());
        let json = format!("[{},{}]", v("v2", "pB", "b.jar", b"b"), v("v1", "pA", "a.jar", b"a"));
        let wanted = vec!["v1".to_string(), "v2".to_string()];
        let e = parse_versions_batch(&json, &wanted).unwrap();
        assert_eq!(e.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["pA", "pB"], "istenen sırayla");
        assert_eq!(e[0].sha512, sha512_hex(b"a"));
        let miss = parse_versions_batch(&json, &["v1".into(), "v9".into(), "v8".into()]).unwrap_err();
        assert!(miss.contains("v9") && miss.contains("v8"));
        assert!(parse_versions_batch(&json.replace("cdn.modrinth.com", "evil.example"), &wanted).is_err());
        let u = versions_batch_url(&wanted).unwrap();
        assert!(u.starts_with("https://api.modrinth.com/v2/versions?ids=%5B%22v1%22%2C%22v2%22%5D"));
        assert!(versions_batch_url(&["../x".to_string()]).is_err());
        assert!(versions_batch_url(&[]).is_err());
    }

    #[test]
    fn hash_lookup_response() {
        let m = parse_version_files(r#"{"ABCDEF":{"id":"v1","project_id":"p"},"123":{"id":"../x"}}"#).unwrap();
        assert_eq!(m.get("abcdef").map(String::as_str), Some("v1"), "özet küçük harfe çevrilir");
        assert_eq!(m.len(), 1, "geçersiz kimlik atılır");
        assert!(parse_version_files("[]").is_err());
    }

    #[test]
    fn installs_mod_with_required_dependency_once() {
        let dir = std::env::temp_dir().join(format!("kazma-mr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (main, api) = (b"main-jar".to_vec(), b"api-jar".to_vec());
        let mut pages: HashMap<String, String> = HashMap::new();
        pages.insert(versions_url("MAIN", "1.21.1", Some("fabric")).unwrap(), version_json("main.jar", &main, &[("FAPI", "required"), ("MAIN", "required")]));
        pages.insert(versions_url("FAPI", "1.21.1", Some("fabric")).unwrap(), version_json("fabric-api.jar", &api, &[]));
        let gj = |u: &str| pages.get(u).cloned().ok_or(format!("beklenmeyen {u}"));
        let gf = |u: &str| if u.ends_with("main.jar") { Ok(main.clone()) } else { Ok(api.clone()) };
        let w = install_project(&dir, "MAIN", "1.21.1", Some("fabric"), Kind::Mod, &gj, &gf, &NullReporter).unwrap();
        assert_eq!(w, ["main.jar", "fabric-api.jar"]);
        assert!(install_project(&dir, "MAIN", "1.21.1", Some("fabric"), Kind::Mod, &gj, &gf, &NullReporter).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_resource_pack_without_loader_or_dependencies() {
        let dir = std::env::temp_dir().join(format!("kazma-rp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data = b"zip-bytes".to_vec();
        let page = version_json_ext("Faithful-64x.zip", &data, &[("SOMEMOD", "required")]);
        let asked = std::cell::RefCell::new(Vec::<String>::new());
        let gj = |u: &str| {
            asked.borrow_mut().push(u.to_string());
            Ok(page.clone())
        };
        let gf = |_: &str| Ok(data.clone());
        let w = install_project(&dir, "RP", "1.21.1", Some("fabric"), Kind::ResourcePack, &gj, &gf, &NullReporter).unwrap();
        assert_eq!(w, ["Faithful-64x.zip"]);
        assert_eq!(asked.borrow().len(), 1, "paketlerde bağımlılık kurulmaz");
        assert!(!asked.borrow()[0].contains("loaders"), "paket sürümü yükleyiciye göre filtrelenmez");
        assert!(dir.join("Faithful-64x.zip").is_file());
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
        assert!(install_project(&dir, "MAIN", "1.21.1", Some("fabric"), Kind::Mod, &gj, &evil, &NullReporter).unwrap_err().contains("SHA-512"));
        assert!(!dir.join("m.jar").exists());
        let none = |_: &str| Ok("[]".to_string());
        assert!(install_project(&dir, "MAIN", "1.21.1", Some("fabric"), Kind::Mod, &none, &evil, &NullReporter).unwrap_err().contains("uygun dosya yok"));
        let _ = fs::remove_dir_all(&dir);
    }
}
