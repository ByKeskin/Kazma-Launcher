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

// ---------- güvenilir kaynaklar ----------

#[derive(Debug, Clone, Serialize)]
pub struct SkinSource {
    pub id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    /// Tarayıcıda açılacak ana sayfa (yalnızca bu listedeki adresler açılır).
    pub browse: Option<&'static str>,
    /// Kazma'nın doğrudan indirebildiği yol: "oyuncu" (ad), "bağlantı" ya da yok (siteden PNG indirip dosyadan eklenir).
    pub direct: Option<&'static str>,
}

pub fn trusted_sources() -> Vec<SkinSource> {
    vec![
        SkinSource { id: "mojang", name: "Mojang (resmi)", note: "Premium bir oyuncunun güncel skini; oyuncu adını yaz.", browse: None, direct: Some("oyuncu") },
        SkinSource { id: "namemc", name: "NameMC", note: "Skin sayfasının adresini yapıştır, doku indirilir.", browse: Some("https://namemc.com/minecraft-skins"), direct: Some("bağlantı") },
        SkinSource { id: "ely", name: "Ely.by", note: "Ely.by hesabı olan (çevrimdışı oyuncular dahil) birinin skini; oyuncu adını yaz.", browse: Some("https://ely.by/skins"), direct: Some("oyuncu") },
        SkinSource { id: "skindex", name: "The Skindex", note: "Siteden PNG olarak indir, sonra \"PNG dosyasından\" ekle.", browse: Some("https://www.minecraftskins.com/"), direct: None },
        SkinSource { id: "pmc", name: "Planet Minecraft", note: "Siteden PNG olarak indir, sonra \"PNG dosyasından\" ekle.", browse: Some("https://www.planetminecraft.com/skins/"), direct: None },
    ]
}

/// Tarayıcıda açılabilecek adresler: yalnızca güvenilir listedeki ana sayfalar.
pub fn browse_url_allowed(url: &str) -> bool {
    trusted_sources().iter().any(|s| s.browse == Some(url))
}

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Yapıştırılan adresi doğrudan indirilebilir bir doku adresine çevirir. Yalnızca güvenilir kaynaklar kabul edilir:
/// NameMC skin sayfası (`namemc.com/skin/<16 hex>`), NameMC dokusu (`s.namemc.com/i/<16 hex>.png`) ve
/// Mojang CDN'i (`textures.minecraft.net/texture/<64 hex>`).
pub fn texture_url_from_link(link: &str) -> R<(String, String)> {
    let u = url::Url::parse(link.trim()).map_err(|_| "Bu geçerli bir bağlantı değil".to_string())?;
    if u.scheme() != "https" {
        return Err("Yalnızca https bağlantıları kabul edilir".into());
    }
    let host = u.host_str().unwrap_or("").trim_start_matches("www.").to_ascii_lowercase();
    let seg: Vec<&str> = u.path_segments().map(|s| s.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let tail = |i: usize| seg.get(i).map(|s| s.trim_end_matches(".png")).unwrap_or("");
    match (host.as_str(), seg.first().copied()) {
        ("namemc.com", Some("skin")) if is_hex(tail(1), 16) => Ok((format!("https://s.namemc.com/i/{}.png", tail(1).to_ascii_lowercase()), format!("NameMC {}", &tail(1)[..6]))),
        ("s.namemc.com", Some("i")) if is_hex(tail(1), 16) => Ok((format!("https://s.namemc.com/i/{}.png", tail(1).to_ascii_lowercase()), format!("NameMC {}", &tail(1)[..6]))),
        ("textures.minecraft.net", Some("texture")) if is_hex(tail(1), 64) => Ok((format!("https://textures.minecraft.net/texture/{}", tail(1).to_ascii_lowercase()), format!("Mojang {}", &tail(1)[..6]))),
        _ => Err("Bu adres güvenilir kaynak listesinde değil. Desteklenenler: NameMC skin sayfası, NameMC dokusu ve Mojang dokusu. Diğer siteler için PNG'yi indirip \"PNG dosyasından\" ekle.".into()),
    }
}

pub fn add_from_link(root: &Path, link: &str) -> R<SkinInfo> {
    let (url, name) = texture_url_from_link(link)?;
    let bytes = get_bytes(&url, MAX_PNG as u64).map_err(|_| "Skin indirilemedi. Adres geçersiz ya da site yanıt vermiyor.".to_string())?;
    validate_skin(&bytes)?;
    add_skin(root, &name, "bağlantı", &bytes)
}

/// Ely.by skin sistemi: oyuncu adıyla doku. Çevrimdışı oyuncular arasında yaygındır.
pub fn add_from_ely(root: &Path, player: &str) -> R<SkinInfo> {
    validate_player_name(player).map_err(|e| e.to_string())?;
    let bytes = get_bytes(&format!("https://skinsystem.ely.by/skins/{player}.png"), MAX_PNG as u64).map_err(|_| "Ely.by'da bu adla bir skin bulunamadı".to_string())?;
    validate_skin(&bytes)?;
    add_skin(root, player, &format!("Ely.by:{player}"), &bytes)
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
    fn trusted_links_only() {
        let (u, n) = texture_url_from_link("https://namemc.com/skin/0123456789abcdef").unwrap();
        assert_eq!((u.as_str(), n.as_str()), ("https://s.namemc.com/i/0123456789abcdef.png", "NameMC 012345"));
        assert_eq!(texture_url_from_link("https://www.namemc.com/skin/0123456789ABCDEF?foo=1").unwrap().0, "https://s.namemc.com/i/0123456789abcdef.png");
        assert_eq!(texture_url_from_link("https://s.namemc.com/i/0123456789abcdef.png").unwrap().0, "https://s.namemc.com/i/0123456789abcdef.png");
        let h = "a".repeat(64);
        assert_eq!(texture_url_from_link(&format!("https://textures.minecraft.net/texture/{h}")).unwrap().0, format!("https://textures.minecraft.net/texture/{h}"));
        for bad in ["http://namemc.com/skin/0123456789abcdef", "https://evil.example/skin/0123456789abcdef", "https://namemc.com.evil.example/skin/0123456789abcdef",
            "https://namemc.com/skin/zzzz", "https://textures.minecraft.net/texture/abc", "https://minecraftskins.com/skin/123/x", "javascript:alert(1)", "not a url", "https://namemc.com/profile/Notch"] {
            assert!(texture_url_from_link(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_list_and_browse_allowlist() {
        let l = trusted_sources();
        assert!(l.iter().any(|s| s.id == "mojang" && s.direct == Some("oyuncu")));
        assert!(l.iter().any(|s| s.id == "skindex" && s.direct.is_none()), "doğrulayamadığımız site doğrudan indirilmez");
        assert!(browse_url_allowed("https://namemc.com/minecraft-skins"));
        assert!(!browse_url_allowed("https://evil.example/"));
        assert!(!browse_url_allowed("https://namemc.com/minecraft-skins?x=1"), "listedeki adresin aynısı olmalı");
        assert!(!browse_url_allowed("file:///C:/Windows/System32/calc.exe"));
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
