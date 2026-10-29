//! Mod yükleyicilerinin sürüm bulma ve kurucu (installer) adresleri. Ayrıştırma saf fonksiyonlardır;
//! ağ erişimi `get` ile dışarıdan verilir, böylece testlenebilir.

use crate::net::R;
use kazma_core::mojang::plain;
use serde_json::Value;

fn pick_json_stable(json: &str, ptr_stable: &str, ptr_version: &str) -> R<String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("sürüm listesi okunamadı: {e}"))?;
    let a = v.as_array().ok_or("sürüm listesi beklenen biçimde değil")?;
    let ver = |e: &Value| e.pointer(ptr_version).and_then(Value::as_str).map(String::from);
    a.iter().find(|e| ptr_stable.is_empty() || e.pointer(ptr_stable).and_then(Value::as_bool) == Some(true)).and_then(ver).ok_or_else(|| "kararlı sürüm bulunamadı".into())
}

/// Fabric meta: `/loader/stable` olan ilk kayıt.
pub fn fabric_pick(json: &str) -> R<String> {
    pick_json_stable(json, "/loader/stable", "/loader/version")
}

/// Quilt meta: sürüm listesi yeniden eskiye; ön sürümler (`-beta`, `-rc`) atlanır.
pub fn quilt_pick(json: &str) -> R<String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("sürüm listesi okunamadı: {e}"))?;
    v.as_array()
        .and_then(|a| a.iter().filter_map(|e| e.get("version").and_then(Value::as_str)).find(|s| !s.contains('-')))
        .map(String::from)
        .ok_or_else(|| "kararlı Quilt sürümü bulunamadı".into())
}

/// Forge: önce önerilen (recommended), yoksa en son (latest).
pub fn forge_pick(promos_json: &str, game: &str) -> R<String> {
    let v: Value = serde_json::from_str(promos_json).map_err(|e| format!("Forge listesi okunamadı: {e}"))?;
    let p = v.get("promos").ok_or("Forge listesinde 'promos' yok")?;
    ["recommended", "latest"]
        .iter()
        .find_map(|k| p.get(format!("{game}-{k}")).and_then(Value::as_str))
        .map(String::from)
        .ok_or_else(|| format!("Forge {game} için sürüm bulamadı"))
}

/// NeoForge sürümleri Minecraft sürümüne göre adlandırılır: 1.21.1 → 21.1.x, 1.21 → 21.0.x. 1.20.2'den önce yok.
pub fn neoforge_prefix(game: &str) -> Option<String> {
    let mut it = game.split('.');
    if it.next()? != "1" {
        return None;
    }
    let minor: u32 = it.next()?.parse().ok()?;
    let patch: u32 = it.next().map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    if minor < 20 || (minor == 20 && patch < 2) {
        return None;
    }
    Some(format!("{minor}.{patch}."))
}

pub fn neoforge_pick(metadata_xml: &str, game: &str) -> R<String> {
    let prefix = neoforge_prefix(game).ok_or_else(|| format!("NeoForge, Minecraft {game} için kullanılamıyor (1.20.2 ve sonrası gerekir)"))?;
    let all: Vec<&str> = metadata_xml.split("<version>").skip(1).filter_map(|s| s.split("</version>").next()).filter(|v| v.starts_with(&prefix)).collect();
    all.iter().rev().find(|v| !v.contains('-')).or(all.last()).map(|s| s.to_string()).ok_or_else(|| format!("NeoForge {game} için sürüm bulamadı"))
}

/// (kurucu adresi, sha1 adresi)
pub fn installer_urls(kind: &str, game: &str, version: &str) -> R<(String, String)> {
    if !plain(game) || !plain(version) {
        return Err("sürüm adı geçersiz karakter içeriyor".into());
    }
    let jar = match kind {
        "forge" => format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{game}-{version}/forge-{game}-{version}-installer.jar"),
        "neoforge" => format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"),
        o => return Err(format!("{o} kurucuyla kurulmaz")),
    };
    let sha = format!("{jar}.sha1");
    Ok((jar, sha))
}

/// Seçilen yükleyicinin, verilen Minecraft sürümü için en uygun sürümü.
pub fn latest_version(kind: &str, game: &str, get: &dyn Fn(&str) -> R<String>) -> R<String> {
    if !plain(game) {
        return Err("sürüm adı geçersiz karakter içeriyor".into());
    }
    match kind {
        "fabric" => fabric_pick(&get(&format!("https://meta.fabricmc.net/v2/versions/loader/{game}"))?),
        "quilt" => quilt_pick(&get("https://meta.quiltmc.org/v3/versions/loader")?),
        "forge" => forge_pick(&get("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")?, game),
        "neoforge" => neoforge_pick(&get("https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml")?, game),
        o => Err(format!("{o} için sürüm aranamaz")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_and_quilt() {
        let f = r#"[{"loader":{"version":"0.17.0-beta.1","stable":false}},{"loader":{"version":"0.16.9","stable":true}}]"#;
        assert_eq!(fabric_pick(f).unwrap(), "0.16.9");
        assert!(fabric_pick("[]").is_err());
        let q = r#"[{"version":"0.27.0-beta.2"},{"version":"0.26.1"},{"version":"0.26.0"}]"#;
        assert_eq!(quilt_pick(q).unwrap(), "0.26.1");
    }

    #[test]
    fn forge_prefers_recommended() {
        let j = r#"{"promos":{"1.20.1-latest":"47.3.12","1.20.1-recommended":"47.2.0","1.21.1-latest":"52.0.1"}}"#;
        assert_eq!(forge_pick(j, "1.20.1").unwrap(), "47.2.0");
        assert_eq!(forge_pick(j, "1.21.1").unwrap(), "52.0.1");
        assert!(forge_pick(j, "1.19").is_err());
    }

    #[test]
    fn neoforge_versions_follow_minecraft() {
        assert_eq!(neoforge_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("1.20.4").as_deref(), Some("20.4."));
        for no in ["1.20.1", "1.19.4", "26.1", "x"] {
            assert_eq!(neoforge_prefix(no), None, "{no}");
        }
        let xml = "<versioning><versions><version>21.0.5</version><version>21.1.0-beta</version><version>21.1.170</version><version>21.1.172</version><version>21.2.1-beta</version></versions></versioning>";
        assert_eq!(neoforge_pick(xml, "1.21.1").unwrap(), "21.1.172", "beta atlanır");
        assert_eq!(neoforge_pick(xml, "1.21").unwrap(), "21.0.5");
        assert_eq!(neoforge_pick(xml, "1.21.2").unwrap(), "21.2.1-beta", "yalnızca beta varsa o seçilir");
        assert!(neoforge_pick(xml, "1.21.3").unwrap_err().contains("sürüm bulamadı"));
        assert!(neoforge_pick(xml, "1.20.1").unwrap_err().contains("1.20.2"));
    }

    #[test]
    fn installer_urls_and_validation() {
        let (j, s) = installer_urls("forge", "1.20.1", "47.3.0").unwrap();
        assert_eq!(j, "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-installer.jar");
        assert_eq!(s, format!("{j}.sha1"));
        let (j, _) = installer_urls("neoforge", "1.21.1", "21.1.172").unwrap();
        assert_eq!(j, "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.172/neoforge-21.1.172-installer.jar");
        assert!(installer_urls("forge", "1.20.1/../x", "1").is_err());
        assert!(installer_urls("fabric", "1.20.1", "1").is_err());
    }

    #[test]
    fn dispatch_uses_the_right_endpoint() {
        let seen = std::cell::RefCell::new(Vec::<String>::new());
        let get = |u: &str| -> R<String> {
            seen.borrow_mut().push(u.to_string());
            Ok(r#"{"promos":{"1.20.1-recommended":"47.2.0"}}"#.to_string())
        };
        assert_eq!(latest_version("forge", "1.20.1", &get).unwrap(), "47.2.0");
        assert!(seen.borrow()[0].contains("promotions_slim.json"));
        assert!(latest_version("vanilla", "1.20.1", &get).is_err());
        assert!(latest_version("forge", "../x", &get).is_err());
    }
}
