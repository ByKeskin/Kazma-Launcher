//! Davet bağlantısı: `kazma://join/host[:port]` ya da düz `host[:port]`.

use crate::{Error, Result};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Invite {
    pub host: String,
    pub port: u16,
}

impl Invite {
    pub fn address(&self) -> String {
        if self.port == 25565 { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }
}

pub fn parse(input: &str) -> Result<Invite> {
    let s = input.trim();
    let s = s.strip_prefix("kazma://join/").unwrap_or(s).trim_end_matches('/');
    if s.is_empty() || s.contains(char::is_whitespace) || s.contains("://") || s.contains('/') {
        return Err(Error::BadInvite("adres boş ya da biçimi yanlış".into()));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| Error::BadInvite(format!("port geçersiz: {p}")))?;
            if port == 0 {
                return Err(Error::BadInvite("port 0 olamaz".into()));
            }
            (h, port)
        }
        None => (s, 25565),
    };
    let label_ok = |l: &str| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if host.is_empty() || host.len() > 253 || !host.split('.').all(label_ok) {
        return Err(Error::BadInvite(format!("sunucu adı geçersiz: {host}")));
    }
    Ok(Invite { host: host.to_ascii_lowercase(), port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        let a = parse("kazma://join/95.12.34.56:25565").unwrap();
        assert_eq!(a, Invite { host: "95.12.34.56".into(), port: 25565 });
        assert_eq!(parse("Dostlar-SMP.tunel.example").unwrap().port, 25565);
        assert_eq!(parse("  play.example.com:25570 ").unwrap().address(), "play.example.com:25570");
        assert_eq!(parse("kazma://join/play.example.com/").unwrap().host, "play.example.com");
    }

    #[test]
    fn rejects() {
        for bad in ["", "kazma://join/", "host name", "host:0", "host:99999", "host:abc", "-bad.example", "http://x.com", "a/b", "a..b"] {
            assert!(parse(bad).is_err(), "{bad:?} reddedilmeli");
        }
    }
}
