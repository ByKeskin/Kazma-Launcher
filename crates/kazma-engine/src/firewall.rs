//! Windows Güvenlik Duvarı: Kazma'nın router'ı bulabilmesi (UDP) ve server'ın dışarıdan bağlantı alabilmesi (TCP) için izin kuralları.
//! Kural eklemek yönetici onayı (UAC) ister; yalnızca kullanıcı düğmeye basınca ve Windows'un kendi onay penceresiyle yapılır.

use crate::net::R;
use serde::Serialize;
use std::path::{Path, PathBuf};
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

#[cfg(windows)]
fn netsh() -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("netsh");
    c.creation_flags(0x0800_0000);
    c
}

#[cfg(windows)]
fn rule_exists(name: &str) -> bool {
    netsh().args(["advfirewall", "firewall", "show", "rule", &format!("name={name}")]).output().map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(not(windows))]
fn rule_exists(_: &str) -> bool {
    true
}

pub fn status(id: &str, port: u16) -> Status {
    if !cfg!(windows) {
        return Status { supported: false, kazma_ok: true, server_ok: true };
    }
    Status { supported: true, kazma_ok: rule_exists(&kazma_rule()), server_ok: rule_exists(&server_rule(id, port)) }
}

/// Eklenecek `netsh` komutları (yönetici olarak çalıştırılacak tek bir `cmd /c` satırı).
pub fn add_commands(id: &str, port: u16, kazma_exe: &Path, java_exe: &Path, need_kazma: bool, need_server: bool) -> R<String> {
    let (k, j) = (kazma_exe.to_string_lossy(), java_exe.to_string_lossy());
    let mut parts = Vec::new();
    if need_kazma {
        if !safe(&k) {
            return Err("Kazma'nın yolunda desteklenmeyen karakter var.".into());
        }
        parts.push(format!("netsh advfirewall firewall add rule name=\"{}\" dir=in action=allow protocol=UDP program=\"{k}\" enable=yes profile=any", kazma_rule()));
    }
    if need_server {
        if !safe(&j) || !safe(id) {
            return Err("Java yolunda desteklenmeyen karakter var.".into());
        }
        parts.push(format!("netsh advfirewall firewall add rule name=\"{}\" dir=in action=allow protocol=TCP localport={port} program=\"{j}\" enable=yes profile=any", server_rule(id, port)));
    }
    if parts.is_empty() {
        return Err("Gerekli izinler zaten var.".into());
    }
    Ok(parts.join(" & "))
}

#[cfg(windows)]
pub fn allow(id: &str, port: u16, java_exe: &Path) -> R<Status> {
    use std::os::windows::process::CommandExt;
    let st = status(id, port);
    let exe: PathBuf = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmdline = add_commands(id, port, &exe, java_exe, !st.kazma_ok, !st.server_ok)?;
    // cmd /c "a & b" yönetici olarak: PowerShell Start-Process -Verb RunAs (Windows'un onay penceresi açılır).
    let arg = format!("/c {cmdline}").replace('\'', "''");
    let ps = format!("Start-Process -FilePath cmd.exe -ArgumentList '{arg}' -Verb RunAs -WindowStyle Hidden -Wait");
    let out = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &ps]).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    let after = status(id, port);
    if after.kazma_ok && after.server_ok {
        Ok(after)
    } else if !out.status.success() {
        Err("Yönetici onayı verilmedi ya da iptal edildi.".into())
    } else {
        Err("Kural eklenemedi. Windows Güvenlik Duvarı'nı elle açıp Java'ya izin verebilirsin.".into())
    }
}

#[cfg(not(windows))]
pub fn allow(_: &str, _: u16, _: &Path) -> R<Status> {
    Err("Güvenlik duvarı kuralları yalnızca Windows'ta eklenir.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_built_and_unsafe_paths_rejected() {
        let c = add_commands("kule-2", 25565, Path::new(r"C:\Users\a\Kazma.exe"), Path::new(r"C:\Users\a\.kazma\runtime\java.exe"), true, true).unwrap();
        assert!(c.contains("protocol=UDP program=\"C:\\Users\\a\\Kazma.exe\"") && c.contains("localport=25565") && c.contains(" & "));
        assert!(c.contains("name=\"Kazma Server kule-2 25565\""));
        assert_eq!(add_commands("x", 25565, Path::new("a"), Path::new("b"), false, true).unwrap().matches("netsh").count(), 1);
        assert!(add_commands("x", 25565, Path::new("a"), Path::new("b"), false, false).is_err());
        for bad in [r#"C:\a" & calc & ".exe"#, "C:\\a|b.exe", "C:\\%PATH%\\x.exe", "a\nb"] {
            assert!(add_commands("x", 1, Path::new("k"), Path::new(bad), false, true).is_err(), "{bad}");
            assert!(add_commands("x", 1, Path::new(bad), Path::new("j"), true, false).is_err(), "{bad}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_needs_nothing() {
        assert_eq!(status("a", 25565), Status { supported: false, kazma_ok: true, server_ok: true });
        assert!(allow("a", 25565, Path::new("java")).is_err());
    }
}
