//! Davet bağlantısı: `kazma://join/host[:port][?m=<manifest adresi>]` ya da düz `host[:port]`.
//! `m` parametresi, server'ın imzalı mod manifestinin (https) adresidir; yüzde kodlanmış olmalıdır.

use crate::{Error, Result};
use url::Url;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Invite {
    pub host: String,
    pub port: u16,
    pub manifest_url: Option<String>,
}

impl Invite {
    pub fn address(&self) -> String {
        if self.port == 25565 { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }

    /// Paylaşılabilir davet bağlantısı.
    pub fn to_link(&self) -> String {
        let mut s = format!("kazma://join/{}", self.address());
        if let Some(m) = &self.manifest_url {
            s.push_str("?m=");
            s.extend(url::form_urlencoded::byte_serialize(m.as_bytes()));
        }
        s
    }
}

pub fn parse(input: &str) -> Result<Invite> {
    let s = input.trim();
    let s = s.strip_prefix("kazma://join/").unwrap_or(s);
    let (addr, query) = match s.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (s, None),
    };
    let addr = addr.trim_end_matches('/');
    if addr.is_empty() || addr.contains(char::is_whitespace) || addr.contains("://") || addr.contains('/') {
        return Err(Error::BadInvite("adres boş ya da biçimi yanlış".into()));
    }
    let (host, port) = match addr.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| Error::BadInvite(format!("port geçersiz: {p}")))?;
            if port == 0 {
                return Err(Error::BadInvite("port 0 olamaz".into()));
            }
            (h, port)
        }
        None => (addr, 25565),
    };
    let label_ok = |l: &str| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if host.is_empty() || host.len() > 253 || !host.split('.').all(label_ok) {
        return Err(Error::BadInvite(format!("sunucu adı geçersiz: {host}")));
    }
    let mut manifest_url = None;
    if let Some(q) = query {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            if k == "m" {
                let u = Url::parse(&v).map_err(|_| Error::BadInvite("manifest adresi geçersiz".into()))?;
                if u.scheme() != "https" || u.host_str().is_none() {
                    return Err(Error::BadInvite("manifest adresi https olmalı".into()));
                }
                manifest_url = Some(v.into_owned());
            }
        }
    }
    Ok(Invite { host: host.to_ascii_lowercase(), port, manifest_url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        let a = parse("kazma://join/95.12.34.56:25565").unwrap();
        assert_eq!(a, Invite { host: "95.12.34.56".into(), port: 25565, manifest_url: None });
        assert_eq!(parse("Dostlar-SMP.tunel.example").unwrap().port, 25565);
        assert_eq!(parse("  play.example.com:25570 ").unwrap().address(), "play.example.com:25570");
        assert_eq!(parse("kazma://join/play.example.com/").unwrap().host, "play.example.com");
    }

    #[test]
    fn manifest_param_roundtrip() {
        let i = Invite { host: "play.example.com".into(), port: 25570, manifest_url: Some("https://example.com/a/kazma.json?x=1&y=2".into()) };
        let link = i.to_link();
        assert!(link.starts_with("kazma://join/play.example.com:25570?m=https%3A%2F%2F"));
        assert_eq!(parse(&link).unwrap(), i);
    }

    #[test]
    fn manifest_param_must_be_https() {
        assert!(parse("kazma://join/a.com?m=http%3A%2F%2Fa.com%2Fk.json").is_err());
        assert!(parse("kazma://join/a.com?m=notaurl").is_err());
        assert_eq!(parse("kazma://join/a.com?other=1").unwrap().manifest_url, None);
    }

    #[test]
    fn rejects() {
        for bad in ["", "kazma://join/", "host name", "host:0", "host:99999", "host:abc", "-bad.example", "http://x.com", "a/b", "a..b"] {
            assert!(parse(bad).is_err(), "{bad:?} reddedilmeli");
        }
    }
}
