//! Kazma çekirdeği. Arayüzden ve Tauri'den bağımsızdır; her şey saf fonksiyondur ve
//! ağ erişimi olmadan test edilebilir. Ağ ve dosya işlemleri Tauri tarafında yapılır.

pub mod account;
pub mod instance;
pub mod invite;
pub mod java;
pub mod launch;
pub mod manifest;
pub mod modjar;
pub mod pack;
pub mod mojang;
pub mod plan;
pub mod sign;
pub mod trust;
pub mod verify;
pub mod version;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("geçersiz oyuncu adı: 3-16 karakter, yalnızca harf, rakam ve alt çizgi")]
    BadPlayerName,
    #[error("geçersiz davet: {0}")]
    BadInvite(String),
    #[error("manifest okunamadı: {0}")]
    BadManifest(String),
    #[error("indirme adresi güvenilir değil: {0}")]
    UntrustedUrl(String),
    #[error("dosya özeti uyuşmuyor")]
    HashMismatch,
    #[error("imza geçersiz")]
    BadSignature,
}

pub type Result<T> = std::result::Result<T, Error>;
