use crate::{Error, Result};
use sha2::{Digest, Sha512};

pub fn sha512_hex(bytes: &[u8]) -> String {
    hex::encode(Sha512::digest(bytes))
}

/// İndirilen dosyayı manifestteki özetle karşılaştırır. Uyuşmazsa dosya kullanılmaz.
pub fn verify_sha512(bytes: &[u8], expected_hex: &str) -> Result<()> {
    if sha512_hex(bytes).eq_ignore_ascii_case(expected_hex.trim()) { Ok(()) } else { Err(Error::HashMismatch) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // SHA-512("abc") için standart test vektörü
        let want = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
        assert_eq!(sha512_hex(b"abc"), want);
        assert!(verify_sha512(b"abc", &want.to_uppercase()).is_ok());
        assert_eq!(verify_sha512(b"abd", want), Err(Error::HashMismatch));
    }
}
