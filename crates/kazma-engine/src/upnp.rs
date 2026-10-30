//! Router'da otomatik port yönlendirme (UPnP IGD). Yalnızca yerel ağdaki ağ geçidiyle konuşulur:
//! SSDP yanıtındaki adres özel (LAN) bir IP olmalı ve yanıtı gönderen cihazla aynı olmalıdır.

use crate::net::R;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Gateway {
    pub control_url: String,
    pub service_type: String,
    pub local_ip: Ipv4Addr,
}

pub fn is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_link_local(),
        _ => false,
    }
}

/// Router'ın bildirdiği dış IP gerçekten internete açık mı? (özel / CGNAT 100.64.0.0/10 ise tünel gerekir)
pub fn is_public_ipv4(s: &str) -> bool {
    match s.parse::<Ipv4Addr>() {
        Ok(v) => {
            let o = v.octets();
            !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_documentation() || (o[0] == 100 && (64..=127).contains(&o[1])))
        }
        Err(_) => false,
    }
}

fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&format!("</{name}>"))? + a;
    Some(xml[a..b].trim().to_string())
}

/// Aygıt tanımından WANIPConnection / WANPPPConnection servisinin controlURL'si.
pub fn parse_description(xml: &str) -> Option<(String, String)> {
    let mut rest = xml;
    while let Some(i) = rest.find("<service>") {
        let end = rest[i..].find("</service>")? + i;
        let block = &rest[i..end];
        if let (Some(t), Some(c)) = (tag(block, "serviceType"), tag(block, "controlURL")) {
            if t.contains(":WANIPConnection:") || t.contains(":WANPPPConnection:") {
                return Some((t, c));
            }
        }
        rest = &rest[end..];
    }
    None
}

fn join_url(location: &str, control: &str) -> R<String> {
    let base = url::Url::parse(location).map_err(|e| e.to_string())?;
    let u = base.join(control).map_err(|e| e.to_string())?;
    if u.host_str() != base.host_str() {
        return Err("Router farklı bir adrese yönlendirmeye çalıştı, reddedildi.".into());
    }
    Ok(u.to_string())
}

fn http_get(url: &str) -> R<String> {
    let resp = ureq::AgentBuilder::new().timeout(Duration::from_secs(4)).build().get(url).call().map_err(|e| format!("Router tanımı alınamadı: {e}"))?;
    let mut s = String::new();
    resp.into_reader().take(256 * 1024).read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn envelope(service: &str, action: &str, args: &[(&str, &str)]) -> String {
    let inner: String = args.iter().map(|(k, v)| format!("<{k}>{}</{k}>", xml_escape(v))).collect();
    format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">{inner}</u:{action}></s:Body></s:Envelope>")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn soap(gw: &Gateway, action: &str, args: &[(&str, &str)]) -> R<String> {
    let body = envelope(&gw.service_type, action, args);
    let r = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()
        .post(&gw.control_url)
        .set("Content-Type", "text/xml; charset=\"utf-8\"")
        .set("SOAPAction", &format!("\"{}#{}\"", gw.service_type, action))
        .send_string(&body);
    match r {
        Ok(resp) => {
            let mut s = String::new();
            resp.into_reader().take(64 * 1024).read_to_string(&mut s).map_err(|e| e.to_string())?;
            Ok(s)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let mut s = String::new();
            let _ = resp.into_reader().take(16 * 1024).read_to_string(&mut s);
            Err(format!("Router isteği reddetti ({code}){}", tag(&s, "errorDescription").map(|d| format!(": {d}")).unwrap_or_default()))
        }
        Err(e) => Err(format!("Router'a ulaşılamadı: {e}")),
    }
}

/// Bir LOCATION adresinden ağ geçidini kurar. `allow_loopback` yalnızca testlerde doğrudur.
pub fn from_location(location: &str, local_ip: Ipv4Addr, allow_loopback: bool) -> R<Gateway> {
    let u = url::Url::parse(location).map_err(|e| e.to_string())?;
    let host: IpAddr = u.host_str().and_then(|h| h.parse().ok()).ok_or("Router adresi bir IP değil.")?;
    if !(is_lan(&host) || (allow_loopback && host.is_loopback())) || u.scheme() != "http" {
        return Err("Router adresi yerel ağda değil, reddedildi.".into());
    }
    let (service_type, control) = parse_description(&http_get(location)?).ok_or("Router UPnP port yönlendirmeyi desteklemiyor ya da kapalı.")?;
    Ok(Gateway { control_url: join_url(location, &control)?, service_type, local_ip })
}

const SEARCH_TARGETS: [&str; 4] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "upnp:rootdevice",
];

/// Özel (LAN) IPv4 adresine sahip tüm ağ arayüzleri; sanal bağdaştırıcılar (VPN, WSL, Hyper-V) arasında doğru olanı bulmak için hepsi denenir.
fn lan_interfaces() -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .map(|l| l.into_iter().filter(|i| !i.is_loopback()).filter_map(|i| match i.ip() { IpAddr::V4(a) if a.is_private() => Some(a), _ => None }).collect())
        .unwrap_or_default();
    v.sort();
    v.dedup();
    v
}

/// SSDP ile yerel ağdaki router'ı bulur. Her LAN arayüzünden, birkaç arama türüyle denenir.
pub fn discover(timeout: Duration) -> R<Gateway> {
    let ifaces = lan_interfaces();
    if ifaces.is_empty() {
        return Err("Yerel ağ bağlantısı bulunamadı (Wi-Fi ya da Ethernet bağlı mı?).".into());
    }
    let socks: Vec<(Ipv4Addr, UdpSocket)> = ifaces
        .iter()
        .filter_map(|ip| {
            let s = UdpSocket::bind(SocketAddr::new(IpAddr::V4(*ip), 0)).ok()?;
            s.set_read_timeout(Some(Duration::from_millis(250))).ok()?;
            Some((*ip, s))
        })
        .collect();
    let t0 = std::time::Instant::now();
    let mut sent_rounds = 0;
    let mut buf = [0u8; 2048];
    let mut tried: Vec<String> = Vec::new();
    while t0.elapsed() < timeout {
        if sent_rounds < 2 && t0.elapsed() >= timeout / 2 * sent_rounds {
            for (_, s) in &socks {
                for st in SEARCH_TARGETS {
                    let msg = format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\n\r\n");
                    let _ = s.send_to(msg.as_bytes(), "239.255.255.250:1900");
                }
            }
            sent_rounds += 1;
        }
        for (local_ip, s) in &socks {
            let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
            let txt = String::from_utf8_lossy(&buf[..n]).to_string();
            let loc = txt.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().to_string()));
            let Some(loc) = loc else { continue };
            // Yanıtı gönderen cihaz ile LOCATION'daki cihaz aynı olmalı.
            let (IpAddr::V4(gw_ip), Ok(u)) = (from.ip(), url::Url::parse(&loc)) else { continue };
            if u.host_str() != Some(&gw_ip.to_string()) || tried.contains(&loc) {
                continue;
            }
            tried.push(loc.clone());
            if let Ok(g) = from_location(&loc, *local_ip, false) {
                return Ok(g);
            }
        }
    }
    Err(if tried.is_empty() {
        "Router yanıt vermedi. Router'da UPnP kapalı olabilir, Windows Güvenlik Duvarı Kazma'yı engelliyor olabilir ya da router'ın yerel ağında değilsin.".into()
    } else {
        "Router bulundu ama UPnP port yönlendirme sunmuyor ya da kapalı.".into()
    })
}

pub fn external_ip(gw: &Gateway) -> R<String> {
    tag(&soap(gw, "GetExternalIPAddress", &[])?, "NewExternalIPAddress").filter(|s| !s.is_empty()).ok_or_else(|| "Router dış IP'yi bildirmedi.".into())
}

pub fn add_mapping(gw: &Gateway, port: u16, description: &str, lease_secs: u32) -> R<()> {
    let (p, lease, local) = (port.to_string(), lease_secs.to_string(), gw.local_ip.to_string());
    soap(
        gw,
        "AddPortMapping",
        &[("NewRemoteHost", ""), ("NewExternalPort", &p), ("NewProtocol", "TCP"), ("NewInternalPort", &p), ("NewInternalClient", &local), ("NewEnabled", "1"), ("NewPortMappingDescription", description), ("NewLeaseDuration", &lease)],
    )
    .map(|_| ())
}

pub fn delete_mapping(gw: &Gateway, port: u16) -> R<()> {
    let p = port.to_string();
    soap(gw, "DeletePortMapping", &[("NewRemoteHost", ""), ("NewExternalPort", &p), ("NewProtocol", "TCP")]).map(|_| ())
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Opened {
    pub external_ip: String,
    pub public: bool,
    pub address: String,
}

/// Router'ı bulur, portu açar, dış IP'yi doğrular. `public=false` ise (CGNAT/çift NAT) port açık olsa da internetten erişilemez.
pub fn open_port(port: u16, name: &str) -> R<(Gateway, Opened)> {
    let gw = discover(Duration::from_secs(6))?;
    add_mapping(&gw, port, name, 0).or_else(|_| add_mapping(&gw, port, name, 3600))?;
    let ip = external_ip(&gw).unwrap_or_default();
    let public = is_public_ipv4(&ip);
    Ok((gw.clone(), Opened { address: if ip.is_empty() { String::new() } else { format!("{ip}:{port}") }, external_ip: ip, public }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const DESC: &str = r#"<root><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3</controlURL></service></serviceList><deviceList><device><deviceList><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service></serviceList></device></deviceList></device></deviceList></device></root>"#;

    /// Sahte router: açıklama + SOAP; gelen istekleri kaydeder.
    fn fake_router(log: Arc<Mutex<Vec<String>>>) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                r.read_line(&mut first).unwrap();
                let mut len = 0usize;
                let mut action = String::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    let low = h.to_ascii_lowercase();
                    if let Some(v) = low.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if low.starts_with("soapaction:") {
                        action = h.split('#').nth(1).unwrap_or("").trim_end_matches(['"', '\r', '\n']).to_string();
                    }
                }
                let mut body = vec![0u8; len];
                std::io::Read::read_exact(&mut r, &mut body).unwrap();
                let body = String::from_utf8_lossy(&body).to_string();
                let (status, out) = if first.starts_with("GET /desc") {
                    ("200 OK", DESC.to_string())
                } else if action == "GetExternalIPAddress" {
                    ("200 OK", "<NewExternalIPAddress>203.0.113.9</NewExternalIPAddress>".to_string())
                } else if body.contains("<NewExternalPort>1</NewExternalPort>") {
                    ("500 Internal Server Error", "<errorDescription>ConflictInMappingEntry</errorDescription>".to_string())
                } else {
                    log.lock().unwrap().push(format!("{action}:{}", body.contains("<NewInternalPort>25565</NewInternalPort>")));
                    ("200 OK", "<ok/>".to_string())
                };
                write!(&s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).unwrap();
            }
        });
        port
    }

    #[test]
    fn description_parsing_and_url_join() {
        let (t, c) = parse_description(DESC).unwrap();
        assert!(t.ends_with("WANIPConnection:1") && c == "/ctl/IPConn");
        assert!(parse_description("<root/>").is_none());
        assert_eq!(join_url("http://192.168.1.1:5000/desc.xml", "/ctl").unwrap(), "http://192.168.1.1:5000/ctl");
        assert!(join_url("http://192.168.1.1:5000/desc.xml", "http://evil.example.com/ctl").is_err(), "başka sunucuya yönlendirme reddedilir");
    }

    #[test]
    fn only_lan_gateways_are_accepted() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        for bad in ["http://8.8.8.8/desc", "http://example.com/desc", "https://192.168.1.1/desc", "http://127.0.0.1/desc"] {
            assert!(from_location(bad, ip, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn public_ip_detection() {
        for (ip, want) in [("203.0.113.9", false), ("85.100.20.4", true), ("100.64.1.1", false), ("100.127.0.1", false), ("100.128.0.1", true), ("192.168.1.2", false), ("10.0.0.5", false), ("", false), ("x", false)] {
            assert_eq!(is_public_ipv4(ip), want, "{ip}");
        }
    }

    #[test]
    fn full_flow_against_fake_router() {
        let log = Arc::new(Mutex::new(vec![]));
        let port = fake_router(log.clone());
        let gw = from_location(&format!("http://127.0.0.1:{port}/desc"), Ipv4Addr::new(192, 168, 1, 20), true).unwrap();
        assert_eq!(gw.control_url, format!("http://127.0.0.1:{port}/ctl/IPConn"));
        assert_eq!(external_ip(&gw).unwrap(), "203.0.113.9");
        add_mapping(&gw, 25565, "Kazma <test>", 0).unwrap();
        delete_mapping(&gw, 25565).unwrap();
        assert_eq!(*log.lock().unwrap(), ["AddPortMapping:true", "DeletePortMapping:false"]);
        let e = add_mapping(&gw, 1, "x", 0).unwrap_err();
        assert!(e.contains("ConflictInMappingEntry"), "{e}");
    }

    #[test]
    fn xml_values_are_escaped() {
        assert!(envelope("urn:x", "A", &[("K", "a<b>&\"")]).contains("<K>a&lt;b&gt;&amp;&quot;</K>"));
    }
}
