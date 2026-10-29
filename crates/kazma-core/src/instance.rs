//! Instance: tek bir oyun sürümü + yükleyici + kendi `mods` klasörü. Serverlar kendi instance'ını alır;
//! böylece bir server'ın modları ya da sürümü diğerlerini bozmaz. Kütüphaneler ve asset'ler ortak kalır.

use crate::manifest::PackManifest;
use crate::mojang::plain;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderSpec {
    Vanilla,
    Fabric { version: String },
    Quilt { version: String },
    Forge { version: String },
    NeoForge { version: String },
}

pub const LOADER_KINDS: &[&str] = &["vanilla", "fabric", "quilt", "forge", "neoforge"];

impl LoaderSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            LoaderSpec::Vanilla => "vanilla",
            LoaderSpec::Fabric { .. } => "fabric",
            LoaderSpec::Quilt { .. } => "quilt",
            LoaderSpec::Forge { .. } => "forge",
            LoaderSpec::NeoForge { .. } => "neoforge",
        }
    }
    pub fn version(&self) -> &str {
        match self {
            LoaderSpec::Vanilla => "",
            LoaderSpec::Fabric { version } | LoaderSpec::Quilt { version } | LoaderSpec::Forge { version } | LoaderSpec::NeoForge { version } => version,
        }
    }
    /// Forge ve NeoForge kendi kurucularıyla (installer) kurulur; Fabric ve Quilt profil JSON'uyla.
    pub fn needs_installer(&self) -> bool {
        matches!(self, LoaderSpec::Forge { .. } | LoaderSpec::NeoForge { .. })
    }
    pub fn from_kind(kind: &str, version: &str) -> Result<LoaderSpec> {
        if kind == "vanilla" {
            return Ok(LoaderSpec::Vanilla);
        }
        if !LOADER_KINDS.contains(&kind) {
            return Err(Error::BadManifest(format!("'{kind}' yükleyicisi desteklenmiyor (vanilla, fabric, quilt, forge, neoforge)")));
        }
        if !plain(version) {
            return Err(Error::BadManifest(format!("{kind} sürümü geçersiz: '{version}'")));
        }
        let v = version.to_string();
        Ok(match kind {
            "fabric" => LoaderSpec::Fabric { version: v },
            "quilt" => LoaderSpec::Quilt { version: v },
            "forge" => LoaderSpec::Forge { version: v },
            _ => LoaderSpec::NeoForge { version: v },
        })
    }
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
    /// `versions/<id>/<id>.json` dosyasının beklenen kimliği.
    pub fn version_id(&self) -> String {
        match &self.loader {
            LoaderSpec::Vanilla => self.game.clone(),
            LoaderSpec::Fabric { version } => format!("fabric-loader-{version}-{}", self.game),
            LoaderSpec::Quilt { version } => format!("quilt-loader-{version}-{}", self.game),
            LoaderSpec::Forge { version } => format!("{}-forge-{version}", self.game),
            LoaderSpec::NeoForge { version } => format!("neoforge-{version}"),
        }
    }
}

/// Oyuncunun kendi seçtiği sürüm ve yükleyici.
pub fn from_selection(game: &str, kind: &str, loader_version: &str) -> Result<InstanceSpec> {
    if !plain(game) {
        return Err(Error::BadManifest(format!("sürüm adı geçersiz: {game}")));
    }
    let loader = LoaderSpec::from_kind(kind, loader_version)?;
    let id = match &loader {
        LoaderSpec::Vanilla => format!("{game}-vanilla"),
        l => format!("{game}-{}-{}", l.kind(), l.version()),
    };
    Ok(InstanceSpec { id, game: game.into(), loader })
}

fn server_id(host: &str, port: u16) -> Result<String> {
    let ok = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !ok {
        return Err(Error::BadInvite(format!("sunucu adı geçersiz: {host}")));
    }
    Ok(format!("server-{}-{port}", host.to_ascii_lowercase()))
}

/// Server'ın manifesti hangi sürümü ve yükleyiciyi istiyorsa, o server için ayrı instance.
pub fn from_manifest(m: &PackManifest, host: &str, port: u16) -> Result<InstanceSpec> {
    if !plain(&m.game) {
        return Err(Error::BadManifest("manifestteki oyun sürümü geçersiz karakter içeriyor".into()));
    }
    let loader = LoaderSpec::from_kind(&m.loader.kind, &m.loader.version)?;
    Ok(InstanceSpec { id: server_id(host, port)?, game: m.game.clone(), loader })
}

/// Davet bağlantısı olmayan (örn. Aternos) bir server için, oyuncunun seçtiği sürümle instance.
pub fn for_server(host: &str, port: u16, game: &str, kind: &str, loader_version: &str) -> Result<InstanceSpec> {
    if !plain(game) {
        return Err(Error::BadManifest(format!("sürüm adı geçersiz: {game}")));
    }
    Ok(InstanceSpec { id: server_id(host, port)?, game: game.into(), loader: LoaderSpec::from_kind(kind, loader_version)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Loader, PackManifest};

    fn pack(kind: &str, game: &str, ver: &str) -> PackManifest {
        PackManifest { name: "S".into(), game: game.into(), loader: Loader { kind: kind.into(), version: ver.into() }, mods: vec![], signer: None, signature: None }
    }

    #[test]
    fn player_selection_all_loaders() {
        let v = from_selection("1.20.1", "vanilla", "").unwrap();
        assert_eq!((v.id.as_str(), v.version_id().as_str()), ("1.20.1-vanilla", "1.20.1"));
        let f = from_selection("1.21.1", "fabric", "0.16.9").unwrap();
        assert_eq!((f.id.as_str(), f.version_id().as_str()), ("1.21.1-fabric-0.16.9", "fabric-loader-0.16.9-1.21.1"));
        assert_eq!(from_selection("1.21.1", "quilt", "0.26.1").unwrap().version_id(), "quilt-loader-0.26.1-1.21.1");
        let fg = from_selection("1.20.1", "forge", "47.3.0").unwrap();
        assert_eq!((fg.version_id().as_str(), fg.loader.needs_installer()), ("1.20.1-forge-47.3.0", true));
        let nf = from_selection("1.21.1", "neoforge", "21.1.172").unwrap();
        assert_eq!((nf.version_id().as_str(), nf.id.as_str()), ("neoforge-21.1.172", "1.21.1-neoforge-21.1.172"));
        assert!(!f.loader.needs_installer());
        assert_eq!(f.mods_rel(), "instances/1.21.1-fabric-0.16.9/mods");
        assert!(from_selection("../x", "vanilla", "").is_err());
        assert!(from_selection("1.21.1", "fabric", "a b").is_err());
        assert!(from_selection("1.21.1", "fabric", "").is_err(), "yükleyici için sürüm şart");
        assert!(from_selection("1.21.1", "liteloader", "1").is_err());
    }

    #[test]
    fn server_decides_version_and_loader() {
        let i = from_manifest(&pack("fabric", "1.20.4", "0.15.11"), "Play.Example.com", 25570).unwrap();
        assert_eq!((i.id.as_str(), i.game.as_str(), i.version_id().as_str()), ("server-play.example.com-25570", "1.20.4", "fabric-loader-0.15.11-1.20.4"));
        assert_eq!(from_manifest(&pack("vanilla", "1.21.1", ""), "a.com", 25565).unwrap().loader, LoaderSpec::Vanilla);
        let f = from_manifest(&pack("forge", "1.20.1", "47.3.0"), "a.com", 25565).unwrap();
        assert_eq!(f.loader, LoaderSpec::Forge { version: "47.3.0".into() });
        assert!(from_manifest(&pack("fabric", "1.21.1", ""), "a.com", 25565).is_err());
        assert!(from_manifest(&pack("fabric", "../x", "0.1"), "a.com", 25565).is_err());
        assert!(from_manifest(&pack("fabric", "1.21.1", "0.1"), "a/b", 25565).is_err());
        assert!(from_manifest(&pack("magic", "1.21.1", "1"), "a.com", 25565).is_err());
    }

    #[test]
    fn plain_address_server_instance() {
        let i = for_server("my_world.aternos.me", 12345, "1.20.1", "forge", "47.3.0").unwrap();
        assert_eq!(i.id, "server-my_world.aternos.me-12345", "Aternos adlarında alt çizgi olabilir");
        assert!(for_server("a/b", 25565, "1.20.1", "vanilla", "").is_err());
        assert!(for_server("a.com", 25565, "1.20.1", "forge", "").is_err());
    }
}
