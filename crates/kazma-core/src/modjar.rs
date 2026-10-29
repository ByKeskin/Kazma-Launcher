//! Bir mod jar'ının `fabric.mod.json` dosyasından kimlik ve çalışma tarafı (istemci/sunucu) okuma.

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
