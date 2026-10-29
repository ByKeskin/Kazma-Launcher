//! Manifest imzalama (Ed25519). Sunucu sahibi imzalar, istemci doğrular.
//! Açık anahtar manifestin içinde taşınır; güveni "ilk katılımda sabitleme" sağlar:
//! istemci anahtarı saklar, sonraki katılımlarda değişmişse uyarır ve reddeder.

use crate::manifest::PackManifest;
use crate::{Error, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

fn canonical(m: &PackManifest) -> Vec<u8> {
    let mut c = m.clone();
    c.signature = None;
    serde_json::to_vec(&c).expect("manifest JSON'a çevrilebilir")
}

pub fn public_key_hex(seed: &[u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

pub fn sign(m: &mut PackManifest, seed: &[u8; 32]) {
    let key = SigningKey::from_bytes(seed);
    m.signer = Some(hex::encode(key.verifying_key().to_bytes()));
    m.signature = None;
    let sig = key.sign(&canonical(m));
    m.signature = Some(hex::encode(sig.to_bytes()));
}

/// İmza geçerliyse manifestteki açık anahtarı (sabitlemek için) döndürür.
/// `pinned` verilmişse ve farklıysa reddeder.
pub fn verify(m: &PackManifest, pinned: Option<&str>) -> Result<String> {
    let signer = m.signer.as_deref().ok_or(Error::BadSignature)?;
    let sig_hex = m.signature.as_deref().ok_or(Error::BadSignature)?;
    if let Some(p) = pinned {
        if !p.eq_ignore_ascii_case(signer) {
            return Err(Error::BadSignature);
        }
    }
    let pk: [u8; 32] = hex::decode(signer).ok().and_then(|b| b.try_into().ok()).ok_or(Error::BadSignature)?;
    let sg: [u8; 64] = hex::decode(sig_hex).ok().and_then(|b| b.try_into().ok()).ok_or(Error::BadSignature)?;
    let vk = VerifyingKey::from_bytes(&pk).map_err(|_| Error::BadSignature)?;
    vk.verify(&canonical(m), &Signature::from_bytes(&sg)).map_err(|_| Error::BadSignature)?;
    Ok(signer.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Loader, ModEntry, Side};

    fn sample() -> PackManifest {
        PackManifest {
            name: "Dostlar SMP".into(), game: "1.21.1".into(),
            loader: Loader { kind: "fabric".into(), version: "0.16.9".into() },
            mods: vec![ModEntry {
                id: "sodium".into(), version: "0.6.0".into(), file: "sodium.jar".into(),
                sha512: "a".repeat(128), url: "https://cdn.modrinth.com/x.jar".into(), side: Side::Both, size: 1,
            }],
            signer: None, signature: None,
        }
    }

    #[test]
    fn sign_verify_and_tamper() {
        let seed = [7u8; 32];
        let mut m = sample();
        sign(&mut m, &seed);
        let key = verify(&m, None).unwrap();
        assert_eq!(key, public_key_hex(&seed));
        assert!(verify(&m, Some(&key)).is_ok(), "sabitlenen anahtarla geçer");

        // mod listesine sonradan zararlı giriş eklemek imzayı bozar
        let mut t = m.clone();
        t.mods[0].url = "https://evil.example/x.jar".into();
        assert_eq!(verify(&t, None), Err(Error::BadSignature));

        // başka anahtarla imzalanmış manifest, sabitlenen anahtara uymaz
        let mut other = sample();
        sign(&mut other, &[9u8; 32]);
        assert!(verify(&other, None).is_ok());
        assert_eq!(verify(&other, Some(&key)), Err(Error::BadSignature));
    }

    #[test]
    fn unsigned_is_rejected() {
        assert_eq!(verify(&sample(), None), Err(Error::BadSignature));
    }

    #[test]
    fn survives_json_roundtrip() {
        let mut m = sample();
        sign(&mut m, &[1u8; 32]);
        let back = crate::manifest::parse(&serde_json::to_string(&m).unwrap()).unwrap();
        assert!(verify(&back, None).is_ok());
    }
}
