// Release derlemesinde Windows'ta konsol penceresi açılmasın.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri kabuğu: yalnızca ince sarmalayıcılar. Asıl mantık `kazma-engine`'de, orada testli.
//! Arayüze giden hatalar düz Türkçe metindir.

use kazma_engine::report::Reporter;
use kazma_engine::{app, install, modrinth, skins};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

struct AppState {
    root: PathBuf,
    prepared: Mutex<Option<app::Prepared>>,
    busy: Mutex<bool>,
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

fn main() {
    let root = app::default_root();
    let _ = std::fs::create_dir_all(&root);
    tauri::Builder::default()
        .manage(AppState { root, prepared: Mutex::new(None), busy: Mutex::new(false) })
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
            install_content
        ])
        .run(tauri::generate_context!())
        .expect("Kazma başlatılamadı");
}
