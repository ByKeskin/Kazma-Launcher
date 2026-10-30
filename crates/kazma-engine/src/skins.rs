//! Skin kitaplığı: PNG dosyaları kaydeder, Mojang'dan oyuncu adıyla skin indirir ve seçili skini
//! instance'a uygular. Çevrimdışı hesaplarda vanilla istemci skin göstermez; skinin oyunda görünmesi için
//! CustomSkinLoader modu gerekir. Skin yalnızca kendi ekranında görünür (sunucudaki diğer oyuncular görmez).

use crate::net::*;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use kazma_core::account::validate_player_name;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_PNG: usize = 200 * 1024;
const PNG_SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinInfo {
    pub id: String,
    pub name: String,
    /// "dosya" ya da "oyuncu:<ad>"
    pub source: String,
    pub added: u64,
}

/// (genişlik, yükseklik). PNG imzası ve IHDR bölümü okunur.
pub fn png_size(b: &[u8]) -> R<(u32, u32)> {
    if b.len() < 24 || b[..8] != PNG_SIG || &b[12..16] != b"IHDR" {
        return Err("Bu bir PNG dosyası değil".into());
    }
    Ok((u32::from_be_bytes([b[16], b[17], b[18], b[19]]), u32::from_be_bytes([b[20], b[21], b[22], b[23]])))
}

/// Minecraft skinleri 64x64 (güncel) ya da 64x32 (eski) olur.
pub fn validate_skin(b: &[u8]) -> R<()> {
    if b.len() > MAX_PNG {
        return Err("Skin dosyası çok büyük (en fazla 200 KB)".into());
    }
    match png_size(b)? {
        (64, 64) | (64, 32) => Ok(()),
        (w, h) => Err(format!("Skin 64x64 ya da 64x32 olmalı, bu dosya {w}x{h}")),
    }
}

fn dir(root: &Path) -> PathBuf {
    root.join("skins")
}

pub fn list_skins(root: &Path) -> Vec<SkinInfo> {
    let mut v: Vec<SkinInfo> = fs::read_to_string(dir(root).join("index.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v.retain(|s| dir(root).join(format!("{}.png", s.id)).is_file());
    v.sort_by(|a, b| b.added.cmp(&a.added));
    v
}

fn save_index(root: &Path, v: &[SkinInfo]) -> R<()> {
    write_atomic(&dir(root).join("index.json"), serde_json::to_string_pretty(v).map_err(|e| e.to_string())?.as_bytes())
}

fn id_ok(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn clean_name(n: &str) -> R<String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 40 || n.chars().any(|c| c.is_control()) {
        return Err("skin adı 1-40 karakter olmalı".into());
    }
    Ok(n.to_string())
}

pub fn add_skin(root: &Path, name: &str, source: &str, bytes: &[u8]) -> R<SkinInfo> {
    validate_skin(bytes)?;
    let name = clean_name(name)?;
    let id = sha1_hex(bytes)[..12].to_string();
    let mut all = list_skins(root);
    if let Some(ex) = all.iter().find(|s| s.id == id) {
        return Err(format!("Bu skin zaten kayıtlı: {}", ex.name));
    }
    write_atomic(&dir(root).join(format!("{id}.png")), bytes)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let info = SkinInfo { id, name, source: source.to_string(), added: now };
    all.push(info.clone());
    save_index(root, &all)?;
    Ok(info)
}

/// Arayüzden gelen base64 PNG'yi ekler (dosya seçici sonucu).
pub fn add_skin_b64(root: &Path, name: &str, b64: &str) -> R<SkinInfo> {
    let payload = b64.rsplit(',').next().unwrap_or(b64); // "data:image/png;base64," önekini at
    let bytes = STANDARD.decode(payload.trim()).map_err(|_| "Dosya okunamadı".to_string())?;
    add_skin(root, name, "dosya", &bytes)
}

/// Mojang'dan oyuncu adıyla indirip kitaplığa ekler.
pub fn add_from_username(root: &Path, player: &str) -> R<SkinInfo> {
    let bytes = fetch_by_username(player)?;
    add_skin(root, player, &format!("oyuncu:{player}"), &bytes)
}

pub fn delete_skin(root: &Path, id: &str) -> R<()> {
    if !id_ok(id) {
        return Err("skin kimliği geçersiz".into());
    }
    let mut all = list_skins(root);
    if !all.iter().any(|s| s.id == id) {
        return Err("skin bulunamadı".into());
    }
    let _ = fs::remove_file(dir(root).join(format!("{id}.png")));
    all.retain(|s| s.id != id);
    save_index(root, &all)
}

pub fn skin_bytes(root: &Path, id: &str) -> R<Vec<u8>> {
    if !id_ok(id) {
        return Err("skin kimliği geçersiz".into());
    }
    fs::read(dir(root).join(format!("{id}.png"))).map_err(|_| "skin bulunamadı".to_string())
}

/// Arayüzde göstermek için `data:` adresi.
pub fn skin_data_url(root: &Path, id: &str) -> R<String> {
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(skin_bytes(root, id)?)))
}

/// Seçili skini bir instance'ın CustomSkinLoader yerel klasörüne yazar (mod yoksa zararsız bir dosyadır).
pub fn apply_to_instance(root: &Path, instance_dir: &Path, player: &str, skin_id: &str) -> R<()> {
    validate_player_name(player).map_err(|e| e.to_string())?;
    let bytes = skin_bytes(root, skin_id)?;
    write_atomic(&instance_dir.join("CustomSkinLoader").join("LocalSkin").join("skins").join(format!("{player}.png")), &bytes)
}

// ---------- Mojang'dan oyuncu adıyla skin ----------

pub fn parse_profile_id(json: &str) -> R<String> {
    let v: Value = serde_json::from_str(json).map_err(|_| "Bu adda bir Minecraft oyuncusu bulunamadı".to_string())?;
    let id = v.get("id").and_then(Value::as_str).unwrap_or("");
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Bu adda bir Minecraft oyuncusu bulunamadı".into());
    }
    Ok(id.to_ascii_lowercase())
}

/// Profil yanıtındaki `textures` özelliğinden skin adresi. Yalnızca textures.minecraft.net kabul edilir.
pub fn parse_skin_url(profile_json: &str) -> R<Option<String>> {
    let v: Value = serde_json::from_str(profile_json).map_err(|e| format!("profil okunamadı: {e}"))?;
    let Some(val) = v.get("properties").and_then(Value::as_array).and_then(|a| a.iter().find(|p| p.get("name").and_then(Value::as_str) == Some("textures"))).and_then(|p| p.get("value")).and_then(Value::as_str) else {
        return Ok(None);
    };
    let raw = STANDARD.decode(val).map_err(|_| "profil dokuları okunamadı".to_string())?;
    let t: Value = serde_json::from_slice(&raw).map_err(|_| "profil dokuları okunamadı".to_string())?;
    let Some(url) = t.pointer("/textures/SKIN/url").and_then(Value::as_str) else { return Ok(None) };
    let ok = url::Url::parse(url).ok().map_or(false, |u| u.scheme() == "https" && u.host_str() == Some("textures.minecraft.net"));
    if !ok {
        return Err(format!("güvenilmeyen skin adresi: {url}"));
    }
    Ok(Some(url.to_string()))
}

pub fn fetch_by_username(name: &str) -> R<Vec<u8>> {
    validate_player_name(name).map_err(|e| e.to_string())?;
    let id = parse_profile_id(&get_text(&format!("https://api.mojang.com/users/profiles/minecraft/{name}"), 64 << 10).map_err(|_| "Bu adda bir Minecraft oyuncusu bulunamadı ya da Mojang'a ulaşılamadı".to_string())?)?;
    let prof = get_text(&format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}"), 256 << 10)?;
    let url = parse_skin_url(&prof)?.ok_or("Bu oyuncunun özel bir skini yok (varsayılan skin kullanıyor)")?;
    let bytes = get_bytes(&url, MAX_PNG as u64)?;
    validate_skin(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fake_png(w: u32, h: u32, salt: u8) -> Vec<u8> {
        let mut b = PNG_SIG.to_vec();
        b.extend([0, 0, 0, 13]);
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0, salt, salt, salt]);
        b
    }
    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-skin-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn validates_png_and_dimensions() {
        assert!(validate_skin(&fake_png(64, 64, 1)).is_ok());
        assert!(validate_skin(&fake_png(64, 32, 1)).is_ok());
        assert!(validate_skin(&fake_png(128, 128, 1)).unwrap_err().contains("128x128"));
        assert!(validate_skin(b"GIF89a....................").unwrap_err().contains("PNG"));
        assert!(validate_skin(&[]).is_err());
        let mut big = fake_png(64, 64, 1);
        big.resize(MAX_PNG + 1, 0);
        assert!(validate_skin(&big).unwrap_err().contains("büyük"));
    }

    #[test]
    fn library_add_list_delete_apply() {
        let r = tmp("lib");
        let a = add_skin(&r, "  Mavi ", "dosya", &fake_png(64, 64, 1)).unwrap();
        assert_eq!((a.name.as_str(), a.id.len()), ("Mavi", 12));
        assert!(add_skin(&r, "Kopya", "dosya", &fake_png(64, 64, 1)).unwrap_err().contains("zaten kayıtlı"));
        let b = add_skin(&r, "Kırmızı", "oyuncu:Notch", &fake_png(64, 32, 2)).unwrap();
        assert_eq!(list_skins(&r).len(), 2);
        assert!(skin_data_url(&r, &a.id).unwrap().starts_with("data:image/png;base64,"));
        assert!(add_skin(&r, "", "dosya", &fake_png(64, 64, 3)).is_err());
        assert!(skin_bytes(&r, "../../x").is_err());

        let inst = r.join("instances/x");
        apply_to_instance(&r, &inst, "Mustafa", &a.id).unwrap();
        assert_eq!(fs::read(inst.join("CustomSkinLoader/LocalSkin/skins/Mustafa.png")).unwrap(), fake_png(64, 64, 1));
        assert!(apply_to_instance(&r, &inst, "../evil", &a.id).is_err(), "oyuncu adı yol kaçıramaz");

        delete_skin(&r, &b.id).unwrap();
        assert_eq!(list_skins(&r).len(), 1);
        assert!(delete_skin(&r, &b.id).is_err());
        assert!(delete_skin(&r, "..\\..\\x").is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn imports_from_browser_base64() {
        let r = tmp("b64");
        let b64 = STANDARD.encode(fake_png(64, 64, 7));
        let a = add_skin_b64(&r, "Dosyadan", &format!("data:image/png;base64,{b64}")).unwrap();
        assert_eq!(a.source, "dosya");
        assert!(add_skin_b64(&r, "x", "%%%").is_err());
        assert!(add_skin_b64(&r, "x", &STANDARD.encode(b"not png at all, just text....")).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn parses_mojang_profile() {
        assert_eq!(parse_profile_id(r#"{"id":"069A79F444E94726A5BEFCA90E38AAF5","name":"Notch"}"#).unwrap(), "069a79f444e94726a5befca90e38aaf5");
        assert!(parse_profile_id("").unwrap_err().contains("bulunamadı"));
        assert!(parse_profile_id(r#"{"id":"zz"}"#).is_err());
        let tex = |url: &str| {
            let inner = format!(r#"{{"textures":{{"SKIN":{{"url":"{url}"}}}}}}"#);
            format!(r#"{{"id":"x","properties":[{{"name":"textures","value":"{}"}}]}}"#, STANDARD.encode(inner))
        };
        assert_eq!(parse_skin_url(&tex("https://textures.minecraft.net/texture/abc")).unwrap().as_deref(), Some("https://textures.minecraft.net/texture/abc"));
        assert!(parse_skin_url(&tex("https://evil.example/s.png")).unwrap_err().contains("güvenilmeyen"));
        assert!(parse_skin_url(&tex("http://textures.minecraft.net/texture/abc")).is_err(), "http reddedilir");
        assert_eq!(parse_skin_url(r#"{"id":"x","properties":[]}"#).unwrap(), None);
        let no_skin = format!(r#"{{"properties":[{{"name":"textures","value":"{}"}}]}}"#, STANDARD.encode(r#"{"textures":{}}"#));
        assert_eq!(parse_skin_url(&no_skin).unwrap(), None);
    }
}
