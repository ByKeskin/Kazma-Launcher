//! Windows Güvenlik Duvarı: Kazma'nın router'ı bulabilmesi (UDP) ve server'ın dışarıdan bağlantı alabilmesi (TCP) için izin kuralları.
//! Kural eklemek yönetici onayı (UAC) ister; yalnızca kullanıcı düğmeye basınca ve Windows'un kendi onay penceresiyle yapılır.

use crate::net::R;
use serde::Serialize;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Status {
    /// Windows dışında kural gerekmez.
    pub supported: bool,
    /// Kazma'nın router araması için UDP izni.
    pub kazma_ok: bool,
    /// Server'ın Java'sı için TCP izni.
    pub server_ok: bool,
}

pub fn kazma_rule() -> String {
    "Kazma Launcher (UPnP)".to_string()
}

pub fn server_rule(id: &str, port: u16) -> String {
    format!("Kazma Server {id} {port}")
}

/// Kural adı ve yol komut satırına girer; tırnak, &, |, ^, %, ! gibi karakterler reddedilir.
fn safe(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| matches!(c, '"' | '&' | '|' | '^' | '%' | '!' | '<' | '>' | '\'' | '`' | ';' | '\r' | '\n'))
}

fn ps_single(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// PowerShell'e -EncodedCommand ile verilecek metin (UTF-16LE, base64): tırnak/kaçış sorunları olmaz.
pub fn encode_command(script: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
fn powershell() -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]).creation_flags(0x0800_0000);
    c
}

/// Çıkış kodu bit alanı: 1 = Kazma kuralı var, 2 = server kuralı var.
#[cfg(windows)]
pub fn status(id: &str, port: u16) -> Status {
    let script = format!(
        "$c=0; if(Get-NetFirewallRule -DisplayName {} -ErrorAction SilentlyContinue){{$c+=1}}; if(Get-NetFirewallRule -DisplayName {} -ErrorAction SilentlyContinue){{$c+=2}}; exit $c",
        ps_single(&kazma_rule()),
        ps_single(&server_rule(id, port))
    );
    let code = powershell().args(["-EncodedCommand", &encode_command(&script)]).output().ok().and_then(|o| o.status.code()).unwrap_or(0);
    Status { supported: true, kazma_ok: code & 1 != 0, server_ok: code & 2 != 0 }
}

#[cfg(not(windows))]
pub fn status(_: &str, _: u16) -> Status {
    Status { supported: false, kazma_ok: true, server_ok: true }
}

/// Yönetici olarak çalışacak betik: eksik kuralları ekler, sonucu `log` dosyasına yazar.
pub fn build_script(id: &str, port: u16, kazma_exe: &Path, java_exe: &Path, need_kazma: bool, need_server: bool, log: &Path) -> R<String> {
    let (k, j, l) = (kazma_exe.to_string_lossy(), java_exe.to_string_lossy(), log.to_string_lossy());
    if !safe(id) || !safe(&l) {
        return Err("Yol desteklenmeyen karakter içeriyor.".into());
    }
    let mut body = String::new();
    if need_kazma {
        if !safe(&k) {
            return Err("Kazma'nın yolunda desteklenmeyen karakter var.".into());
        }
        let n = ps_single(&kazma_rule());
        body += &format!("if(-not (Get-NetFirewallRule -DisplayName {n} -ErrorAction SilentlyContinue)){{New-NetFirewallRule -DisplayName {n} -Direction Inbound -Action Allow -Protocol UDP -Program {} -Profile Any | Out-Null}};", ps_single(&k));
    }
    if need_server {
        if !safe(&j) {
            return Err("Java yolunda desteklenmeyen karakter var.".into());
        }
        let n = ps_single(&server_rule(id, port));
        body += &format!("if(-not (Get-NetFirewallRule -DisplayName {n} -ErrorAction SilentlyContinue)){{New-NetFirewallRule -DisplayName {n} -Direction Inbound -Action Allow -Protocol TCP -LocalPort {port} -Program {} -Profile Any | Out-Null}};", ps_single(&j));
    }
    if body.is_empty() {
        return Err("Gerekli izinler zaten var.".into());
    }
    Ok(format!("$ErrorActionPreference='Stop'; try {{ {body} 'ok' | Out-File -Encoding utf8 -LiteralPath {0} }} catch {{ $_.Exception.Message | Out-File -Encoding utf8 -LiteralPath {0} }}", ps_single(&l)))
}

#[cfg(windows)]
pub fn allow(id: &str, port: u16, java_exe: &Path) -> R<Status> {
    let st = status(id, port);
    let exe: PathBuf = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = std::env::temp_dir().join("kazma-firewall.txt");
    let _ = std::fs::remove_file(&log);
    let script = build_script(id, port, &exe, java_exe, !st.kazma_ok, !st.server_ok, &log)?;
    let outer = format!(
        "try {{ Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand','{}' -Verb RunAs -WindowStyle Hidden -Wait }} catch {{ exit 7 }}",
        encode_command(&script)
    );
    let out = powershell().args(["-EncodedCommand", &encode_command(&outer)]).output().map_err(|e| e.to_string())?;
    let after = status(id, port);
    if after.kazma_ok && after.server_ok {
        return Ok(after);
    }
    if out.status.code() == Some(7) {
        return Err("Yönetici onayı verilmedi ya da iptal edildi.".into());
    }
    let detail = std::fs::read_to_string(&log).unwrap_or_default();
    let detail = detail.trim_start_matches('\u{feff}').trim();
    Err(if detail.is_empty() || detail == "ok" {
        "Kural eklenemedi ve ayrıntı alınamadı. Windows Güvenlik Duvarı hizmeti kapalı olabilir.".to_string()
    } else {
        format!("Kural eklenemedi: {}", detail.chars().take(300).collect::<String>())
    })
}

#[cfg(not(windows))]
pub fn allow(_: &str, _: u16, _: &Path) -> R<Status> {
    Err("Güvenlik duvarı kuralları yalnızca Windows'ta eklenir.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_built_and_unsafe_paths_rejected() {
        let log = Path::new(r"C:\Temp\kazma-firewall.txt");
        let c = build_script("kule-2", 25565, Path::new(r"C:\Users\a\Kazma.exe"), Path::new(r"C:\Users\a\.kazma\runtime\java.exe"), true, true, log).unwrap();
        assert!(c.contains("-Protocol UDP -Program 'C:\\Users\\a\\Kazma.exe'") && c.contains("-LocalPort 25565") && c.contains("'Kazma Server kule-2 25565'"));
        assert!(c.contains("catch") && c.contains("kazma-firewall.txt"));
        assert_eq!(build_script("x", 25565, Path::new("a"), Path::new("b"), false, true, log).unwrap().matches("New-NetFirewallRule").count(), 1);
        assert!(build_script("x", 25565, Path::new("a"), Path::new("b"), false, false, log).is_err());
        for bad in ["C:\\a'; calc; '.exe", "C:\\a|b.exe", "C:\\%PATH%\\x.exe", "a\nb", "C:\\a\"b"] {
            assert!(build_script("x", 1, Path::new("k"), Path::new(bad), false, true, log).is_err(), "{bad}");
            assert!(build_script("x", 1, Path::new(bad), Path::new("j"), true, false, log).is_err(), "{bad}");
        }
    }

    #[test]
    fn encoded_command_is_utf16_base64() {
        assert_eq!(encode_command("a"), "YQA=");
        assert_eq!(encode_command("exit 1"), "ZQB4AGkAdAAgADEA");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_needs_nothing() {
        assert_eq!(status("a", 25565), Status { supported: false, kazma_ok: true, server_ok: true });
        assert!(allow("a", 25565, Path::new("java")).is_err());
    }
}
