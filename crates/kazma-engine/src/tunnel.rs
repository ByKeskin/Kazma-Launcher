//! Tünel (playit.gg): router'da port açılamayan ağlar (CGNAT, kurumsal ağ) için. Resmî ajan GitHub'dan indirilir,
//! Windows'ta Authenticode imzası doğrulanmadan çalıştırılmaz. Hesap bağlama ve tünel oluşturma playit.gg sitesinde bir kez yapılır.

use crate::net::*;
use crate::report::Reporter;
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENT_URL: &str = "https://github.com/playit-cloud/playit-agent/releases/latest/download/playit-windows-x86_64-signed.exe";

pub fn agent_dir(root: &Path) -> PathBuf {
    root.join("tunnel")
}

pub fn agent_path(root: &Path) -> PathBuf {
    agent_dir(root).join("playit.exe")
}

/// Ajanın yazdığı satırdan hesap bağlama adresini çıkarır (yalnızca https://playit.gg/claim/<harf-rakam>).
pub fn claim_url_in(line: &str) -> Option<String> {
    let i = line.find("https://playit.gg/claim/")?;
    let tail: String = line[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '-' | '_' | '?' | '=' | '&')).collect();
    claim_allowed(&tail).then_some(tail)
}

pub fn claim_allowed(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(u) => u.scheme() == "https" && u.host_str() == Some("playit.gg") && u.path().starts_with("/claim/") && u.path().len() > 7 && u.username().is_empty() && u.port().is_none(),
        Err(_) => false,
    }
}

pub fn open_claim(url: &str) -> R<()> {
    if !claim_allowed(url) {
        return Err("Bu adres izin verilen playit.gg bağlantılarından değil.".into());
    }
    crate::app::open_in_file_manager(Path::new(url))
}

/// playit.gg ajanını imzalayan kuruluş (Authenticode sertifikasındaki ad). Başka bir imzacı kabul edilmez.
pub const PUBLISHER: &str = "Developed Methods LLC";

/// PowerShell çıktısı: "<Durum>|<İmzacı adı>". Yalnızca durum "Valid" ve ad PUBLISHER ise kabul edilir.
pub fn signature_ok(output: &str) -> bool {
    let line = output.trim();
    match line.split_once('|') {
        Some((status, name)) => status.trim() == "Valid" && name.trim() == PUBLISHER,
        None => false,
    }
}

#[cfg(windows)]
fn signature_info(p: &Path) -> String {
    use std::os::windows::process::CommandExt;
    let path = p.to_string_lossy().replace('\'', "''");
    let script = format!("$s=Get-AuthenticodeSignature -LiteralPath '{path}'; $n=''; if($s.SignerCertificate){{$n=$s.SignerCertificate.GetNameInfo('SimpleName',$false)}}; Write-Output ($s.Status.ToString()+'|'+$n)");
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn signature_info(_: &Path) -> String {
    String::new()
}

/// Ajanı indirir (yoksa) ve imzasını doğrular.
pub fn ensure_agent(root: &Path, rep: &dyn Reporter) -> R<PathBuf> {
    if !cfg!(windows) {
        return Err("Tünel ajanı şimdilik yalnızca Windows'ta kurulur.".into());
    }
    let p = agent_path(root);
    if !p.is_file() {
        rep.stage("playit.gg ajanı indiriliyor");
        let bytes = get_bytes(AGENT_URL, 80 << 20)?;
        if bytes.len() < 100_000 || &bytes[..2] != b"MZ" {
            return Err("İndirilen dosya bir Windows programı değil.".into());
        }
        fs::create_dir_all(agent_dir(root)).map_err(|e| e.to_string())?;
        let tmp = agent_dir(root).join("playit.download");
        fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        let info = signature_info(&tmp);
        if !signature_ok(&info) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("playit.gg ajanının dijital imzası doğrulanamadı; çalıştırılmadı. (Beklenen imzacı: {PUBLISHER}; bulunan: {})", if info.is_empty() { "okunamadı".to_string() } else { info }));
        }
        fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pinned_publisher_with_a_valid_signature_passes() {
        assert!(signature_ok("Valid|Developed Methods LLC\r\n"));
        for bad in ["Valid|playit", "Valid|Developed Methods LLC Evil", "NotSigned|", "HashMismatch|Developed Methods LLC", "UnknownError|Developed Methods LLC", "", "Valid", "valid|Developed Methods LLC"] {
            assert!(!signature_ok(bad), "{bad}");
        }
    }

    #[test]
    fn claim_urls_are_strict() {
        assert_eq!(claim_url_in("Visit https://playit.gg/claim/abc123XY to link").as_deref(), Some("https://playit.gg/claim/abc123XY"));
        assert_eq!(claim_url_in("x").as_deref(), None);
        for bad in ["https://playit.gg.evil.com/claim/a", "http://playit.gg/claim/a", "https://playit.gg/claim/", "https://playit.gg:444/claim/a", "https://evil.com/claim/a", "https://u@playit.gg/claim/a", "https://playit.gg/other/a"] {
            assert!(!claim_allowed(bad), "{bad}");
        }
        assert!(open_claim("https://evil.com/claim/a").is_err());
    }
}
