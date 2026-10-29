//! Alternatif (çevrimdışı) hesaplar. Çevrimdışı UUID, Java'daki
//! `UUID.nameUUIDFromBytes(("OfflinePlayer:" + ad).getBytes())` ile aynıdır;
//! böylece server ile istemci aynı kimliği üretir.

use crate::{Error, Result};
use md5::{Digest, Md5};

pub fn validate_player_name(name: &str) -> Result<()> {
    let ok = (3..=16).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok { Ok(()) } else { Err(Error::BadPlayerName) }
}

/// Tireli küçük harfli UUID (sürüm 3).
pub fn offline_uuid(name: &str) -> Result<String> {
    validate_player_name(name)?;
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    let x = hex::encode(h);
    Ok(format!("{}-{}-{}-{}-{}", &x[0..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(validate_player_name("Efe_Kral").is_ok());
        assert_eq!(validate_player_name("ab"), Err(Error::BadPlayerName));
        assert_eq!(validate_player_name("bu ad çok uzun ve geçersiz"), Err(Error::BadPlayerName));
        assert_eq!(validate_player_name("a-b-c"), Err(Error::BadPlayerName));
    }

    #[test]
    fn matches_java_name_uuid_from_bytes() {
        // Değerler Python'da MD5 + sürüm/varyant bitleriyle bağımsız hesaplandı.
        assert_eq!(offline_uuid("Notch").unwrap(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(offline_uuid("Efe_Kral").unwrap(), "f0180f74-7e22-35a1-8953-3717dee31846");
    }

    #[test]
    fn uuid_shape_and_determinism() {
        let a = offline_uuid("Efe_Kral").unwrap();
        assert_eq!(a, offline_uuid("Efe_Kral").unwrap());
        assert_ne!(a, offline_uuid("efe_kral").unwrap(), "büyük/küçük harf farklı kimlik verir");
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'3', "sürüm 3");
        assert!(matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "RFC 4122 varyantı");
    }
}
