//! Discord Rich Presence ("Kazma Launcher oynuyor"). Discord istemcisinin yerel IPC kanalıyla konuşur; ağ erişimi yoktur.
//! Discord kapalıysa ya da uygulama kimliği yoksa sessizce hiçbir şey yapmaz.

use crate::net::R;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::Mutex;

pub trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub details: String,
    pub state: String,
    pub start: u64,
    pub large_image: String,
    pub large_text: String,
}

pub fn valid_app_id(id: &str) -> bool {
    (15..=22).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

pub fn frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut v = Vec::with_capacity(8 + body.len());
    v.extend_from_slice(&op.to_le_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&body);
    v
}

fn read_frame(p: &mut dyn Read) -> R<(u32, Value)> {
    let mut h = [0u8; 8];
    p.read_exact(&mut h).map_err(|e| e.to_string())?;
    let (op, len) = (u32::from_le_bytes(h[0..4].try_into().unwrap()), u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize);
    if len > 64 * 1024 {
        return Err("Discord yanıtı çok büyük".into());
    }
    let mut b = vec![0u8; len];
    p.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok((op, serde_json::from_slice(&b).unwrap_or(Value::Null)))
}

fn clip(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

pub fn activity_json(a: &Activity) -> Value {
    json!({
        "details": clip(&a.details, 128),
        "state": clip(&a.state, 128),
        "timestamps": { "start": a.start },
        "assets": { "large_image": clip(&a.large_image, 64), "large_text": clip(&a.large_text, 128) }
    })
}

fn set_activity_frame(act: Option<&Activity>, nonce: u32) -> Vec<u8> {
    frame(1, &json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": act.map(activity_json) }, "nonce": nonce.to_string() }))
}

/// El sıkışma: Discord "READY" yanıtı verene kadar bekler.
pub fn handshake(p: &mut dyn Pipe, app_id: &str) -> R<()> {
    p.write_all(&frame(0, &json!({ "v": 1, "client_id": app_id }))).map_err(|e| e.to_string())?;
    let (op, v) = read_frame(p)?;
    if op == 2 || v.get("evt").and_then(Value::as_str) != Some("READY") {
        return Err("Discord bağlantıyı kabul etmedi (uygulama kimliği yanlış olabilir).".into());
    }
    Ok(())
}

pub fn send_activity(p: &mut dyn Pipe, act: Option<&Activity>, nonce: u32) -> R<()> {
    p.write_all(&set_activity_frame(act, nonce)).map_err(|e| e.to_string())?;
    let (_, v) = read_frame(p)?;
    if v.get("evt").and_then(Value::as_str) == Some("ERROR") {
        return Err(v.pointer("/data/message").and_then(Value::as_str).unwrap_or("Discord hata verdi").to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn open_pipe() -> Option<Box<dyn Pipe>> {
    (0..10).find_map(|i| std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")).ok().map(|f| Box::new(f) as Box<dyn Pipe>))
}

#[cfg(unix)]
fn open_pipe() -> Option<Box<dyn Pipe>> {
    let dirs: Vec<String> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var(k).ok()).chain(["/tmp".to_string()]).collect();
    dirs.iter().find_map(|d| (0..10).find_map(|i| std::os::unix::net::UnixStream::connect(format!("{d}/discord-ipc-{i}")).ok().map(|s| Box::new(s) as Box<dyn Pipe>)))
}

struct Conn {
    pipe: Box<dyn Pipe>,
    app_id: String,
    nonce: u32,
}

static CONN: Mutex<Option<Conn>> = Mutex::new(None);

/// Etkinliği ayarlar (`None` = temizle). Hata olursa bağlantı bırakılır; bir sonraki çağrıda yeniden denenir.
pub fn update(app_id: &str, act: Option<&Activity>) -> R<()> {
    if !valid_app_id(app_id) {
        return Err("Discord uygulama kimliği geçersiz.".into());
    }
    let mut g = CONN.lock().unwrap();
    if g.as_ref().map_or(false, |c| c.app_id != app_id) {
        *g = None;
    }
    if g.is_none() {
        let mut pipe = open_pipe().ok_or("Discord açık değil.")?;
        handshake(pipe.as_mut(), app_id)?;
        *g = Some(Conn { pipe, app_id: app_id.to_string(), nonce: 0 });
    }
    let c = g.as_mut().unwrap();
    c.nonce += 1;
    let n = c.nonce;
    let r = send_activity(c.pipe.as_mut(), act, n);
    if r.is_err() {
        *g = None;
    }
    r
}

pub fn clear(app_id: &str) {
    let _ = update(app_id, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_and_framing() {
        assert!(valid_app_id("1234567890123456789") && !valid_app_id("abc") && !valid_app_id("12345678901234x") && !valid_app_id(""));
        let f = frame(1, &json!({"a":1}));
        assert_eq!(&f[0..4], &1u32.to_le_bytes());
        assert_eq!(u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize, f.len() - 8);
        let (op, v) = read_frame(&mut &f[..]).unwrap();
        assert_eq!((op, v["a"].as_i64()), (1, Some(1)));
        assert!(read_frame(&mut &[1u8, 0, 0, 0, 255, 255, 255, 255][..]).is_err(), "dev boyutlu çerçeve reddedilir");
    }

    #[test]
    fn activity_is_sanitized() {
        let a = Activity { details: "x\ny".repeat(100), state: "s".into(), start: 5, large_image: "kazma".into(), large_text: "Kazma Launcher".into() };
        let j = activity_json(&a);
        assert!(!j["details"].as_str().unwrap().contains('\n') && j["details"].as_str().unwrap().chars().count() <= 128);
        assert_eq!(j["timestamps"]["start"], 5);
        assert_eq!(j["assets"]["large_image"], "kazma");
    }

    #[cfg(unix)]
    #[test]
    fn talks_to_a_fake_discord() {
        use std::os::unix::net::UnixListener;
        let path = std::env::temp_dir().join(format!("kz-discord-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let (op, hs) = read_frame(&mut s).unwrap();
            assert_eq!((op, hs["client_id"].as_str()), (0, Some("1234567890123456789")));
            s.write_all(&frame(1, &json!({"cmd":"DISPATCH","evt":"READY"}))).unwrap();
            let mut seen = vec![];
            for _ in 0..2 {
                let (op, v) = read_frame(&mut s).unwrap();
                assert_eq!(op, 1);
                seen.push((v["cmd"].as_str().unwrap().to_string(), v["args"]["activity"].is_null()));
                s.write_all(&frame(1, &json!({"cmd":"SET_ACTIVITY","evt":null}))).unwrap();
            }
            seen
        });
        let mut c = std::os::unix::net::UnixStream::connect(&path).unwrap();
        handshake(&mut c, "1234567890123456789").unwrap();
        let a = Activity { details: "Oynuyor".into(), state: "1.21.1".into(), start: 1, large_image: "kazma".into(), large_text: "K".into() };
        send_activity(&mut c, Some(&a), 1).unwrap();
        send_activity(&mut c, None, 2).unwrap();
        assert_eq!(t.join().unwrap(), [("SET_ACTIVITY".to_string(), false), ("SET_ACTIVITY".to_string(), true)]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn handshake_rejects_errors() {
        let mut bad = std::io::Cursor::new(frame(2, &json!({"code":4000})));
        struct Rw(std::io::Cursor<Vec<u8>>);
        impl Read for Rw { fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { self.0.read(b) } }
        impl Write for Rw { fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { Ok(b.len()) } fn flush(&mut self) -> std::io::Result<()> { Ok(()) } }
        let mut rw = Rw(std::io::Cursor::new(std::mem::take(bad.get_mut())));
        assert!(handshake(&mut rw, "1234567890123456789").is_err());
    }
}
