//! Java çalışma zamanı: Adoptium (Eclipse Temurin) JRE'si. Sürüm numarasını oyun dosyası belirler
//! (1.21 için 21, 1.17-1.20.4 için 17 ya da 21, eski sürümler için 8).

use crate::version::{Env, Os};
use crate::{Error, Result};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub struct JavaPackage {
    pub link: String,
    pub sha256: String,
    pub name: String,
    pub size: u64,
}

pub fn adoptium_query_url(major: u32, env: &Env) -> String {
    let os = match env.os {
        Os::Windows => "windows",
        Os::Linux => "linux",
        Os::Osx => "mac",
    };
    let arch = match env.arch.as_str() {
        "arm64" => "aarch64",
        "x86" => "x32",
        _ => "x64",
    };
    format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jre&os={os}&vendor=eclipse")
}

/// Adoptium yanıtının ilk paketini okur. İndirme adresi github.com olmalı; bütünlük SHA-256 ile doğrulanır.
pub fn parse_adoptium(json: &str) -> Result<JavaPackage> {
    let v: Value = serde_json::from_str(json).map_err(|e| Error::BadManifest(e.to_string()))?;
    let pkg = v.pointer("/0/binary/package").ok_or_else(|| Error::BadManifest("Adoptium yanıtında paket yok".into()))?;
    let s = |k: &str| pkg.get(k).and_then(Value::as_str).map(str::to_string);
    let link = s("link").ok_or_else(|| Error::BadManifest("paket adresi yok".into()))?;
    let sha256 = s("checksum").ok_or_else(|| Error::BadManifest("paket özeti yok".into()))?;
    let name = s("name").ok_or_else(|| Error::BadManifest("paket adı yok".into()))?;
    let size = pkg.get("size").and_then(Value::as_u64).unwrap_or(0);
    let host_ok = url::Url::parse(&link).ok().map_or(false, |u| u.scheme() == "https" && u.host_str() == Some("github.com"));
    if !host_ok {
        return Err(Error::UntrustedUrl(link));
    }
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::BadManifest("paket özeti geçersiz".into()));
    }
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err(Error::BadManifest("paket adı geçersiz".into()));
    }
    Ok(JavaPackage { link, sha256, name, size })
}

/// `root` altında Java'nın kurulacağı klasör.
pub fn runtime_dir(major: u32) -> String {
    format!("runtime/java-{major}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_url() {
        assert_eq!(
            adoptium_query_url(21, &Env::windows_x64()),
            "https://api.adoptium.net/v3/assets/latest/21/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse"
        );
    }

    #[test]
    fn parses_package() {
        let sum = "c".repeat(64);
        let j = format!(
            r#"[{{"binary":{{"package":{{"link":"https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_x64_windows_hotspot_21.0.5_11.zip","checksum":"{sum}","name":"OpenJDK21U-jre_x64_windows_hotspot_21.0.5_11.zip","size":47000000}}}}}}]"#
        );
        let p = parse_adoptium(&j).unwrap();
        assert_eq!(p.sha256, sum);
        assert!(p.name.ends_with(".zip"));
    }

    #[test]
    fn rejects_foreign_host_and_bad_sum() {
        let ok = "c".repeat(64);
        let evil = format!(r#"[{{"binary":{{"package":{{"link":"https://evil.example/j.zip","checksum":"{ok}","name":"j.zip","size":1}}}}}}]"#);
        assert!(matches!(parse_adoptium(&evil), Err(Error::UntrustedUrl(_))));
        let bad = r#"[{"binary":{"package":{"link":"https://github.com/a/b.zip","checksum":"zz","name":"b.zip","size":1}}}]"#;
        assert!(parse_adoptium(bad).is_err());
        assert!(parse_adoptium("[]").is_err());
    }
}
