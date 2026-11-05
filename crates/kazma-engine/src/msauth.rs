//! Microsoft hesabıyla giriş (cihaz kodu akışı): Microsoft → Xbox Live → XSTS → Minecraft hizmetleri.
//! Kullanıcı microsoft.com/link adresine kodu girer; launcher şifreyi hiç görmez.
//! Yenileme anahtarı diske yazılır; Windows'ta DPAPI ile (yalnızca bu kullanıcı çözebilir) şifrelenir.
//! Ağ işleri `Http` arayüzünden geçer, böylece tüm akış sahte sunucuyla testlenir.

use crate::net::*;
#[cfg(windows)]
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Mojang'ın onayladığı Azure uygulamasının kimliği. Derlemede `KAZMA_MS_CLIENT_ID` ile de verilebilir;
/// ikisi de boşsa kullanıcı ayarlardan girer.
pub const DEFAULT_CLIENT_ID: &str = "0c49a09f-8cb7-4c96-9fbf-80be097f45bb";

const DEVICE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";

/// Azure uygulama kimliği bir GUID'dir (8-4-4-4-12 onaltılık).
pub fn client_id_ok(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Kullanılacak kimlik: kullanıcının girdiği, yoksa derlemeye gömülü olan.
pub fn client_id(user: Option<&str>) -> Option<String> {
    // Derleme ortamında değişken boş tanımlıysa (CI'da tanımsız depo değişkeni) gömülü kimliğe düşülür.
    let built = option_env!("KAZMA_MS_CLIENT_ID").filter(|v| client_id_ok(v)).unwrap_or(DEFAULT_CLIENT_ID);
    user.filter(|u| client_id_ok(u)).map(String::from).or_else(|| Some(built.to_string()).filter(|b| client_id_ok(b)))
}

// ---------- ağ arayüzü ----------

pub trait Http {
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> R<(u16, String)>;
    fn post_json(&self, url: &str, body: &str) -> R<(u16, String)>;
    fn get_bearer(&self, url: &str, token: &str) -> R<(u16, String)>;
}

pub struct Live;

fn into_resp(r: Result<ureq::Response, ureq::Error>) -> R<(u16, String)> {
    let read = |resp: ureq::Response| {
        let code = resp.status();
        let mut b = Vec::new();
        resp.into_reader().take(1 << 20).read_to_end(&mut b).map_err(|e| e.to_string())?;
        Ok((code, String::from_utf8_lossy(&b).into_owned()))
    };
    match r {
        Ok(resp) => read(resp),
        Err(ureq::Error::Status(_, resp)) => read(resp),
        Err(e) => Err(format!("Bağlantı kurulamadı: {e}")),
    }
}

impl Http for Live {
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> R<(u16, String)> {
        into_resp(agent().post(url).send_form(form))
    }
    fn post_json(&self, url: &str, body: &str) -> R<(u16, String)> {
        into_resp(agent().post(url).set("Content-Type", "application/json").set("Accept", "application/json").send_string(body))
    }
    fn get_bearer(&self, url: &str, token: &str) -> R<(u16, String)> {
        into_resp(agent().get(url).set("Authorization", &format!("Bearer {token}")).set("Accept", "application/json").call())
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

// ---------- adımlar ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip)]
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Kullanıcının tarayıcıda açacağı adres yalnızca Microsoft'a ait olmalı.
pub fn verification_ok(u: &str) -> bool {
    url::Url::parse(u)
        .map(|u| u.scheme() == "https" && matches!(u.host_str(), Some("www.microsoft.com" | "microsoft.com" | "login.microsoftonline.com" | "login.live.com" | "aka.ms")))
        .unwrap_or(false)
}

pub fn start_device(h: &dyn Http, client: &str) -> R<DeviceCode> {
    if !client_id_ok(client) {
        return Err("Azure uygulama kimliği geçersiz.".into());
    }
    let (code, body) = h.post_form(DEVICE_URL, &[("client_id", client), ("scope", SCOPE)])?;
    let v = json_of(&body);
    if code != 200 {
        let d = s(&v, "error_description");
        return Err(format!("Microsoft giriş kodu vermedi (HTTP {code}). {}", d.lines().next().unwrap_or("")));
    }
    let uri = s(&v, "verification_uri");
    if !verification_ok(&uri) {
        return Err("Microsoft beklenmeyen bir doğrulama adresi verdi; işlem durduruldu.".into());
    }
    let dc = DeviceCode { user_code: s(&v, "user_code"), verification_uri: uri, device_code: s(&v, "device_code"), interval: v.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 30), expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(900).min(1800) };
    if dc.user_code.is_empty() || dc.device_code.is_empty() {
        return Err("Microsoft'un yanıtı eksik.".into());
    }
    Ok(dc)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MsTokens {
    pub access: String,
    pub refresh: String,
}

fn tokens_of(v: &Value) -> R<MsTokens> {
    let (a, r) = (s(v, "access_token"), s(v, "refresh_token"));
    if a.is_empty() || r.is_empty() {
        return Err("Microsoft'un yanıtında erişim anahtarı yok.".into());
    }
    Ok(MsTokens { access: a, refresh: r })
}

/// Kullanıcı kodu girene kadar bekler. `sleep` ve `cancelled` testte sahtelenir.
pub fn poll_device(h: &dyn Http, client: &str, dc: &DeviceCode, sleep: &dyn Fn(u64), cancelled: &dyn Fn() -> bool) -> R<MsTokens> {
    let mut interval = dc.interval;
    let mut waited = 0u64;
    while waited <= dc.expires_in {
        if cancelled() {
            return Err("Giriş iptal edildi.".into());
        }
        sleep(interval);
        waited += interval;
        let (_, body) = h.post_form(TOKEN_URL, &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", client), ("device_code", &dc.device_code)])?;
        let v = json_of(&body);
        match s(&v, "error").as_str() {
            "" => return tokens_of(&v),
            "authorization_pending" => {}
            "slow_down" => interval = (interval + 5).min(30),
            "authorization_declined" | "access_denied" => return Err("Giriş reddedildi.".into()),
            "expired_token" => return Err("Kodun süresi doldu; yeniden dene.".into()),
            "invalid_client" | "unauthorized_client" => return Err("Bu Azure uygulaması genel istemci (public client) akışına izin vermiyor. Azure'da Authentication → Allow public client flows: Evet yap.".into()),
            other => return Err(format!("Microsoft girişi başarısız: {other} {}", s(&v, "error_description").lines().next().unwrap_or(""))),
        }
    }
    Err("Kodun süresi doldu; yeniden dene.".into())
}

pub fn refresh_tokens(h: &dyn Http, client: &str, refresh: &str) -> R<MsTokens> {
    let (_, body) = h.post_form(TOKEN_URL, &[("grant_type", "refresh_token"), ("client_id", client), ("refresh_token", refresh), ("scope", SCOPE)])?;
    let v = json_of(&body);
    match s(&v, "error").as_str() {
        "" => tokens_of(&v),
        "invalid_grant" | "interaction_required" => Err("Microsoft oturumun süresi dolmuş. Hesap kutusundan yeniden giriş yap.".into()),
        other => Err(format!("Oturum yenilenemedi: {other}")),
    }
}

fn xsts_message(x: u64) -> String {
    match x {
        2148916233 => "Bu Microsoft hesabının Xbox profili yok. xbox.com'dan bir Xbox profili oluşturup yeniden dene.".into(),
        2148916235 => "Xbox Live bulunduğun ülkede kullanılamıyor.".into(),
        2148916236 | 2148916237 => "Hesabın yaş doğrulaması gerekiyor (xbox.com).".into(),
        2148916238 => "Bu hesap çocuk hesabı; bir aile grubuna eklenip Xbox izni verilmeli (account.microsoft.com/family).".into(),
        n => format!("Xbox doğrulaması reddedildi (kod {n})."),
    }
}

pub struct McLogin {
    pub token: String,
    pub expires_in: u64,
}

/// Microsoft erişim anahtarı → Xbox Live → XSTS → Minecraft erişim anahtarı.
pub fn exchange(h: &dyn Http, ms_access: &str) -> R<McLogin> {
    let xbl_req = json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={ms_access}")},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"});
    let (c, b) = h.post_json("https://user.auth.xboxlive.com/user/authenticate", &xbl_req.to_string())?;
    let xbl = json_of(&b);
    if c != 200 || s(&xbl, "Token").is_empty() {
        return Err(format!("Xbox Live girişi başarısız (HTTP {c})."));
    }
    let xsts_req = json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[s(&xbl, "Token")]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"});
    let (c, b) = h.post_json("https://xsts.auth.xboxlive.com/xsts/authorize", &xsts_req.to_string())?;
    let xsts = json_of(&b);
    if c != 200 {
        return Err(xsts.get("XErr").and_then(Value::as_u64).map(xsts_message).unwrap_or_else(|| format!("Xbox yetkilendirmesi başarısız (HTTP {c}).")));
    }
    let uhs = xsts.pointer("/DisplayClaims/xui/0/uhs").and_then(Value::as_str).unwrap_or("").to_string();
    let xtoken = s(&xsts, "Token");
    if uhs.is_empty() || xtoken.is_empty() || !uhs.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Xbox yanıtı beklenen biçimde değil.".into());
    }
    let (c, b) = h.post_json("https://api.minecraftservices.com/authentication/login_with_xbox", &json!({"identityToken":format!("XBL3.0 x={uhs};{xtoken}")}).to_string())?;
    let mc = json_of(&b);
    if c == 403 {
        return Err("Minecraft hizmeti bu uygulamayı reddetti (Invalid app registration). Mojang onayı bu Azure uygulama kimliği için henüz geçerli değil.".into());
    }
    if c != 200 || s(&mc, "access_token").is_empty() {
        return Err(format!("Minecraft girişi başarısız (HTTP {c})."));
    }
    Ok(McLogin { token: s(&mc, "access_token"), expires_in: mc.get("expires_in").and_then(Value::as_u64).unwrap_or(3600) })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Profile {
    pub name: String,
    /// Çizgili UUID.
    pub uuid: String,
}

fn dash(id: &str) -> Option<String> {
    let i = id.replace('-', "").to_ascii_lowercase();
    (i.len() == 32 && i.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("{}-{}-{}-{}-{}", &i[0..8], &i[8..12], &i[12..16], &i[16..20], &i[20..32]))
}

pub fn profile(h: &dyn Http, mc_token: &str) -> R<Profile> {
    let (c, b) = h.get_bearer("https://api.minecraftservices.com/minecraft/profile", mc_token)?;
    if c == 404 {
        return Err("Bu Microsoft hesabında Minecraft Java Edition yok. Oyunu satın alınca (ya da Game Pass ile) tekrar dene.".into());
    }
    let v = json_of(&b);
    let (name, id) = (s(&v, "name"), s(&v, "id"));
    let uuid = dash(&id).ok_or_else(|| format!("Minecraft profili okunamadı (HTTP {c})."))?;
    kazma_core::account::validate_player_name(&name).map_err(|e| e.to_string())?;
    Ok(Profile { name, uuid })
}

// ---------- saklama ----------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Stored {
    pub name: String,
    pub uuid: String,
    pub refresh_token: String,
    pub mc_token: String,
    pub mc_expires: u64,
}

fn store_path(root: &Path) -> PathBuf {
    root.join("accounts").join("microsoft.bin")
}

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> R<Vec<u8>> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let op = if protect { "Protect" } else { "Unprotect" };
    let script = format!("Add-Type -AssemblyName System.Security; $b=[Convert]::FromBase64String([Console]::In.ReadToEnd().Trim()); [Convert]::ToBase64String([Security.Cryptography.ProtectedData]::{op}($b,$null,'CurrentUser'))");
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Windows veri koruması başlatılamadı: {e}"))?;
    child.stdin.take().ok_or("stdin yok")?.write_all(base64::engine::general_purpose::STANDARD.encode(data).as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("Hesap bilgisi Windows tarafından çözülemedi (başka bir kullanıcıyla mı kaydedildi?). Yeniden giriş yap.".into());
    }
    base64::engine::general_purpose::STANDARD.decode(String::from_utf8_lossy(&out.stdout).trim()).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn dpapi(data: &[u8], _protect: bool) -> R<Vec<u8>> {
    Ok(data.to_vec())
}

pub fn save(root: &Path, st: &Stored) -> R<()> {
    let raw = serde_json::to_vec(st).map_err(|e| e.to_string())?;
    write_atomic(&store_path(root), &dpapi(&raw, true)?)
}

pub fn load(root: &Path) -> R<Stored> {
    let raw = std::fs::read(store_path(root)).map_err(|_| "Microsoft hesabı bağlı değil.".to_string())?;
    serde_json::from_slice(&dpapi(&raw, false)?).map_err(|_| "Kayıtlı hesap bilgisi okunamadı; yeniden giriş yap.".to_string())
}

pub fn forget(root: &Path) {
    let _ = std::fs::remove_file(store_path(root));
}

pub fn signed_in(root: &Path) -> bool {
    store_path(root).is_file()
}

/// Microsoft ve Minecraft anahtarlarını baştan alır, profili okur ve kaydeder.
fn complete(h: &dyn Http, root: &Path, ms: MsTokens) -> R<Profile> {
    let mc = exchange(h, &ms.access)?;
    let p = profile(h, &mc.token)?;
    save(root, &Stored { name: p.name.clone(), uuid: p.uuid.clone(), refresh_token: ms.refresh, mc_token: mc.token, mc_expires: now() + mc.expires_in })?;
    Ok(p)
}

/// Oyun başlatılırken: geçerli Minecraft anahtarı (süresi yaklaşmışsa yenilenir).
#[derive(Debug)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub token: String,
}

pub fn session(h: &dyn Http, root: &Path, client: &str) -> R<Session> {
    let st = load(root)?;
    if st.mc_expires > now() + 300 && !st.mc_token.is_empty() {
        return Ok(Session { name: st.name, uuid: st.uuid, token: st.mc_token });
    }
    let ms = refresh_tokens(h, client, &st.refresh_token)?;
    let p = complete(h, root, ms)?;
    let st = load(root)?;
    Ok(Session { name: p.name, uuid: p.uuid, token: st.mc_token })
}

// ---------- arayüz akışı: başlat / bekle / iptal ----------

struct Pending {
    dc: DeviceCode,
    client: String,
    cancel: Arc<AtomicBool>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

pub fn begin(client: &str) -> R<DeviceCode> {
    begin_with(&Live, client)
}

pub fn begin_with(h: &dyn Http, client: &str) -> R<DeviceCode> {
    let dc = start_device(h, client)?;
    if let Some(old) = PENDING.lock().unwrap().replace(Pending { dc: dc.clone(), client: client.to_string(), cancel: Arc::new(AtomicBool::new(false)) }) {
        old.cancel.store(true, Ordering::SeqCst);
    }
    Ok(dc)
}

pub fn pending_uri() -> Option<String> {
    PENDING.lock().unwrap().as_ref().map(|p| p.dc.verification_uri.clone())
}

pub fn cancel() {
    if let Some(p) = PENDING.lock().unwrap().take() {
        p.cancel.store(true, Ordering::SeqCst);
    }
}

/// Kullanıcı tarayıcıda kodu girene kadar bekler (engelleyici; ayrı iş parçacığında çağrılmalı).
pub fn finish(root: &Path) -> R<Profile> {
    finish_with(&Live, root, &|s| std::thread::sleep(std::time::Duration::from_secs(s)))
}

pub fn finish_with(h: &dyn Http, root: &Path, sleep: &dyn Fn(u64)) -> R<Profile> {
    let (dc, client, cancel) = {
        let g = PENDING.lock().unwrap();
        let p = g.as_ref().ok_or("Önce giriş kodu al.")?;
        (p.dc.clone(), p.client.clone(), p.cancel.clone())
    };
    let r = poll_device(h, &client, &dc, sleep, &|| cancel.load(Ordering::SeqCst)).and_then(|ms| complete(h, root, ms));
    let mut g = PENDING.lock().unwrap();
    if g.as_ref().map_or(false, |p| Arc::ptr_eq(&p.cancel, &cancel)) {
        *g = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const CID: &str = "12345678-90ab-cdef-1234-567890abcdef";

    struct Fake<F: Fn(&str, &str) -> R<(u16, String)>>(F, RefCell<Vec<String>>);
    impl<F: Fn(&str, &str) -> R<(u16, String)>> Http for Fake<F> {
        fn post_form(&self, url: &str, form: &[(&str, &str)]) -> R<(u16, String)> {
            let b = form.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
            self.1.borrow_mut().push(url.to_string());
            (self.0)(url, &b)
        }
        fn post_json(&self, url: &str, body: &str) -> R<(u16, String)> {
            self.1.borrow_mut().push(url.to_string());
            (self.0)(url, body)
        }
        fn get_bearer(&self, url: &str, token: &str) -> R<(u16, String)> {
            self.1.borrow_mut().push(url.to_string());
            (self.0)(url, token)
        }
    }

    fn happy(url: &str, body: &str) -> R<(u16, String)> {
        let ok = |s: String| Ok((200, s));
        match url {
            DEVICE_URL => ok(r#"{"user_code":"AB12CD","device_code":"dev","verification_uri":"https://www.microsoft.com/link","expires_in":900,"interval":5}"#.into()),
            TOKEN_URL => ok(r#"{"access_token":"msacc","refresh_token":"msref","expires_in":3600}"#.into()),
            "https://user.auth.xboxlive.com/user/authenticate" => {
                assert!(body.contains("d=msacc"), "RpsTicket d= önekiyle gider");
                ok(r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"999"}]}}"#.into())
            }
            "https://xsts.auth.xboxlive.com/xsts/authorize" => ok(r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"999"}]}}"#.into()),
            "https://api.minecraftservices.com/authentication/login_with_xbox" => {
                assert!(body.contains("XBL3.0 x=999;xsts"));
                ok(r#"{"access_token":"mctok","expires_in":86400}"#.into())
            }
            "https://api.minecraftservices.com/minecraft/profile" => {
                assert_eq!(body, "mctok");
                ok(r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#.into())
            }
            u => Err(format!("beklenmeyen adres {u}")),
        }
    }

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kazma-ms-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn client_id_shape() {
        assert!(client_id_ok(CID));
        assert!(!client_id_ok("12345678-90ab-cdef-1234-567890abcde"));
        assert!(!client_id_ok("zzzzzzzz-90ab-cdef-1234-567890abcdef"));
        assert_eq!(client_id(Some(CID)).as_deref(), Some(CID));
        assert_eq!(client_id(Some("kötü")), client_id(None), "geçersiz giriş yok sayılır");
    }

    #[test]
    fn embedded_client_id_is_valid() {
        assert!(client_id_ok(DEFAULT_CLIENT_ID), "gömülü Azure kimliği geçerli biçimde olmalı");
        assert_eq!(client_id(None).as_deref(), Some(DEFAULT_CLIENT_ID));
    }

    #[test]
    fn verification_only_microsoft() {
        assert!(verification_ok("https://www.microsoft.com/link"));
        assert!(verification_ok("https://login.live.com/oauth20_remoteconnect.srf"));
        assert!(!verification_ok("http://www.microsoft.com/link"));
        assert!(!verification_ok("https://microsoft.com.evil.example/link"));
        assert!(!verification_ok("https://evil.example/www.microsoft.com"));
    }

    #[test]
    fn whole_login_flow_saves_profile_and_session_reuses_token() {
        let root = tmp("flow");
        let h = Fake(happy, RefCell::new(vec![]));
        let dc = start_device(&h, CID).unwrap();
        assert_eq!(dc.user_code, "AB12CD");
        let ms = poll_device(&h, CID, &dc, &|_| {}, &|| false).unwrap();
        let p = complete(&h, &root, ms).unwrap();
        assert_eq!((p.name.as_str(), p.uuid.as_str()), ("Notch", "069a79f4-44e9-4726-a5be-fca90e38aaf5"));
        assert!(signed_in(&root));
        let before = h.1.borrow().len();
        let s = session(&h, &root, CID).unwrap();
        assert_eq!((s.token.as_str(), s.name.as_str()), ("mctok", "Notch"));
        assert_eq!(h.1.borrow().len(), before, "süresi dolmamış anahtar için ağa gidilmez");
        forget(&root);
        assert!(!signed_in(&root) && session(&h, &root, CID).is_err());
    }

    #[test]
    fn expired_token_is_refreshed_and_refresh_token_rotates() {
        let root = tmp("refresh");
        save(&root, &Stored { name: "Notch".into(), uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(), refresh_token: "old".into(), mc_token: "stale".into(), mc_expires: 1 }).unwrap();
        let h = Fake(
            |u: &str, b: &str| {
                if u == TOKEN_URL {
                    assert!(b.contains("grant_type=refresh_token") && b.contains("refresh_token=old"));
                    return Ok((200, r#"{"access_token":"msacc","refresh_token":"newref"}"#.into()));
                }
                happy(u, b)
            },
            RefCell::new(vec![]),
        );
        let s = session(&h, &root, CID).unwrap();
        assert_eq!(s.token, "mctok");
        assert_eq!(load(&root).unwrap().refresh_token, "newref");
        let bad = Fake(|_: &str, _: &str| Ok((400, r#"{"error":"invalid_grant"}"#.into())), RefCell::new(vec![]));
        save(&root, &Stored { name: "N".into(), uuid: "u".into(), refresh_token: "old".into(), mc_token: String::new(), mc_expires: 0 }).unwrap();
        assert!(session(&bad, &root, CID).unwrap_err().contains("yeniden giriş"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn polling_waits_slows_down_and_stops() {
        let dc = DeviceCode { user_code: "X".into(), verification_uri: "https://www.microsoft.com/link".into(), device_code: "d".into(), interval: 5, expires_in: 900 };
        let n = Cell::new(0);
        let h = Fake(
            |_: &str, _: &str| {
                n.set(n.get() + 1);
                Ok(match n.get() {
                    1 => (400, r#"{"error":"authorization_pending"}"#.into()),
                    2 => (400, r#"{"error":"slow_down"}"#.into()),
                    _ => (200, r#"{"access_token":"a","refresh_token":"r"}"#.into()),
                })
            },
            RefCell::new(vec![]),
        );
        let slept = RefCell::new(vec![]);
        let t = poll_device(&h, CID, &dc, &|s| slept.borrow_mut().push(s), &|| false).unwrap();
        assert_eq!(t.access, "a");
        assert_eq!(*slept.borrow(), vec![5, 5, 10], "slow_down aralığı büyütür");
        let denied = Fake(|_: &str, _: &str| Ok((400, r#"{"error":"authorization_declined"}"#.into())), RefCell::new(vec![]));
        assert!(poll_device(&denied, CID, &dc, &|_| {}, &|| false).unwrap_err().contains("reddedildi"));
        let pending = Fake(|_: &str, _: &str| Ok((400, r#"{"error":"authorization_pending"}"#.into())), RefCell::new(vec![]));
        assert!(poll_device(&pending, CID, &dc, &|_| {}, &|| false).unwrap_err().contains("süresi"), "süre dolunca durur");
        assert!(poll_device(&pending, CID, &dc, &|_| {}, &|| true).unwrap_err().contains("iptal"));
    }

    #[test]
    fn explains_xbox_and_mojang_failures() {
        let xerr = |code: u64| {
            Fake(
                move |u: &str, b: &str| {
                    if u.contains("xsts") {
                        return Ok((401, format!(r#"{{"XErr":{code}}}"#)));
                    }
                    happy(u, b)
                },
                RefCell::new(vec![]),
            )
        };
        assert!(exchange(&xerr(2148916233), "msacc").err().unwrap().contains("Xbox profili yok"));
        assert!(exchange(&xerr(2148916238), "msacc").err().unwrap().contains("çocuk"));
        let rejected = Fake(|u: &str, b: &str| if u.contains("login_with_xbox") { Ok((403, "{}".into())) } else { happy(u, b) }, RefCell::new(vec![]));
        assert!(exchange(&rejected, "msacc").err().unwrap().contains("Invalid app registration"));
        let nogame = Fake(|_: &str, _: &str| Ok((404, "{}".into())), RefCell::new(vec![]));
        assert!(profile(&nogame, "t").unwrap_err().contains("Java Edition yok"));
    }

    #[test]
    fn device_start_rejects_foreign_verification_page() {
        let h = Fake(|_: &str, _: &str| Ok((200, r#"{"user_code":"A","device_code":"d","verification_uri":"https://evil.example/link"}"#.into())), RefCell::new(vec![]));
        assert!(start_device(&h, CID).unwrap_err().contains("beklenmeyen"));
        assert!(start_device(&h, "kötü").is_err());
    }

    #[test]
    fn ui_flow_via_pending_state() {
        let root = tmp("ui");
        let h = Fake(happy, RefCell::new(vec![]));
        assert!(finish_with(&h, &root, &|_| {}).is_err() || pending_uri().is_some());
        let dc = begin_with(&h, CID).unwrap();
        assert_eq!(pending_uri().as_deref(), Some(dc.verification_uri.as_str()));
        let p = finish_with(&h, &root, &|_| {}).unwrap();
        assert_eq!(p.name, "Notch");
        assert!(pending_uri().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
