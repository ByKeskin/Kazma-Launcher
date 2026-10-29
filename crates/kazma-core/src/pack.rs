//! Paket kodu: server sahibinin mod listesini, hiçbir yere yüklemeden tek bir kısa metin olarak paylaşması.
//! Kod yalnızca Modrinth sürüm kimliklerini taşır; indirme adresleri ve SHA-512 özetleri oyuncunun
//! cihazında Modrinth'ten alınır. Kod imzasızdır: güveni, onu kimden aldığına dayanır.

use crate::instance::LoaderSpec;
use crate::invite;
use crate::manifest::{LocalMod, ModEntry, SyncPlan};
use crate::mojang::plain;
use crate::{Error, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};

pub const PREFIX: &str = "kazma://pack/";
const MAX_MODS: usize = 500;
const MAX_JSON: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackCode {
    pub v: u8,
    /// server adı
    pub n: String,
    /// server adresi `host[:port]`
    pub a: String,
    /// Minecraft sürümü
    pub g: String,
    /// yükleyici türü
    pub l: String,
    /// yükleyici sürümü
    #[serde(default)]
    pub lv: String,
    /// Modrinth sürüm kimlikleri
    pub m: Vec<String>,
}

pub fn is_pack_code(text: &str) -> bool {
    text.trim().starts_with(PREFIX)
}

fn version_id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn validate(p: &PackCode) -> Result<()> {
    let bad = |m: &str| Err(Error::BadManifest(format!("paket kodu geçersiz: {m}")));
    if p.v != 1 {
        return bad("sürüm desteklenmiyor");
    }
    if p.n.is_empty() || p.n.chars().count() > 60 || p.n.chars().any(|c| c.is_control()) {
        return bad("server adı");
    }
    if !p.a.is_empty() && invite::parse(&p.a).is_err() {
        return bad("server adresi");
    }
    if !plain(&p.g) {
        return bad("Minecraft sürümü");
    }
    LoaderSpec::from_kind(&p.l, &p.lv).map_err(|e| Error::BadManifest(format!("paket kodu geçersiz: {e}")))?;
    if p.m.is_empty() || p.m.len() > MAX_MODS {
        return bad("mod sayısı");
    }
    let mut seen = HashSet::new();
    if !p.m.iter().all(|id| version_id_ok(id) && seen.insert(id.clone())) {
        return bad("mod kimliği");
    }
    Ok(())
}

pub fn encode(p: &PackCode) -> Result<String> {
    validate(p)?;
    let json = serde_json::to_vec(p).map_err(|e| Error::BadManifest(e.to_string()))?;
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(&json).map_err(|e| Error::BadManifest(e.to_string()))?;
    let z = enc.finish().map_err(|e| Error::BadManifest(e.to_string()))?;
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z)))
}

pub fn decode(text: &str) -> Result<PackCode> {
    let t: String = text.split_whitespace().collect();
    let body = t.strip_prefix(PREFIX).unwrap_or(&t);
    let z = URL_SAFE_NO_PAD.decode(body).map_err(|_| Error::BadManifest("paket kodu bozuk (eksik ya da fazladan karakter)".into()))?;
    let mut json = Vec::new();
    // sıkıştırma bombasına karşı üst sınır
    DeflateDecoder::new(&z[..]).take(MAX_JSON + 1).read_to_end(&mut json).map_err(|_| Error::BadManifest("paket kodu bozuk".into()))?;
    if json.len() as u64 > MAX_JSON {
        return Err(Error::BadManifest("paket kodu çok büyük".into()));
    }
    let p: PackCode = serde_json::from_slice(&json).map_err(|_| Error::BadManifest("paket kodu bozuk".into()))?;
    validate(&p)?;
    Ok(p)
}

/// Kodda dosya adı yok, yalnızca özet var: yerel dosyalar özetlerine göre eşleştirilir.
/// Pakette olmayan modlar devre dışı bırakılır; istemciye özel olanlar (örn. Iris) yerinde kalır.
pub fn plan_by_hash(mods: &[ModEntry], local: &[LocalMod]) -> SyncPlan {
    let have: HashSet<String> = local.iter().map(|l| l.sha512.to_ascii_lowercase()).collect();
    let want: HashSet<String> = mods.iter().map(|m| m.sha512.to_ascii_lowercase()).collect();
    let mut p = SyncPlan::default();
    for m in mods {
        if have.contains(&m.sha512.to_ascii_lowercase()) {
            p.ready.push(m.id.clone());
        } else {
            p.download.push(m.clone());
        }
    }
    for l in local {
        if want.contains(&l.sha512.to_ascii_lowercase()) {
            continue;
        }
        if l.client_only {
            p.keep.push(l.clone());
        } else {
            p.disable.push(l.clone());
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Side;

    fn sample(n: usize) -> PackCode {
        PackCode {
            v: 1,
            n: "Dostlar SMP".into(),
            a: "my_world.aternos.me:12345".into(),
            g: "1.20.1".into(),
            l: "forge".into(),
            lv: "47.3.0".into(),
            m: (0..n).map(|i| format!("Ab{:06}", i * 7919 % 999_983)).collect(),
        }
    }

    #[test]
    fn roundtrip_and_size() {
        let p = sample(60);
        let c = encode(&p).unwrap();
        assert!(c.starts_with("kazma://pack/"));
        assert_eq!(decode(&c).unwrap(), p);
        assert!(c.len() < 1200, "60 mod bir sohbet mesajına sığmalı, uzunluk {}", c.len());
        // boşluk, satır sonu ve öneksiz yapıştırma da çalışır
        let spaced = format!("  {}\n", &c[..40]) + &c[40..];
        assert_eq!(decode(&spaced).unwrap(), p);
        assert_eq!(decode(c.trim_start_matches(PREFIX)).unwrap(), p);
        assert!(is_pack_code(&format!(" {c}")));
    }

    #[test]
    fn rejects_bad_codes() {
        assert!(decode("kazma://pack/!!!").is_err());
        assert!(decode("kazma://pack/").is_err());
        assert!(decode("kazma://pack/AAAA").is_err());
        let mut p = sample(3);
        p.l = "magic".into();
        assert!(encode(&p).is_err());
        let mut p = sample(3);
        p.m[0] = "../x".into();
        assert!(encode(&p).is_err());
        let mut p = sample(3);
        p.m[1] = p.m[0].clone();
        assert!(encode(&p).is_err(), "yinelenen kimlik");
        let mut p = sample(3);
        p.a = "bad host".into();
        assert!(encode(&p).is_err());
        let mut p = sample(3);
        p.m.clear();
        assert!(encode(&p).is_err());
        let mut p = sample(3);
        p.lv = String::new();
        assert!(encode(&p).is_err(), "forge için sürüm şart");
        let mut p = sample(3);
        p.n = "x\u{0}y".into();
        assert!(encode(&p).is_err());
    }

    #[test]
    fn decompression_bomb_is_capped() {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&vec![b'a'; 3 << 20]).unwrap();
        let z = enc.finish().unwrap();
        let code = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z));
        assert!(decode(&code).unwrap_err().to_string().contains("çok büyük"));
    }

    fn entry(id: &str, sha: char) -> ModEntry {
        ModEntry { id: id.into(), version: "1".into(), file: format!("{id}.jar"), sha512: sha.to_string().repeat(128), url: "https://cdn.modrinth.com/x.jar".into(), side: Side::Both, size: 5 }
    }
    fn local(id: &str, sha: char, client: bool) -> LocalMod {
        LocalMod { id: id.into(), file: format!("{id}.jar"), sha512: sha.to_string().repeat(128), client_only: client }
    }

    #[test]
    fn hash_plan() {
        let pack = vec![entry("create", 'a'), entry("lithium", 'b'), entry("jei", 'c')];
        let loc = vec![local("create", 'a', false), local("lithium", 'x', false), local("iris", 'i', true), local("optifine", 'o', false)];
        let p = plan_by_hash(&pack, &loc);
        assert_eq!(p.ready, ["create"]);
        assert_eq!(p.download.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["lithium", "jei"]);
        assert_eq!(p.keep.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["iris"]);
        assert_eq!(p.disable.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["lithium", "optifine"], "eski lithium dosyası kenara alınır");
        assert!(p.update.is_empty());
        assert!(plan_by_hash(&pack, &[local("a", 'a', false), local("b", 'b', false), local("c", 'c', false)]).is_noop());
    }
}
