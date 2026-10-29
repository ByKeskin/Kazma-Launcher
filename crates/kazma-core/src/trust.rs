//! İlk katılımda sabitleme (TOFU): bir server'ın imza anahtarı ilk görüldüğünde saklanır.
//! Sonradan anahtar değişirse (sunucu ele geçirilmiş ya da araya biri girmiş olabilir) katılım reddedilir.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustStore {
    #[serde(default)]
    pub servers: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pin {
    /// Bu server ilk kez görülüyor; kullanıcıya parmak izi gösterilip onay alınmalı.
    New,
    Same,
}

impl TrustStore {
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| Error::BadManifest(format!("güven deposu okunamadı: {e}")))
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("seri hale gelir")
    }
    pub fn check(&self, server: &str, key_hex: &str) -> Result<Pin> {
        match self.servers.get(server) {
            None => Ok(Pin::New),
            Some(k) if k.eq_ignore_ascii_case(key_hex) => Ok(Pin::Same),
            Some(_) => Err(Error::BadSignature),
        }
    }
    pub fn pin(&mut self, server: &str, key_hex: &str) {
        self.servers.insert(server.to_string(), key_hex.to_ascii_lowercase());
    }
}

/// Kullanıcıya gösterilecek kısa parmak izi: `ab12-cd34-ef56-7890`.
pub fn fingerprint(key_hex: &str) -> String {
    let d = Sha256::digest(key_hex.to_ascii_lowercase().as_bytes());
    let h = hex::encode(&d[..8]);
    h.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_flow() {
        let mut t = TrustStore::default();
        assert_eq!(t.check("a.com:25565", "AA"), Ok(Pin::New));
        t.pin("a.com:25565", "AA");
        assert_eq!(t.check("a.com:25565", "aa"), Ok(Pin::Same), "büyük/küçük harf fark etmez");
        assert_eq!(t.check("a.com:25565", "bb"), Err(Error::BadSignature), "anahtar değişti");
        assert_eq!(t.check("b.com:25565", "bb"), Ok(Pin::New), "başka server bağımsız");
    }

    #[test]
    fn json_roundtrip() {
        let mut t = TrustStore::default();
        t.pin("a.com:25565", "ab12");
        assert_eq!(TrustStore::parse(&t.to_json()).unwrap(), t);
        assert_eq!(TrustStore::parse("{}").unwrap(), TrustStore::default());
        assert!(TrustStore::parse("[").is_err());
    }

    #[test]
    fn fingerprint_shape() {
        let f = fingerprint("00ff");
        assert_eq!(f.len(), 19);
        assert_eq!(f.matches('-').count(), 3);
        assert_eq!(f, fingerprint("00FF"));
        assert_ne!(f, fingerprint("00fe"));
    }
}
