//! Mojang `version_manifest_v2.json` okuma ve mod yükleyici profil adresleri.
//! Ağ isteği yapmaz; JSON metnini alır, ayrıştırır.

use crate::{Error, Result};
use serde::Deserialize;

pub const VERSION_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VersionRef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: VersionKind,
    pub url: String,
    pub sha1: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Deserialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<VersionRef>,
}

impl VersionManifest {
    pub fn releases(&self) -> Vec<&VersionRef> {
        self.versions.iter().filter(|v| v.kind == VersionKind::Release).collect()
    }
    pub fn find(&self, id: &str) -> Option<&VersionRef> {
        self.versions.iter().find(|v| v.id == id)
    }
}

pub fn parse_manifest(json: &str) -> Result<VersionManifest> {
    serde_json::from_str(json).map_err(|e| Error::BadManifest(e.to_string()))
}

fn plain(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

/// Fabric'in istemci profil JSON'u. Girdiler yol parçası olduğu için kısıtlı karakter kümesi şart.
pub fn fabric_profile_url(game: &str, loader: &str) -> Result<String> {
    if !plain(game) || !plain(loader) {
        return Err(Error::BadManifest("sürüm adı geçersiz karakter içeriyor".into()));
    }
    Ok(format!("https://meta.fabricmc.net/v2/versions/loader/{game}/{loader}/profile/json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "latest": {"release": "1.21.4", "snapshot": "25w02a"},
      "versions": [
        {"id":"25w02a","type":"snapshot","url":"https://piston-meta.mojang.com/v1/packages/aa/25w02a.json","time":"2025-01-08T12:00:00+00:00","releaseTime":"2025-01-08T12:00:00+00:00","sha1":"aa","complianceLevel":1},
        {"id":"1.21.4","type":"release","url":"https://piston-meta.mojang.com/v1/packages/bb/1.21.4.json","time":"2024-12-03T10:00:00+00:00","releaseTime":"2024-12-03T10:00:00+00:00","sha1":"bb","complianceLevel":1},
        {"id":"1.21.1","type":"release","url":"https://piston-meta.mojang.com/v1/packages/cc/1.21.1.json","time":"2024-08-08T12:00:00+00:00","releaseTime":"2024-08-08T12:00:00+00:00","sha1":"cc","complianceLevel":1},
        {"id":"b1.7.3","type":"old_beta","url":"https://example/b173.json","time":"2011-07-08T00:00:00+00:00","releaseTime":"2011-07-08T00:00:00+00:00","sha1":"dd"}
      ]}"#;

    #[test]
    fn parses_and_filters() {
        let m = parse_manifest(SAMPLE).unwrap();
        assert_eq!(m.latest.release, "1.21.4");
        let ids: Vec<_> = m.releases().iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["1.21.4", "1.21.1"]);
        assert_eq!(m.find("b1.7.3").unwrap().kind, VersionKind::OldBeta);
        assert!(m.find("9.9").is_none());
    }

    #[test]
    fn broken_json() {
        assert!(matches!(parse_manifest("{"), Err(Error::BadManifest(_))));
    }

    #[test]
    fn fabric_url() {
        assert_eq!(
            fabric_profile_url("1.21.1", "0.16.9").unwrap(),
            "https://meta.fabricmc.net/v2/versions/loader/1.21.1/0.16.9/profile/json"
        );
        assert!(fabric_profile_url("1.21/../x", "0.16.9").is_err());
        assert!(fabric_profile_url("1.21.1", "").is_err());
    }
}
