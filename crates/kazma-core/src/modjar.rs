//! Bir mod jar'ının üst verisinden (Fabric, Quilt, Forge, NeoForge) kimlik ve çalışma tarafı okuma.

use crate::manifest::{LocalMod, Side};
use crate::{Error, Result};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq)]
pub struct ModMeta {
    pub id: String,
    pub version: Option<String>,
    pub side: Side,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    environment: Option<String>,
}

pub fn parse_fabric_mod_json(json: &str) -> Result<ModMeta> {
    let r: Raw = serde_json::from_str(json).map_err(|e| Error::BadManifest(format!("fabric.mod.json okunamadı: {e}")))?;
    if r.id.is_empty() || r.id.len() > 64 || !r.id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
        return Err(Error::BadManifest(format!("mod kimliği geçersiz: {}", r.id)));
    }
    let side = match r.environment.as_deref() {
        Some("client") => Side::Client,
        Some("server") => Side::Server,
        _ => Side::Both,
    };
    Ok(ModMeta { id: r.id, version: r.version, side })
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub fn parse_quilt_mod_json(json: &str) -> Result<ModMeta> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| Error::BadManifest(format!("quilt.mod.json okunamadı: {e}")))?;
    let id = v.pointer("/quilt_loader/id").and_then(|x| x.as_str()).unwrap_or("");
    if !id_ok(id) {
        return Err(Error::BadManifest(format!("mod kimliği geçersiz: {id}")));
    }
    let side = match v.pointer("/minecraft/environment").and_then(|x| x.as_str()) {
        Some("client") => Side::Client,
        Some("dedicated_server") => Side::Server,
        _ => Side::Both,
    };
    let version = v.pointer("/quilt_loader/version").and_then(|x| x.as_str()).map(String::from);
    Ok(ModMeta { id: id.into(), version, side })
}

/// `META-INF/mods.toml` (Forge) ve `META-INF/neoforge.mods.toml` (NeoForge): ilk `modId` ve `version`.
/// Tam TOML çözümleyicisi kullanmıyoruz; yalnızca düz `anahtar = "değer"` satırlarına bakıyoruz.
pub fn parse_mods_toml(text: &str) -> Result<ModMeta> {
    let val = |key: &str| -> Option<String> {
        text.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| {
            let (k, v) = l.split_once('=')?;
            if k.trim() != key {
                return None;
            }
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            Some(v.split('"').next().unwrap_or("").to_string())
        })
    };
    let id = val("modId").unwrap_or_default();
    if !id_ok(&id) {
        return Err(Error::BadManifest(format!("mod kimliği geçersiz: {id}")));
    }
    Ok(ModMeta { id, version: val("version"), side: Side::Both })
}

/// Jar içindeki dosyaları `read` ile okuyup hangi yükleyicinin modu olduğunu bulur.
pub fn identify(read: &dyn Fn(&str) -> Option<String>) -> Option<ModMeta> {
    if let Some(t) = read("fabric.mod.json") {
        if let Ok(m) = parse_fabric_mod_json(&t) {
            return Some(m);
        }
    }
    if let Some(t) = read("quilt.mod.json") {
        if let Ok(m) = parse_quilt_mod_json(&t) {
            return Some(m);
        }
    }
    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(t) = read(name) {
            if let Ok(m) = parse_mods_toml(&t) {
                return Some(m);
            }
        }
    }
    None
}

pub fn to_local(file: &str, sha512: &str, meta: &ModMeta) -> LocalMod {
    LocalMod { id: meta.id.clone(), file: file.into(), sha512: sha512.into(), client_only: meta.side == Side::Client }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides() {
        let m = parse_fabric_mod_json(r#"{"schemaVersion":1,"id":"sodium","version":"0.6.0","environment":"client","name":"Sodium"}"#).unwrap();
        assert_eq!((m.id.as_str(), m.side), ("sodium", Side::Client));
        assert_eq!(parse_fabric_mod_json(r#"{"id":"create","environment":"*"}"#).unwrap().side, Side::Both);
        assert_eq!(parse_fabric_mod_json(r#"{"id":"x"}"#).unwrap().side, Side::Both);
        assert_eq!(parse_fabric_mod_json(r#"{"id":"srv","environment":"server"}"#).unwrap().side, Side::Server);
    }

    #[test]
    fn quilt_and_toml() {
        let q = parse_quilt_mod_json(r#"{"schema_version":1,"quilt_loader":{"id":"qsl","version":"1.0"},"minecraft":{"environment":"client"}}"#).unwrap();
        assert_eq!((q.id.as_str(), q.side), ("qsl", Side::Client));
        assert_eq!(parse_quilt_mod_json(r#"{"quilt_loader":{"id":"x"},"minecraft":{"environment":"dedicated_server"}}"#).unwrap().side, Side::Server);
        let toml = "modLoader=\"javafml\"\nloaderVersion=\"[47,)\"\n# modId=\"yorum\"\n[[mods]]\nmodId=\"create\"\nversion=\"${file.jarVersion}\"\ndisplayName=\"Create\"\n";
        let m = parse_mods_toml(toml).unwrap();
        assert_eq!(m.id, "create");
        assert!(parse_mods_toml("[[mods]]\nmodId=\"../x\"\n").is_err());
        assert!(parse_mods_toml("nothing").is_err());
    }

    #[test]
    fn identify_tries_each_loader() {
        let only = |name: &'static str, body: &'static str| move |n: &str| if n == name { Some(body.to_string()) } else { None };
        assert_eq!(identify(&only("fabric.mod.json", r#"{"id":"a"}"#)).unwrap().id, "a");
        assert_eq!(identify(&only("quilt.mod.json", r#"{"quilt_loader":{"id":"b"}}"#)).unwrap().id, "b");
        assert_eq!(identify(&only("META-INF/mods.toml", "modId=\"c\"")).unwrap().id, "c");
        assert_eq!(identify(&only("META-INF/neoforge.mods.toml", "modId=\"d\"")).unwrap().id, "d");
        assert!(identify(&|_| None).is_none());
    }

    #[test]
    fn rejects_bad() {
        assert!(parse_fabric_mod_json("{").is_err());
        assert!(parse_fabric_mod_json(r#"{"id":"../x"}"#).is_err());
        assert!(parse_fabric_mod_json(r#"{"id":""}"#).is_err());
    }

    #[test]
    fn local_mod_client_flag() {
        let m = parse_fabric_mod_json(r#"{"id":"iris","environment":"client"}"#).unwrap();
        let l = to_local("iris.jar", "ab", &m);
        assert!(l.client_only);
        assert_eq!(l.file, "iris.jar");
    }
}
