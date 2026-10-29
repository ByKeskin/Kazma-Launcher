//! Hangi dosyaların indirileceğini, sınıf yolunu ve çıkarılacak yerel (native) jar'ları hesaplar.
//! Tüm yollar `root` (örn. `%APPDATA%\.kazma`) altına göredir. Sunucudan gelen her yol ve adres
//! burada denetlenir: dosya sistemine yazılacak yol kaçamaz, adres izinli bir Mojang/mod yükleyici
//! sunucusundan olmalıdır.

use crate::version::{Env, Resolved};
use crate::{Error, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use url::Url;

pub const TRUSTED_GAME_HOSTS: &[&str] = &[
    "piston-meta.mojang.com",
    "piston-data.mojang.com",
    "launchermeta.mojang.com",
    "launcher.mojang.com",
    "libraries.minecraft.net",
    "resources.download.minecraft.net",
    "meta.fabricmc.net",
    "maven.fabricmc.net",
    "maven.neoforged.net",
    "maven.minecraftforge.net",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    pub url: String,
    pub rel_path: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeJar {
    pub rel_path: String,
    pub exclude: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub downloads: Vec<DownloadItem>,
    /// `libraries/...` yolları, istemci jar'ı en sonda.
    pub classpath: Vec<String>,
    pub natives: Vec<NativeJar>,
    pub client_jar: String,
    pub asset_index_path: String,
}

pub fn safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 260
        && !p.starts_with('/')
        && !p.contains("..")
        && !p.contains('\\')
        && !p.contains(':')
        && !p.chars().any(|c| c.is_control())
}

fn safe_component(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && !s.contains(['/', '\\', ':']) && !s.contains("..") && !s.chars().any(|c| c.is_control())
}

pub fn game_url_trusted(url: &str) -> bool {
    match Url::parse(url) {
        Ok(u) => u.scheme() == "https" && u.host_str().map_or(false, |h| TRUSTED_GAME_HOSTS.contains(&h)),
        Err(_) => false,
    }
}

fn item(url: &str, rel_path: String, sha1: &Option<String>, size: Option<u64>) -> Result<DownloadItem> {
    if !safe_rel_path(&rel_path) {
        return Err(Error::BadManifest(format!("güvensiz dosya yolu: {rel_path}")));
    }
    if !game_url_trusted(url) {
        return Err(Error::UntrustedUrl(url.to_string()));
    }
    Ok(DownloadItem { url: url.to_string(), rel_path, sha1: sha1.clone(), size })
}

pub fn build_plan(r: &Resolved, env: &Env) -> Result<Plan> {
    if !safe_component(&r.jar_id) || !safe_component(&r.asset_index.id) {
        return Err(Error::BadManifest("sürüm kimliği geçersiz karakter içeriyor".into()));
    }
    let mut p = Plan::default();
    let mut seen_dl = HashSet::new();
    let mut seen_cp = HashSet::new();
    let mut push_dl = |p: &mut Plan, d: DownloadItem| {
        if seen_dl.insert(d.rel_path.clone()) {
            p.downloads.push(d);
        }
    };

    for lib in r.libraries.iter().filter(|l| l.allowed(env)) {
        if let Some(a) = lib.artifact() {
            let rel = format!("libraries/{}", a.path);
            push_dl(&mut p, item(&a.url, rel.clone(), &a.sha1, a.size)?);
            if seen_cp.insert(rel.clone()) {
                p.classpath.push(rel.clone());
            }
            if lib.is_modern_native() {
                p.natives.push(NativeJar { rel_path: rel, exclude: vec!["META-INF/".into()] });
            }
        }
        if let Some(n) = lib.legacy_native(env) {
            let rel = format!("libraries/{}", n.path);
            push_dl(&mut p, item(&n.url, rel.clone(), &n.sha1, n.size)?);
            let exclude = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_else(|| vec!["META-INF/".into()]);
            p.natives.push(NativeJar { rel_path: rel, exclude });
        }
    }

    let client = r.client.as_ref().ok_or_else(|| Error::BadManifest("istemci jar bilgisi yok".into()))?;
    p.client_jar = format!("versions/{0}/{0}.jar", r.jar_id);
    let jar = p.client_jar.clone();
    push_dl(&mut p, item(&client.url, jar, &client.sha1, client.size)?);
    p.classpath.push(p.client_jar.clone());

    p.asset_index_path = format!("assets/indexes/{}.json", r.asset_index.id);
    let idx = p.asset_index_path.clone();
    push_dl(&mut p, item(&r.asset_index.url, idx, &r.asset_index.sha1, r.asset_index.size)?);
    Ok(p)
}

#[derive(Deserialize)]
struct AssetFile {
    objects: HashMap<String, AssetObj>,
}
#[derive(Deserialize)]
struct AssetObj {
    hash: String,
    size: u64,
}

/// Asset indeksindeki nesneler: `assets/objects/ab/abcdef...`. Aynı özet tek kez indirilir.
pub fn asset_items(index_json: &str) -> Result<Vec<DownloadItem>> {
    let f: AssetFile = serde_json::from_str(index_json).map_err(|e| Error::BadManifest(e.to_string()))?;
    let mut by_hash: Vec<(&String, u64)> = f.objects.values().map(|o| (&o.hash, o.size)).collect();
    by_hash.sort();
    by_hash.dedup_by(|a, b| a.0 == b.0);
    by_hash
        .into_iter()
        .map(|(h, size)| {
            if h.len() != 40 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::BadManifest(format!("asset özeti geçersiz: {h}")));
            }
            let h = h.to_ascii_lowercase();
            Ok(DownloadItem {
                url: format!("https://resources.download.minecraft.net/{}/{}", &h[..2], h),
                rel_path: format!("assets/objects/{}/{}", &h[..2], h),
                sha1: Some(h),
                size: Some(size),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{fixtures::*, parse, resolve};

    fn vanilla() -> Resolved {
        resolve(parse(VANILLA).unwrap(), None).unwrap()
    }

    #[test]
    fn windows_plan() {
        let p = build_plan(&vanilla(), &Env::windows_x64()).unwrap();
        let paths: Vec<_> = p.downloads.iter().map(|d| d.rel_path.as_str()).collect();
        assert!(paths.contains(&"libraries/com/mojang/logging/1.2.7/logging-1.2.7.jar"));
        assert!(paths.contains(&"libraries/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"));
        assert!(!paths.iter().any(|p| p.contains("objc-bridge") || p.contains("natives-linux") || p.contains("x86")));
        assert!(paths.contains(&"versions/1.21.1/1.21.1.jar"));
        assert!(paths.contains(&"assets/indexes/17.json"));
        assert_eq!(p.classpath.last().unwrap(), "versions/1.21.1/1.21.1.jar", "istemci jar'ı sonda");
        assert_eq!(p.natives.len(), 1);
        assert_eq!(p.natives[0].exclude, ["META-INF/"]);
    }

    #[test]
    fn linux_plan_differs() {
        let env = Env { os: crate::version::Os::Linux, ..Env::windows_x64() };
        let p = build_plan(&vanilla(), &env).unwrap();
        assert!(p.downloads.iter().any(|d| d.rel_path.contains("natives-linux")));
        assert!(!p.downloads.iter().any(|d| d.rel_path.contains("natives-windows")));
    }

    #[test]
    fn fabric_plan_has_maven_libs() {
        let r = resolve(parse(FABRIC).unwrap(), Some(parse(VANILLA).unwrap())).unwrap();
        let p = build_plan(&r, &Env::windows_x64()).unwrap();
        assert!(p.downloads.iter().any(|d| d.url == "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"));
        assert_eq!(p.client_jar, "versions/1.21.1/1.21.1.jar", "jar üst sürümden gelir");
    }

    #[test]
    fn legacy_plan_extracts_natives() {
        let r = resolve(parse(LEGACY).unwrap(), None).unwrap();
        let p = build_plan(&r, &Env::windows_x64()).unwrap();
        assert_eq!(p.natives.len(), 1);
        assert!(p.natives[0].rel_path.ends_with("natives-windows.jar"));
        assert!(!p.classpath.iter().any(|c| c.contains("natives-windows")), "yerel jar sınıf yoluna girmez");
    }

    #[test]
    fn rejects_untrusted_host_and_bad_paths() {
        let mut v = parse(VANILLA).unwrap();
        v.libraries[0].downloads.as_mut().unwrap().artifact.as_mut().unwrap().url = "https://evil.example/x.jar".into();
        assert!(matches!(build_plan(&resolve(v, None).unwrap(), &Env::windows_x64()), Err(Error::UntrustedUrl(_))));

        let mut v = parse(VANILLA).unwrap();
        v.libraries[0].downloads.as_mut().unwrap().artifact.as_mut().unwrap().path = Some("../../evil.jar".into());
        assert!(build_plan(&resolve(v, None).unwrap(), &Env::windows_x64()).is_err());

        let mut v = parse(VANILLA).unwrap();
        v.libraries[0].downloads.as_mut().unwrap().artifact.as_mut().unwrap().url =
            "http://libraries.minecraft.net/a.jar".into();
        assert!(build_plan(&resolve(v, None).unwrap(), &Env::windows_x64()).is_err(), "http reddedilir");

        let mut v = parse(VANILLA).unwrap();
        v.id = "..\\x".into();
        assert!(build_plan(&resolve(v, None).unwrap(), &Env::windows_x64()).is_err());
    }

    #[test]
    fn asset_objects() {
        let h1 = "a".repeat(40);
        let h2 = "b".repeat(40);
        let json = format!(
            r#"{{"objects":{{"icons/a.png":{{"hash":"{h1}","size":5}},"icons/b.png":{{"hash":"{h2}","size":7}},"copy.png":{{"hash":"{h1}","size":5}}}}}}"#
        );
        let items = asset_items(&json).unwrap();
        assert_eq!(items.len(), 2, "aynı özet bir kez");
        assert_eq!(items[0].rel_path, format!("assets/objects/aa/{h1}"));
        assert_eq!(items[0].url, format!("https://resources.download.minecraft.net/aa/{h1}"));
        assert!(asset_items(r#"{"objects":{"x":{"hash":"../../zz","size":1}}}"#).is_err());
    }
}
