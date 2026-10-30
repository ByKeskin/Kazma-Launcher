//! Oyun içi menü teması: Kazma temasına uygun panorama (başlık ekranı arka planı) içeren bir kaynak paketi üretir ve
//! instance'ın `options.txt` dosyasında etkinleştirir. Paket sürümün kendi `pack.mcmeta` bilgisini kullanır, bu yüzden
//! "uyumsuz paket" uyarısı çıkmaz. Yalnızca Minecraft 1.14 ve sonrası için uygulanır.

use crate::net::*;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub const PACK_FILE: &str = "KazmaTheme.zip";
const PACK_ENTRY: &str = "file/KazmaTheme.zip";

macro_rules! pano {
    ($v:literal) => {
        [
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_0.png")).as_slice(),
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_1.png")).as_slice(),
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_2.png")).as_slice(),
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_3.png")).as_slice(),
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_4.png")).as_slice(),
            include_bytes!(concat!("../assets/theme/", $v, "_panorama_5.png")).as_slice(),
        ]
    };
}

fn assets(variant: &str) -> R<([&'static [u8]; 6], &'static [u8])> {
    match variant {
        "obsidian" => Ok((pano!("obsidian"), include_bytes!("../assets/theme/obsidian_pack.png"))),
        "terminal" => Ok((pano!("terminal"), include_bytes!("../assets/theme/terminal_pack.png"))),
        _ => Err("Bilinmeyen tema.".into()),
    }
}

/// 1.14+ ve yeni sürüm adları (26.x, snapshot'lar) desteklenir.
pub fn supported(game: &str) -> bool {
    match game.strip_prefix("1.") {
        Some(rest) => rest.split('.').next().and_then(|m| m.parse::<u32>().ok()).map_or(false, |m| m >= 14),
        None => true,
    }
}

/// İstemci jar'ındaki `pack.mcmeta` içindeki "pack" bölümü.
pub fn pack_section(client_jar: &Path) -> Option<Value> {
    let f = fs::File::open(client_jar).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut e = z.by_name("pack.mcmeta").ok()?;
    let mut s = String::new();
    e.by_ref().take(64 * 1024).read_to_string(&mut s).ok()?;
    serde_json::from_str::<Value>(&s).ok()?.get("pack").cloned()
}

pub fn build_pack(variant: &str, section: &Value) -> R<Vec<u8>> {
    let (pano, icon) = assets(variant)?;
    let mut sec = section.clone();
    sec.as_object_mut().ok_or("pack bölümü beklenen biçimde değil")?.insert("description".into(), json!("Kazma teması"));
    let mut buf = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("pack.mcmeta", o).map_err(|e| e.to_string())?;
        z.write_all(json!({ "pack": sec }).to_string().as_bytes()).map_err(|e| e.to_string())?;
        z.start_file("pack.png", o).map_err(|e| e.to_string())?;
        z.write_all(icon).map_err(|e| e.to_string())?;
        for (i, p) in pano.iter().enumerate() {
            z.start_file(format!("assets/minecraft/textures/gui/title/background/panorama_{i}.png"), o).map_err(|e| e.to_string())?;
            z.write_all(p).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

/// `options.txt` içindeki `resourcePacks:[...]` satırına paketi ekler ya da çıkarır; başka hiçbir ayara dokunmaz.
pub fn edit_options(text: &str, enable: bool) -> String {
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| {
            let Some(rest) = l.strip_prefix("resourcePacks:") else { return l.to_string() };
            found = true;
            let mut list: Vec<String> = serde_json::from_str(rest).unwrap_or_default();
            list.retain(|x| x != PACK_ENTRY);
            if enable {
                list.push(PACK_ENTRY.to_string());
            }
            format!("resourcePacks:{}", serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
        })
        .collect();
    if !found && enable {
        lines.push(format!("resourcePacks:[\"vanilla\",\"{PACK_ENTRY}\"]"));
    }
    if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" }
}

/// Paketi yazar ve etkinleştirir (`enable=false` ise kaldırır).
pub fn apply(instance_dir: &Path, game: &str, client_jar: &Path, variant: &str, enable: bool) -> R<()> {
    // Kazma Tema modu (varsa) rengi buradan okur; mod yoksa dosya zararsızdır.
    if enable && ["obsidian", "terminal"].contains(&variant) {
        let cfg = instance_dir.join("config");
        if fs::create_dir_all(&cfg).is_ok() {
            let _ = write_atomic(&cfg.join("kazma-theme.txt"), variant.as_bytes());
        }
    }
    let opts = instance_dir.join("options.txt");
    let packs = instance_dir.join("resourcepacks");
    if !enable || !supported(game) {
        if let Ok(t) = fs::read_to_string(&opts) {
            if t.contains(PACK_ENTRY) {
                write_atomic(&opts, edit_options(&t, false).as_bytes())?;
            }
        }
        let _ = fs::remove_file(packs.join(PACK_FILE));
        return Ok(());
    }
    let section = pack_section(client_jar).ok_or("Sürümün pack bilgisi okunamadı")?;
    let zip = build_pack(variant, &section)?;
    fs::create_dir_all(&packs).map_err(|e| e.to_string())?;
    write_atomic(&packs.join(PACK_FILE), &zip)?;
    let old = fs::read_to_string(&opts).unwrap_or_default();
    let new = edit_options(&old, true);
    if new != old {
        write_atomic(&opts, new.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-gt-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_jar(d: &Path) -> std::path::PathBuf {
        let p = d.join("client.jar");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(br#"{"pack":{"description":{"translate":"dataPack.vanilla.description"},"pack_format":34,"min_format":34,"max_format":34}}"#).unwrap();
        z.finish().unwrap();
        p
    }

    #[test]
    fn version_support() {
        for (g, ok) in [("1.8.9", false), ("1.12.2", false), ("1.13.2", false), ("1.14", true), ("1.21.1", true), ("26.3", true), ("24w14a", true), ("1.x", false)] {
            assert_eq!(supported(g), ok, "{g}");
        }
    }

    #[test]
    fn options_editing_is_minimal_and_idempotent() {
        let o = "version:3955\nresourcePacks:[\"vanilla\",\"fabric\",\"file/Benim.zip\"]\nlang:tr_tr\n";
        let on = edit_options(o, true);
        assert!(on.contains("[\"vanilla\",\"fabric\",\"file/Benim.zip\",\"file/KazmaTheme.zip\"]") && on.contains("lang:tr_tr") && on.starts_with("version:3955"));
        assert_eq!(edit_options(&on, true), on, "tekrar eklenmez");
        assert_eq!(edit_options(&on, false), o);
        assert_eq!(edit_options("", true), "resourcePacks:[\"vanilla\",\"file/KazmaTheme.zip\"]\n");
        assert_eq!(edit_options("lang:en_us\n", false), "lang:en_us\n");
        assert!(edit_options("resourcePacks:bozuk\n", true).contains("[\"file/KazmaTheme.zip\"]"), "bozuk satır sıfırlanır");
    }

    #[test]
    fn pack_uses_the_versions_own_format() {
        let d = tmp("pack");
        let sec = pack_section(&fake_jar(&d)).unwrap();
        assert_eq!(sec["pack_format"], 34);
        for v in ["obsidian", "terminal"] {
            let bytes = build_pack(v, &sec).unwrap();
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut m = String::new();
            z.by_name("pack.mcmeta").unwrap().read_to_string(&mut m).unwrap();
            let j: Value = serde_json::from_str(&m).unwrap();
            assert_eq!((j["pack"]["pack_format"].as_i64(), j["pack"]["min_format"].as_i64(), j["pack"]["description"].as_str()), (Some(34), Some(34), Some("Kazma teması")));
            for i in 0..6 {
                assert!(z.by_name(&format!("assets/minecraft/textures/gui/title/background/panorama_{i}.png")).is_ok());
            }
        }
        assert!(build_pack("x", &sec).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_enables_and_disables() {
        let d = tmp("apply");
        let jar = fake_jar(&d);
        let inst = d.join("inst");
        fs::create_dir_all(&inst).unwrap();
        fs::write(inst.join("options.txt"), "gamma:1.0\n").unwrap();
        apply(&inst, "1.21.1", &jar, "obsidian", true).unwrap();
        assert!(inst.join("resourcepacks/KazmaTheme.zip").is_file());
        let t = fs::read_to_string(inst.join("options.txt")).unwrap();
        assert!(t.contains("gamma:1.0") && t.contains("file/KazmaTheme.zip"));
        assert_eq!(fs::read_to_string(inst.join("config/kazma-theme.txt")).unwrap(), "obsidian");
        apply(&inst, "1.21.1", &jar, "terminal", false).unwrap();
        assert!(!inst.join("resourcepacks/KazmaTheme.zip").exists());
        assert!(!fs::read_to_string(inst.join("options.txt")).unwrap().contains("KazmaTheme"));
        apply(&inst, "1.8.9", &jar, "obsidian", true).unwrap();
        assert!(!inst.join("resourcepacks/KazmaTheme.zip").exists(), "eski sürümde uygulanmaz");
        assert!(apply(&inst, "1.21.1", &d.join("yok.jar"), "obsidian", true).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
