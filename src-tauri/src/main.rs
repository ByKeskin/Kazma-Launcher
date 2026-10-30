// Release derlemesinde Windows'ta konsol penceresi açılmasın.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri kabuğu: yalnızca ince sarmalayıcılar. Asıl mantık `kazma-engine`'de, orada testli.
//! Arayüze giden hatalar düz Türkçe metindir.

use kazma_engine::report::Reporter;
use kazma_engine::{app, install, modrinth, serverhost, skins, tunnel, upnp};
use serde::Serialize;
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

struct AppState {
    root: PathBuf,
    prepared: Mutex<Option<app::Prepared>>,
    busy: Mutex<bool>,
    hosts: Arc<serverhost::Manager>,
    /// Açık UPnP port yönlendirmeleri (server kimliği → router).
    gateways: Mutex<HashMap<String, (upnp::Gateway, u16)>>,
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    kind: &'static str,
    text: String,
    done: usize,
    total: usize,
}

#[derive(Serialize, Clone)]
struct DoneEvent {
    ok: bool,
    error: Option<String>,
    code: Option<i32>,
    log: Option<String>,
}

/// Motorun ilerleme bildirimlerini arayüze olay olarak iletir.
struct Events(AppHandle);

impl Reporter for Events {
    fn stage(&self, text: &str) {
        let _ = self.0.emit("progress", ProgressEvent { kind: "stage", text: text.to_string(), done: 0, total: 0 });
    }
    fn progress(&self, label: &str, done: usize, total: usize) {
        let _ = self.0.emit("progress", ProgressEvent { kind: "bar", text: label.to_string(), done, total });
    }
}

#[derive(Serialize, Clone)]
struct LogEvent {
    id: String,
    line: String,
}

#[derive(Serialize, Clone)]
struct StateEvent {
    id: String,
    state: String,
}

#[derive(Serialize, Clone)]
struct ClaimEvent {
    id: String,
    url: String,
}

/// Server konsolu satırlarını ve durum değişikliklerini arayüze iletir.
struct ServerEvents(AppHandle);

impl serverhost::Sink for ServerEvents {
    fn line(&self, id: &str, text: &str) {
        let _ = self.0.emit("server-log", LogEvent { id: id.to_string(), line: text.to_string() });
    }
    fn state(&self, id: &str, state: &str) {
        let _ = self.0.emit("server-state", StateEvent { id: id.to_string(), state: state.to_string() });
        if state == "stopped" {
            release_port(&self.0.state::<AppState>(), id);
        }
    }
}

/// Tünel ajanının çıktısı server konsoluna "[tünel]" önekiyle yazılır; hesap bağlama adresi ayrıca bildirilir.
struct TunnelEvents(AppHandle, String);

impl serverhost::Sink for TunnelEvents {
    fn line(&self, _: &str, text: &str) {
        let _ = self.0.emit("server-log", LogEvent { id: self.1.clone(), line: format!("[tünel] {text}") });
        if let Some(url) = tunnel::claim_url_in(text) {
            let _ = self.0.emit("tunnel-claim", ClaimEvent { id: self.1.clone(), url });
        }
    }
    fn state(&self, _: &str, state: &str) {
        let _ = self.0.emit("tunnel-state", StateEvent { id: self.1.clone(), state: state.to_string() });
    }
}

fn release_port(st: &AppState, id: &str) {
    let g = st.gateways.lock().unwrap().remove(id);
    if let Some((gw, port)) = g {
        let _ = upnp::delete_mapping(&gw, port);
    }
}

fn release_all(st: &AppState) {
    let ids: Vec<String> = st.gateways.lock().unwrap().keys().cloned().collect();
    for id in ids {
        release_port(st, &id);
    }
}

fn begin(state: &AppState) -> CmdResult<()> {
    let mut b = state.busy.lock().unwrap();
    if *b {
        return Err("Başka bir işlem sürüyor, bitmesini bekle.".into());
    }
    *b = true;
    Ok(())
}

fn finish(handle: &AppHandle, r: Result<kazma_engine::install::GameExit, String>) {
    let ev = match r {
        Ok(e) => DoneEvent { ok: e.code == 0, error: None, code: Some(e.code), log: Some(e.log.display().to_string()) },
        Err(e) => DoneEvent { ok: false, error: Some(e), code: None, log: None },
    };
    *handle.state::<AppState>().busy.lock().unwrap() = false;
    let _ = handle.emit("launch-done", ev);
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> app::Config {
    app::load_config(&state.root)
}

#[tauri::command]
fn set_player(state: State<'_, AppState>, name: String) -> CmdResult<app::Config> {
    app::set_player(&state.root, &name)
}

/// Instance klasörünü (dünyalar dahil) kalıcı olarak siler. Arayüz önce onay alır.
#[tauri::command]
fn delete_instance(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    app::delete_instance(&state.root, &id)
}

/// Server sahibi: mod klasöründen paylaşılabilir paket kodu üretir. `instance` verilirse onun mods klasörü kullanılır.
#[tauri::command]
async fn make_pack(handle: AppHandle, state: State<'_, AppState>, instance: Option<String>, path: String, name: String, address: String, game: String, loader: String, version: Option<String>) -> CmdResult<kazma_engine::serverkit::PackBuild> {
    let dir = match instance {
        Some(id) => app::instance_dir(&state.root, &id)?.join("mods"),
        None => std::path::PathBuf::from(path),
    };
    tauri::async_runtime::spawn_blocking(move || app::make_pack(&dir, &name, &address, &game, &loader, version, &Events(handle)))
        .await
        .map_err(|e| e.to_string())?
}

/// Instance klasörünü Dosya Gezgini'nde açar.
#[tauri::command]
fn open_instance_folder(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    app::open_instance_folder(&state.root, &id)
}

// ---------- skinler ----------

#[tauri::command]
fn list_skins(state: State<'_, AppState>) -> Vec<skins::SkinInfo> {
    skins::list_skins(&state.root)
}

#[tauri::command]
fn skin_data(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    skins::skin_data_url(&state.root, &id)
}

#[tauri::command]
fn add_skin(state: State<'_, AppState>, name: String, data: String) -> CmdResult<skins::SkinInfo> {
    skins::add_skin_b64(&state.root, &name, &data)
}

#[tauri::command]
async fn fetch_skin(state: State<'_, AppState>, player: String) -> CmdResult<skins::SkinInfo> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || skins::add_from_username(&root, &player)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn skin_sources() -> Vec<skins::SkinSource> {
    skins::trusted_sources()
}

#[tauri::command]
async fn add_skin_link(state: State<'_, AppState>, link: String) -> CmdResult<skins::SkinInfo> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || skins::add_from_link(&root, &link)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_skin_ely(state: State<'_, AppState>, player: String) -> CmdResult<skins::SkinInfo> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || skins::add_from_ely(&root, &player)).await.map_err(|e| e.to_string())?
}

/// Diğer launcher'lardaki oyun klasörleri (dünyalar, profiller).
#[tauri::command]
async fn scan_sources(state: State<'_, AppState>) -> CmdResult<Vec<kazma_engine::importer::Source>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::scan_sources(&root)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn add_source(state: State<'_, AppState>, path: String) -> CmdResult<Vec<kazma_engine::importer::Source>> {
    app::add_source(&state.root, &path)
}

/// Başka launcher'larda bulunan hesap adları ve kimlikleri (belirteç okunmaz).
#[tauri::command]
async fn scan_accounts(state: State<'_, AppState>) -> CmdResult<Vec<kazma_engine::importer::FoundAccount>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::scan_accounts(&root)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn set_identity(state: State<'_, AppState>, name: String, uuid: String) -> CmdResult<app::Config> {
    app::set_identity(&state.root, &name, &uuid)
}

#[tauri::command]
async fn import_run(handle: AppHandle, state: State<'_, AppState>, req: app::ImportRequest) -> CmdResult<app::ImportResult> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::import_run(&root, &req, &Events(handle))).await.map_err(|e| e.to_string())?
}

/// Yalnızca güvenilir kaynak listesindeki adresleri tarayıcıda açar.
#[tauri::command]
fn open_url(url: String) -> CmdResult<()> {
    app::open_trusted_url(&url)
}

#[tauri::command]
fn delete_skin(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let c = app::load_config(&state.root);
    if c.skin.as_deref() == Some(id.as_str()) {
        app::set_skin(&state.root, None)?;
    }
    skins::delete_skin(&state.root, &id)
}

/// `id` boşsa seçim kaldırılır.
#[tauri::command]
fn select_skin(state: State<'_, AppState>, id: Option<String>) -> CmdResult<app::Config> {
    app::set_skin(&state.root, id)
}

#[tauri::command]
fn get_instances(state: State<'_, AppState>) -> Vec<app::InstanceInfo> {
    app::list_instances(&state.root)
}

#[tauri::command]
async fn get_versions(snapshots: bool) -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || install::list_versions(snapshots)).await.map_err(|e| e.to_string())?
}

/// `loader`: vanilla, fabric, quilt, forge ya da neoforge. `version` boşsa en uygun yükleyici sürümü ağdan bulunur.
#[tauri::command]
async fn create_instance(state: State<'_, AppState>, name: String, game: String, loader: String, version: Option<String>) -> CmdResult<app::InstanceInfo> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::create_instance(&root, &name, &game, &loader, version)).await.map_err(|e| e.to_string())?
}

/// Adresle bağlanma (Aternos vb.): server'ı sorgulayıp sürümünü ve mod yükleyici ipucunu döndürür.
#[tauri::command]
async fn probe_server(handle: AppHandle, state: State<'_, AppState>, address: String) -> CmdResult<app::ProbeView> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::probe_server(&root, &address, &Events(handle))).await.map_err(|e| e.to_string())?
}

/// Server için instance oluşturur (oyunu açmaz); oyuncu önce mod ekleyebilir.
#[tauri::command]
async fn connect_prepare(handle: AppHandle, state: State<'_, AppState>, address: String, game: String, loader: String, version: Option<String>) -> CmdResult<app::InstanceInfo> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::connect_server(&root, &address, &game, &loader, version, false, &None, &Events(handle)).map(|r| r.0))
        .await
        .map_err(|e| e.to_string())?
}

/// Server için instance oluşturur ve oyunu açıp doğrudan bağlanır.
#[tauri::command]
fn connect_launch(handle: AppHandle, state: State<'_, AppState>, address: String, game: String, loader: String, version: Option<String>) -> CmdResult<()> {
    begin(&state)?;
    let root = state.root.clone();
    std::thread::spawn(move || {
        let r = app::connect_server(&root, &address, &game, &loader, version, true, &None, &Events(handle.clone()))
            .and_then(|(_, exit)| exit.ok_or_else(|| "oyun başlatılamadı".to_string()));
        finish(&handle, r);
    });
    Ok(())
}

#[tauri::command]
fn launch_instance(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    begin(&state)?;
    let root = state.root.clone();
    std::thread::spawn(move || {
        let r = app::launch_instance(&root, &id, &None, &Events(handle.clone()));
        finish(&handle, r);
    });
    Ok(())
}

#[tauri::command]
async fn join_prepare(handle: AppHandle, state: State<'_, AppState>, invite: String) -> CmdResult<app::JoinView> {
    let root = state.root.clone();
    let p = tauri::async_runtime::spawn_blocking(move || app::join_prepare(&root, &invite, &Events(handle))).await.map_err(|e| e.to_string())??;
    let view = app::join_view(&p);
    *state.prepared.lock().unwrap() = Some(p);
    Ok(view)
}

#[tauri::command]
fn join_run(handle: AppHandle, state: State<'_, AppState>, trust: bool) -> CmdResult<()> {
    let p = state.prepared.lock().unwrap().clone().ok_or_else(|| "Önce server'ı sorgula.".to_string())?;
    begin(&state)?;
    let root = state.root.clone();
    std::thread::spawn(move || {
        let r = app::join_run(&root, &p, trust, &None, &Events(handle.clone()));
        finish(&handle, r);
    });
    Ok(())
}

/// `kind`: mod, resourcepack ya da shader.
#[tauri::command]
async fn search_content(state: State<'_, AppState>, instance: String, kind: String, query: String, category: Option<String>) -> CmdResult<Vec<modrinth::Hit>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::search_content(&root, &instance, &kind, &query, category.as_deref())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_content(handle: AppHandle, state: State<'_, AppState>, instance: String, kind: String, project: String) -> CmdResult<Vec<String>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || app::install_content(&root, &instance, &kind, &project, &Events(handle))).await.map_err(|e| e.to_string())?
}


// ---------- server geçmişi ve durumu ----------

#[tauri::command]
fn list_servers(state: State<'_, AppState>) -> Vec<app::ServerEntry> {
    app::list_servers(&state.root)
}

#[tauri::command]
async fn server_status(host: String, port: u16) -> CmdResult<app::ServerStatus> {
    tauri::async_runtime::spawn_blocking(move || app::server_status(&host, port)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn set_discord(state: State<'_, AppState>, enabled: bool, app_id: String) -> CmdResult<app::Config> {
    app::set_discord(&state.root, enabled, &app_id)
}

// ---------- server kurma ve yönetim ----------

#[derive(Serialize)]
struct HostView {
    #[serde(flatten)]
    spec: serverhost::ServerSpec,
    state: String,
    tunnel: String,
}

#[tauri::command]
fn host_list(state: State<'_, AppState>) -> Vec<HostView> {
    serverhost::list_specs(&state.root)
        .into_iter()
        .map(|s| HostView { state: state.hosts.state_of(&s.id), tunnel: state.hosts.state_of(&format!("tunnel:{}", s.id)), spec: s })
        .collect()
}

#[tauri::command]
async fn host_create(handle: AppHandle, state: State<'_, AppState>, req: serverhost::CreateRequest) -> CmdResult<serverhost::ServerSpec> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || serverhost::create(&root, &req, &kazma_core::version::Env::current(), &Events(handle))).await.map_err(|e| e.to_string())?
}

/// Server'ı başlatır; açıksa router'da portu da açar (arka planda, sonuç konsola yazılır).
#[tauri::command]
async fn host_start(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let (root, hosts) = (state.root.clone(), state.hosts.clone());
    let sink: Arc<dyn serverhost::Sink> = Arc::new(ServerEvents(handle.clone()));
    let spec = serverhost::read_spec(&root, &id)?;
    let (r, s2) = (root.clone(), sink.clone());
    let id2 = id.clone();
    tauri::async_runtime::spawn_blocking(move || hosts.start(&r, &id2, &kazma_core::version::Env::current(), s2)).await.map_err(|e| e.to_string())??;
    if spec.upnp {
        std::thread::spawn(move || {
            sink.line(&id, "[Kazma] Router'da port açılıyor (UPnP)...");
            match upnp::open_port(spec.port, &format!("Kazma {}", spec.name)) {
                Ok((gw, o)) => {
                    handle.state::<AppState>().gateways.lock().unwrap().insert(id.clone(), (gw, spec.port));
                    if o.public {
                        let _ = serverhost::set_public_address(&root, &id, &o.address);
                        sink.line(&id, &format!("[Kazma] Port açıldı. Dış adres: {}", o.address));
                    } else {
                        sink.line(&id, &format!("[Kazma] Port router'da açıldı ama router'ın dış IP'si ({}) internete açık değil (operatör ortak IP kullanıyor olabilir). Dışarıdan bağlantı için tünel kullan.", o.external_ip));
                    }
                }
                Err(e) => sink.line(&id, &format!("[Kazma] Otomatik port açılamadı: {e} Tünel kullanabilir ya da portu router'dan elle açabilirsin.")),
            }
            let _ = handle.emit("server-changed", id);
        });
    }
    Ok(())
}

#[tauri::command]
async fn host_stop(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let hosts = state.hosts.clone();
    let sink: Arc<dyn serverhost::Sink> = Arc::new(ServerEvents(handle));
    tauri::async_runtime::spawn_blocking(move || hosts.stop(&id, std::time::Duration::from_secs(30), &sink)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn host_restart(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let (root, hosts) = (state.root.clone(), state.hosts.clone());
    let sink: Arc<dyn serverhost::Sink> = Arc::new(ServerEvents(handle));
    tauri::async_runtime::spawn_blocking(move || {
        hosts.stop(&id, std::time::Duration::from_secs(30), &sink)?;
        if !hosts.wait_stopped(&id, std::time::Duration::from_secs(40)) {
            return Err("Server kapanmadı, yeniden başlatılamadı.".to_string());
        }
        hosts.start(&root, &id, &kazma_core::version::Env::current(), sink)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn host_send(state: State<'_, AppState>, id: String, command: String) -> CmdResult<()> {
    state.hosts.send(&id, &command)
}

#[tauri::command]
fn host_log(state: State<'_, AppState>, id: String) -> Vec<String> {
    state.hosts.log(&id)
}

#[tauri::command]
fn host_props(state: State<'_, AppState>, id: String) -> CmdResult<std::collections::BTreeMap<String, String>> {
    Ok(serverhost::read_properties(&serverhost::data_dir(&state.root, &id)?))
}

#[tauri::command]
fn host_set_props(state: State<'_, AppState>, id: String, updates: std::collections::BTreeMap<String, String>) -> CmdResult<()> {
    serverhost::set_properties(&state.root, &id, &updates)
}

#[tauri::command]
fn host_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if state.hosts.state_of(&id) != "stopped" {
        return Err("Önce server'ı durdur.".into());
    }
    release_port(&state, &id);
    serverhost::delete_server(&state.root, &id)
}

#[tauri::command]
fn host_open_folder(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    app::open_in_file_manager(&serverhost::data_dir(&state.root, &id)?)
}

#[tauri::command]
async fn host_export(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || serverhost::export_package(&root, &id).map(|p| p.display().to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn host_open_exports(state: State<'_, AppState>) -> CmdResult<()> {
    let d = state.root.join("exports");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    app::open_in_file_manager(&d)
}

#[tauri::command]
async fn host_search(state: State<'_, AppState>, id: String, query: String) -> CmdResult<Vec<modrinth::Hit>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || serverhost::search_content(&root, &id, &query)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn host_install(handle: AppHandle, state: State<'_, AppState>, id: String, project: String) -> CmdResult<Vec<String>> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || serverhost::install_content(&root, &id, &project, &Events(handle))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn host_pack_code(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<kazma_engine::serverkit::PackBuild> {
    let root = state.root.clone();
    tauri::async_runtime::spawn_blocking(move || serverhost::pack_code(&root, &id, &Events(handle))).await.map_err(|e| e.to_string())?
}

/// Otomatik port yönlendirmeyi aç/kapat (açıksa bir sonraki başlatmada uygulanır; çalışıyorsa hemen açılır).
#[tauri::command]
async fn host_upnp(state: State<'_, AppState>, id: String, on: bool) -> CmdResult<serverhost::ServerSpec> {
    let root = state.root.clone();
    if !on {
        release_port(&state, &id);
    }
    tauri::async_runtime::spawn_blocking(move || serverhost::set_upnp(&root, &id, on)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn host_set_address(state: State<'_, AppState>, id: String, address: String) -> CmdResult<serverhost::ServerSpec> {
    serverhost::set_public_address(&state.root, &id, &address)
}

/// playit.gg tünel ajanını indirir (imza doğrulanır) ve çalıştırır. İlk kullanımda konsolda hesap bağlama adresi çıkar.
#[tauri::command]
async fn tunnel_start(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    serverhost::read_spec(&state.root, &id)?;
    let (root, hosts) = (state.root.clone(), state.hosts.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let exe = tunnel::ensure_agent(&root, &Events(handle.clone()))?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--stdout").current_dir(tunnel::agent_dir(&root));
        hosts.run_command(&format!("tunnel:{id}"), cmd, Arc::new(TunnelEvents(handle, id.clone())))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn tunnel_stop(handle: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let hosts = state.hosts.clone();
    let sink: Arc<dyn serverhost::Sink> = Arc::new(TunnelEvents(handle, id.clone()));
    tauri::async_runtime::spawn_blocking(move || hosts.stop(&format!("tunnel:{id}"), std::time::Duration::from_secs(1), &sink)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn tunnel_open_claim(url: String) -> CmdResult<()> {
    tunnel::open_claim(&url)
}

fn main() {
    let root = app::default_root();
    let _ = std::fs::create_dir_all(&root);
    tauri::Builder::default()
        .manage(AppState { root, prepared: Mutex::new(None), busy: Mutex::new(false), hosts: Arc::new(serverhost::Manager::new()), gateways: Mutex::new(HashMap::new()) })
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_player,
            get_instances,
            delete_instance,
            make_pack,
            open_instance_folder,
            list_skins,
            skin_data,
            add_skin,
            fetch_skin,
            skin_sources,
            add_skin_link,
            add_skin_ely,
            open_url,
            scan_sources,
            add_source,
            scan_accounts,
            set_identity,
            import_run,
            delete_skin,
            select_skin,
            get_versions,
            create_instance,
            probe_server,
            connect_prepare,
            connect_launch,
            launch_instance,
            join_prepare,
            join_run,
            search_content,
            install_content,
            list_servers,
            server_status,
            host_list,
            host_create,
            host_start,
            host_stop,
            host_restart,
            host_send,
            host_log,
            host_props,
            host_set_props,
            host_delete,
            host_open_folder,
            host_export,
            host_open_exports,
            host_search,
            host_install,
            host_pack_code,
            host_upnp,
            host_set_address,
            tunnel_start,
            tunnel_stop,
            tunnel_open_claim,
            set_discord
        ])
        .build(tauri::generate_context!())
        .expect("Kazma başlatılamadı")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let st = handle.state::<AppState>();
                let sink: Arc<dyn serverhost::Sink> = Arc::new(ServerEvents(handle.clone()));
                st.hosts.stop_all(&sink);
                release_all(&st);
            }
        });
}
