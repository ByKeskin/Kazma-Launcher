//! Server'ın yayınladığı mod manifesti ve istemcide eşitleme planı.
//!
//! Güvenlik kuralları (hepsi `validate` içinde):
//!  * dosya adı düz bir `.jar` adı olmalı (`..`, `/`, `\` yok): yol atlatma saldırısını keser,
//!  * indirme adresi https olmalı ve izinli bir sunucudan gelmeli,
//!  * özet (SHA-512) 128 onaltılık karakter olmalı.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Both,
    Client,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loader {
    pub kind: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModEntry {
    pub id: String,
    pub version: String,
    pub file: String,
    pub sha512: String,
    pub url: String,
    pub side: Side,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub name: String,
    pub game: String,
    pub loader: Loader,
    pub mods: Vec<ModEntry>,
    /// Ed25519 açık anahtarı (hex). Sunucu sahibine aittir; ilk katılımda istemci sabitler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    /// `signature` alanı çıkarılmış manifestin JSON baytları üzerinde Ed25519 imzası (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

pub const TRUSTED_HOSTS: &[&str] = &["cdn.modrinth.com", "edge.forgecdn.net", "mediafilez.forgecdn.net"];

pub fn safe_file_name(f: &str) -> bool {
    !f.is_empty()
        && f.len() <= 128
        && !f.contains("..")
        && !f.contains('/')
        && !f.contains('\\')
        && !f.contains(':')
        && f.to_ascii_lowercase().ends_with(".jar")
        && f.chars().all(|c| !c.is_control())
}

/// `extra_host`: davet edilen server'ın kendi adı. Server kendi barındırdığı modları buradan sunabilir.
pub fn url_trusted(url: &str, extra_host: Option<&str>) -> bool {
    url_trusted_for(url, extra_host.as_slice())
}

/// `extra_hosts`: server'ın kendi alan adı ve manifesti sunan alan adı gibi ek izinli adresler.
pub fn url_trusted_for(url: &str, extra_hosts: &[&str]) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    if u.scheme() != "https" {
        return false;
    }
    let Some(host) = u.host_str() else { return false };
    TRUSTED_HOSTS.contains(&host) || extra_hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
}

pub fn parse(json: &str) -> Result<PackManifest> {
    serde_json::from_str(json).map_err(|e| Error::BadManifest(e.to_string()))
}

impl PackManifest {
    pub fn validate(&self, extra_host: Option<&str>) -> Result<()> {
        self.validate_with(extra_host.as_slice())
    }

    pub fn validate_with(&self, extra_hosts: &[&str]) -> Result<()> {
        for m in &self.mods {
            if !safe_file_name(&m.file) {
                return Err(Error::BadManifest(format!("güvensiz dosya adı: {}", m.file)));
            }
            if m.sha512.len() != 128 || !m.sha512.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::BadManifest(format!("{}: SHA-512 özeti geçersiz", m.id)));
            }
            if m.side != Side::Server && !url_trusted_for(&m.url, extra_hosts) {
                return Err(Error::UntrustedUrl(m.url.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMod {
    pub id: String,
    pub file: String,
    pub sha512: String,
    /// Yalnızca istemci modu (örn. Iris): server'da olmaması normaldir, silinmez.
    pub client_only: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    /// Aynı id ve aynı özet: dokunulmaz.
    pub ready: Vec<String>,
    /// Hiç yok: indirilecek.
    pub download: Vec<ModEntry>,
    /// Var ama özet farklı: eskisi devre dışı, yenisi inecek.
    pub update: Vec<(LocalMod, ModEntry)>,
    /// Manifestte olmayan ve istemciye özel de olmayan: devre dışı bırakılacak.
    pub disable: Vec<LocalMod>,
    /// Manifestte olmayan ama istemciye özel: yerelde kalır.
    pub keep: Vec<LocalMod>,
}

impl SyncPlan {
    pub fn download_bytes(&self) -> u64 {
        self.download.iter().map(|m| m.size).sum::<u64>() + self.update.iter().map(|(_, m)| m.size).sum::<u64>()
    }
    pub fn is_noop(&self) -> bool {
        self.download.is_empty() && self.update.is_empty() && self.disable.is_empty()
    }
}

pub fn plan(manifest: &PackManifest, local: &[LocalMod]) -> SyncPlan {
    let mut out = SyncPlan::default();
    let mut used = vec![false; local.len()];

    for want in manifest.mods.iter().filter(|m| m.side != Side::Server) {
        // aynı id'li, henüz eşleşmemiş ilk yerel dosya
        let hit = local.iter().enumerate().find(|(i, l)| !used[*i] && l.id == want.id);
        match hit {
            Some((i, l)) => {
                used[i] = true;
                if l.sha512.eq_ignore_ascii_case(&want.sha512) {
                    out.ready.push(want.id.clone());
                } else {
                    out.update.push((l.clone(), want.clone()));
                }
            }
            None => out.download.push(want.clone()),
        }
    }
    for (i, l) in local.iter().enumerate().filter(|(i, _)| !used[*i]) {
        let _ = i;
        if l.client_only && !manifest.mods.iter().any(|m| m.id == l.id) {
            out.keep.push(l.clone());
        } else {
            out.disable.push(l.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: char) -> String {
        std::iter::repeat(c).take(128).collect()
    }
    pub(crate) fn entry(id: &str, hash: char, side: Side) -> ModEntry {
        ModEntry {
            id: id.into(), version: "1.0".into(), file: format!("{id}.jar"), sha512: h(hash),
            url: format!("https://cdn.modrinth.com/data/{id}/versions/1/{id}.jar"), side, size: 1_000_000,
        }
    }
    pub(crate) fn pack(mods: Vec<ModEntry>) -> PackManifest {
        PackManifest {
            name: "Dostlar SMP".into(), game: "1.21.1".into(),
            loader: Loader { kind: "fabric".into(), version: "0.16.9".into() },
            mods, signer: None, signature: None,
        }
    }
    fn local(id: &str, hash: char, client_only: bool) -> LocalMod {
        LocalMod { id: id.into(), file: format!("{id}.jar"), sha512: h(hash), client_only }
    }

    #[test]
    fn plan_covers_every_case() {
        let m = pack(vec![
            entry("sodium", 'a', Side::Both),
            entry("create", 'b', Side::Both),
            entry("lithium", 'c', Side::Both),
            entry("serveronly", 'd', Side::Server),
        ]);
        let l = vec![
            local("sodium", 'a', false),          // hazır
            local("lithium", '0', false),         // güncelle
            local("iris", 'e', true),             // istemciye özel, kalır
            local("optifine", 'f', false),        // manifestte yok, devre dışı
        ];
        let p = plan(&m, &l);
        assert_eq!(p.ready, ["sodium"]);
        assert_eq!(p.download.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["create"]);
        assert_eq!(p.update.len(), 1);
        assert_eq!(p.update[0].1.id, "lithium");
        assert_eq!(p.keep[0].id, "iris");
        assert_eq!(p.disable[0].id, "optifine");
        assert_eq!(p.download_bytes(), 2_000_000);
        assert!(!p.is_noop());
    }

    #[test]
    fn identical_state_is_noop() {
        let m = pack(vec![entry("sodium", 'a', Side::Both)]);
        assert!(plan(&m, &[local("sodium", 'a', false)]).is_noop());
    }

    #[test]
    fn duplicate_local_copy_is_disabled() {
        let m = pack(vec![entry("sodium", 'a', Side::Both)]);
        let p = plan(&m, &[local("sodium", 'a', false), local("sodium", '9', false)]);
        assert_eq!(p.ready, ["sodium"]);
        assert_eq!(p.disable.len(), 1);
    }

    #[test]
    fn rejects_path_traversal() {
        for f in ["../evil.jar", "a/b.jar", "a\\b.jar", "C:evil.jar", "x.exe", "", ".."] {
            let mut e = entry("x", 'a', Side::Both);
            e.file = f.into();
            assert!(pack(vec![e]).validate(None).is_err(), "{f:?} kabul edilmemeli");
        }
        assert!(pack(vec![entry("x", 'a', Side::Both)]).validate(None).is_ok());
    }

    #[test]
    fn url_policy() {
        assert!(url_trusted("https://cdn.modrinth.com/data/x.jar", None));
        assert!(!url_trusted("http://cdn.modrinth.com/data/x.jar", None), "http reddedilir");
        assert!(!url_trusted("https://evil.example/x.jar", None));
        assert!(!url_trusted("https://cdn.modrinth.com.evil.example/x.jar", None), "alan adı sonek hilesi");
        assert!(url_trusted("https://play.example.com/mods/x.jar", Some("play.example.com")));
        assert!(!url_trusted("not a url", None));
        let mut e = entry("x", 'a', Side::Both);
        e.url = "https://evil.example/x.jar".into();
        assert!(matches!(pack(vec![e]).validate(None), Err(Error::UntrustedUrl(_))));
    }

    #[test]
    fn multiple_extra_hosts() {
        let mut e = entry("x", 'a', Side::Both);
        e.url = "https://files.example.org/x.jar".into();
        let m = pack(vec![e]);
        assert!(m.validate_with(&["play.example.com", "files.example.org"]).is_ok());
        assert!(m.validate_with(&["play.example.com"]).is_err());
    }

    #[test]
    fn bad_hash_rejected() {
        let mut e = entry("x", 'a', Side::Both);
        e.sha512 = "zz".into();
        assert!(pack(vec![e]).validate(None).is_err());
    }

    #[test]
    fn json_roundtrip() {
        let m = pack(vec![entry("sodium", 'a', Side::Client)]);
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"side\":\"client\""));
        assert_eq!(parse(&j).unwrap(), m);
        assert!(parse("{").is_err());
    }
}
