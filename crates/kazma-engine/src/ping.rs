//! Minecraft Server List Ping: bir server'a bağlanmadan sürümünü, oyuncu sayısını ve mod yükleyici ipucunu öğrenir.
//! Aternos ve benzeri servislerde adres dışında bilgi gerekmesin diye kullanılır.

use crate::net::R;
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const PROTOCOL_HINT: u32 = 767;

pub fn write_varint(buf: &mut Vec<u8>, mut v: u32) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            buf.push(b | 0x80);
        } else {
            buf.push(b);
            return;
        }
    }
}

pub fn read_varint(r: &mut impl Read) -> R<u32> {
    let mut n = 0u32;
    for i in 0..5 {
        let mut b = [0u8; 1];
        r.read_exact(&mut b).map_err(|e| format!("yanıt okunamadı: {e}"))?;
        n |= ((b[0] & 0x7F) as u32) << (7 * i);
        if b[0] & 0x80 == 0 {
            return Ok(n);
        }
    }
    Err("geçersiz yanıt (uzun sayı)".into())
}

fn frame(body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(&mut out, body.len() as u32);
    out.extend(body);
    out
}

pub fn handshake_packet(host: &str, port: u16) -> Vec<u8> {
    let mut b = Vec::new();
    write_varint(&mut b, 0x00);
    write_varint(&mut b, PROTOCOL_HINT);
    write_varint(&mut b, host.len() as u32);
    b.extend(host.as_bytes());
    b.extend(port.to_be_bytes());
    write_varint(&mut b, 1); // sonraki durum: status
    frame(b)
}

pub fn status_request_packet() -> Vec<u8> {
    frame(vec![0x00])
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub version_name: String,
    pub protocol: i64,
    pub players_online: i64,
    pub players_max: i64,
    pub motd: String,
    /// "forge" (Forge ya da NeoForge) ipucu; Fabric/Quilt kendini duyurmaz.
    pub loader_hint: Option<String>,
}

fn strip_colors(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '§' {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn flatten_text(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => out.push_str(s),
        Value::Array(a) => a.iter().for_each(|x| flatten_text(x, out)),
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(Value::as_str) {
                out.push_str(t);
            }
            if let Some(e) = o.get("extra") {
                flatten_text(e, out);
            }
        }
        _ => {}
    }
}

pub fn parse_status(json: &str) -> R<Status> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("server yanıtı okunamadı: {e}"))?;
    let mut motd = String::new();
    if let Some(d) = v.get("description") {
        flatten_text(d, &mut motd);
    }
    let motd = strip_colors(&motd).split_whitespace().collect::<Vec<_>>().join(" ");
    let hint = ["forgeData", "neoForgeData", "modinfo"].iter().any(|k| v.get(*k).is_some()).then(|| "forge".to_string());
    Ok(Status {
        version_name: strip_colors(v.pointer("/version/name").and_then(Value::as_str).unwrap_or("")),
        protocol: v.pointer("/version/protocol").and_then(Value::as_i64).unwrap_or(0),
        players_online: v.pointer("/players/online").and_then(Value::as_i64).unwrap_or(0),
        players_max: v.pointer("/players/max").and_then(Value::as_i64).unwrap_or(0),
        motd,
        loader_hint: hint,
    })
}

/// `Paper 1.21.1` → 1.21.1; `Velocity 1.7.2-1.21.1` gibi aralıkta en yeni uygun sürümü seçer.
/// `1.21.x` gibi joker sürümler tahmin için kullanılmaz.
pub fn guess_game_version(name: &str, available: &[String]) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            let wildcard = run.ends_with('.') && matches!(chars.get(i), Some('x') | Some('X'));
            let tok = run.trim_matches('.').to_string();
            if !wildcard && tok.contains('.') && tok.split('.').all(|s| !s.is_empty()) {
                found.push(tok);
            }
        } else {
            i += 1;
        }
    }
    found.into_iter().filter(|t| available.iter().any(|a| a == t)).last()
}

pub fn query(host: &str, port: u16, timeout: Duration) -> R<Status> {
    let addr = (host, port).to_socket_addrs().map_err(|_| format!("{host} adresi çözülemedi"))?.next().ok_or_else(|| format!("{host} adresi çözülemedi"))?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("bağlanılamadı: {e}"))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    s.write_all(&handshake_packet(host, port)).and_then(|_| s.write_all(&status_request_packet())).map_err(|e| format!("gönderilemedi: {e}"))?;
    let _len = read_varint(&mut s)?;
    let id = read_varint(&mut s)?;
    if id != 0 {
        return Err("beklenmeyen yanıt türü".into());
    }
    let n = read_varint(&mut s)? as usize;
    if n == 0 || n > (1 << 20) {
        return Err("yanıt boyutu geçersiz".into());
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).map_err(|e| format!("yanıt okunamadı: {e}"))?;
    parse_status(&String::from_utf8_lossy(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn varint_roundtrip() {
        for v in [0u32, 1, 127, 128, 300, 25565, 2097151, u32::MAX] {
            let mut b = Vec::new();
            write_varint(&mut b, v);
            assert_eq!(read_varint(&mut &b[..]).unwrap(), v);
        }
        assert!(read_varint(&mut &[0x80u8; 6][..]).is_err());
    }

    #[test]
    fn handshake_layout() {
        let p = handshake_packet("a.com", 25565);
        assert_eq!(p[0] as usize, p.len() - 1, "uzunluk öneki");
        assert_eq!(p[1], 0x00, "paket kimliği");
        assert!(p.windows(5).any(|w| w == b"a.com"));
        assert_eq!(&p[p.len() - 3..], &[0x63, 0xDD, 0x01], "port (BE) ve durum=1");
        assert_eq!(status_request_packet(), vec![1, 0]);
    }

    #[test]
    fn parses_status_variants() {
        let s = parse_status(r#"{"version":{"name":"Paper 1.21.1","protocol":767},"players":{"max":20,"online":3},"description":{"text":"§aHoş geldin","extra":[{"text":" §lDostlar"}]}}"#).unwrap();
        assert_eq!((s.version_name.as_str(), s.protocol, s.players_online, s.players_max), ("Paper 1.21.1", 767, 3, 20));
        assert_eq!(s.motd, "Hoş geldin Dostlar");
        assert_eq!(s.loader_hint, None);
        let f = parse_status(r#"{"version":{"name":"1.20.1","protocol":763},"players":{"max":10,"online":0},"description":"x","forgeData":{"channels":[],"mods":[],"fmlNetworkVersion":3}}"#).unwrap();
        assert_eq!(f.loader_hint.as_deref(), Some("forge"));
        let old = parse_status(r#"{"version":{"name":"1.12.2","protocol":340},"description":{"text":"m"},"modinfo":{"type":"FML","modList":[]}}"#).unwrap();
        assert_eq!(old.loader_hint.as_deref(), Some("forge"));
        assert!(parse_status("{").is_err());
    }

    #[test]
    fn guesses_version_from_names() {
        let av: Vec<String> = ["1.7.2", "1.20.1", "1.21", "1.21.1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(guess_game_version("Paper 1.21.1", &av).as_deref(), Some("1.21.1"));
        assert_eq!(guess_game_version("Velocity 1.7.2-1.21.1", &av).as_deref(), Some("1.21.1"), "aralıkta en yenisi");
        assert_eq!(guess_game_version("1.20.1", &av).as_deref(), Some("1.20.1"));
        assert_eq!(guess_game_version("BungeeCord 1.8.x-1.21.x", &av), None);
        assert_eq!(guess_game_version("§4● Offline", &av), None);
        assert_eq!(guess_game_version("Spigot 9.9.9", &av), None, "listede olmayan sürüm önerilmez");
    }

    #[test]
    fn talks_to_a_fake_server() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let len = read_varint(&mut c).unwrap() as usize;
            let mut hs = vec![0u8; len];
            c.read_exact(&mut hs).unwrap();
            let mut req = [0u8; 2];
            c.read_exact(&mut req).unwrap();
            assert_eq!(req, [1, 0], "durum isteği");
            let json = br#"{"version":{"name":"Forge 1.20.1","protocol":763},"players":{"max":8,"online":2},"description":"Aternos","forgeData":{}}"#;
            let mut body = Vec::new();
            write_varint(&mut body, 0);
            write_varint(&mut body, json.len() as u32);
            body.extend(json);
            c.write_all(&frame(body)).unwrap();
            hs
        });
        let st = query("127.0.0.1", port, Duration::from_secs(3)).unwrap();
        assert_eq!((st.version_name.as_str(), st.players_online), ("Forge 1.20.1", 2));
        assert_eq!(st.loader_hint.as_deref(), Some("forge"));
        let hs = t.join().unwrap();
        assert_eq!(hs[0], 0x00);
    }

    #[test]
    fn unreachable_server_is_a_friendly_error() {
        let e = query("127.0.0.1", 1, Duration::from_millis(500)).unwrap_err();
        assert!(e.contains("bağlanılamadı"));
    }
}
