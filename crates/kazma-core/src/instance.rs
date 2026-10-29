//! Instance: tek bir oyun sürümü + yükleyici + kendi `mods` klasörü. Serverlar kendi instance'ını alır;
//! böylece bir server'ın modları ya da sürümü diğerlerini bozmaz. Kütüphaneler ve asset'ler ortak kalır.

use crate::manifest::PackManifest;
use crate::mojang::plain;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderSpec {
    Vanilla,
    Fabric { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceSpec {
    pub id: String,
    pub game: String,
    pub loader: LoaderSpec,
}

impl InstanceSpec {
    pub fn dir_rel(&self) -> String {
        format!("instances/{}", self.id)
    }
    pub fn mods_rel(&self) -> String {
        format!("instances/{}/mods", self.id)
    }
    /// `versions/<id>/<id>.json` dosyasının kimliği.
    pub fn version_id(&self) -> String {
        match &self.loader {
            LoaderSpec::Vanilla => self.game.clone(),
            LoaderSpec::Fabric { version } => format!("fabric-loader-{version}-{}", self.game),
        }
    }
}

/// Oyuncunun kendi seçtiği sürüm (isteğe bağlı Fabric).
pub fn from_selection(game: &str, fabric_version: Option<&str>) -> Result<InstanceSpec> {
    if !plain(game) {
        return Err(Error::BadManifest(format!("sürüm adı geçersiz: {game}")));
    }
    match fabric_version {
        None => Ok(InstanceSpec { id: format!("{game}-vanilla"), game: game.into(), loader: LoaderSpec::Vanilla }),
        Some(v) if plain(v) => Ok(InstanceSpec { id: format!("{game}-fabric-{v}"), game: game.into(), loader: LoaderSpec::Fabric { version: v.into() } }),
        Some(v) => Err(Error::BadManifest(format!("Fabric sürümü geçersiz: {v}"))),
    }
}

/// Server'ın manifesti hangi sürümü ve yükleyiciyi istiyorsa, o server için ayrı instance.
pub fn from_manifest(m: &PackManifest, host: &str, port: u16) -> Result<InstanceSpec> {
    if !plain(&m.game) {
        return Err(Error::BadManifest("manifestteki oyun sürümü geçersiz karakter içeriyor".into()));
    }
    let loader = match m.loader.kind.as_str() {
        "vanilla" => LoaderSpec::Vanilla,
        "fabric" if plain(&m.loader.version) => LoaderSpec::Fabric { version: m.loader.version.clone() },
        "fabric" => return Err(Error::BadManifest("manifestteki Fabric sürümü geçersiz".into())),
        other => return Err(Error::BadManifest(format!("'{other}' yükleyicisi henüz desteklenmiyor (şimdilik vanilla ve fabric)"))),
    };
    let host_ok = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !host_ok {
        return Err(Error::BadInvite(format!("sunucu adı geçersiz: {host}")));
    }
    Ok(InstanceSpec { id: format!("server-{}-{port}", host.to_ascii_lowercase()), game: m.game.clone(), loader })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Loader, PackManifest};

    fn pack(kind: &str, game: &str, ver: &str) -> PackManifest {
        PackManifest { name: "S".into(), game: game.into(), loader: Loader { kind: kind.into(), version: ver.into() }, mods: vec![], signer: None, signature: None }
    }

    #[test]
    fn player_selection() {
        let v = from_selection("1.20.1", None).unwrap();
        assert_eq!((v.id.as_str(), v.version_id().as_str()), ("1.20.1-vanilla", "1.20.1"));
        let f = from_selection("1.21.1", Some("0.16.9")).unwrap();
        assert_eq!(f.id, "1.21.1-fabric-0.16.9");
        assert_eq!(f.version_id(), "fabric-loader-0.16.9-1.21.1");
        assert_eq!(f.mods_rel(), "instances/1.21.1-fabric-0.16.9/mods");
        assert!(from_selection("../x", None).is_err());
        assert!(from_selection("1.21.1", Some("a b")).is_err());
    }

    #[test]
    fn server_decides_version() {
        let i = from_manifest(&pack("fabric", "1.20.4", "0.15.11"), "Play.Example.com", 25570).unwrap();
        assert_eq!(i.id, "server-play.example.com-25570");
        assert_eq!(i.game, "1.20.4");
        assert_eq!(i.loader, LoaderSpec::Fabric { version: "0.15.11".into() });
        assert_eq!(i.version_id(), "fabric-loader-0.15.11-1.20.4");
        assert_eq!(from_manifest(&pack("vanilla", "1.21.1", ""), "a.com", 25565).unwrap().loader, LoaderSpec::Vanilla);
        assert!(from_manifest(&pack("fabric", "1.21.1", ""), "a.com", 25565).is_err(), "Fabric için sürüm şart");
    }

    #[test]
    fn unsupported_loader_and_bad_inputs() {
        assert!(matches!(from_manifest(&pack("forge", "1.20.1", "47.2.0"), "a.com", 25565), Err(Error::BadManifest(m)) if m.contains("henüz desteklenmiyor")));
        assert!(from_manifest(&pack("fabric", "../x", "0.1"), "a.com", 25565).is_err());
        assert!(from_manifest(&pack("fabric", "1.21.1", "0.1"), "a/b", 25565).is_err());
    }
}
