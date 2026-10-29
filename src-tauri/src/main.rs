// Release derlemesinde Windows'ta konsol penceresi açılmasın.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri kabuğu: yalnızca ince sarmalayıcılar. Asıl mantık `kazma-engine`'de, orada testli.
//! Arayüze giden hatalar düz Türkçe metindir.

use kazma_engine::report::Reporter;
use kazma_engine::{app, install, modrinth};
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

#[tauri::command]
fn get_instances(state: State<'_, AppState>) -> Vec<app::InstanceInfo> {
    app::list_instances(&state.root)
}

#[tauri::command]
async fn get_versions(snapshots: bool) -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || install::list_versions(snapshots)).await.map_err(|e| e.to_string())?
}

/// `fabric`: boş = vanilla değil, "latest" = en son kararlı Fabric, başka bir değer = o Fabric sürümü.
/// Vanilla için `None` gönderilir.
#[tauri::command]
async fn create_instance(state: State<'_, AppState>, name: String, game: String, fabric: Option<String>) -> CmdResult<app::InstanceInfo> {
    let root = state.root.clone();
    let loader = fabric.map(|v| if v == "latest" { None } else { Some(v) });
    tauri::async_runtime::spawn_blocking(move || app::create_instance(&root, &name, &game, loader)).await.map_err(|e| e.to_string())?
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

#[tauri::command]
async fn search_mods(state: State<'_, AppState>, instance: String, query: String, category: Option<String>) -> CmdResult<Vec<modrinth::Hit>> {
    let info = app::read_info(&state.root, &instance)?;
    if info.loader != "fabric" {
        return Err("Mod eklemek için Fabric'li bir instance seç.".into());
    }
    tauri::async_runtime::spawn_blocking(move || modrinth::search_live(&query, &info.game, "fabric", category.as_deref())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_mod(handle: AppHandle, state: State<'_, AppState>, instance: String, project: String) -> CmdResult<Vec<String>> {
    let info = app::read_info(&state.root, &instance)?;
    if info.loader != "fabric" {
        return Err("Mod eklemek için Fabric'li bir instance seç.".into());
    }
    if info.kind == "server" {
        return Err("Bir server'ın modları manifestle yönetilir; buraya elle mod eklenmez.".into());
    }
    let dir = state.root.join("instances").join(&info.id).join("mods");
    tauri::async_runtime::spawn_blocking(move || modrinth::install_live(&dir, &project, &info.game, "fabric", &Events(handle))).await.map_err(|e| e.to_string())?
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
            get_versions,
            create_instance,
            launch_instance,
            join_prepare,
            join_run,
            search_mods,
            install_mod
        ])
        .run(tauri::generate_context!())
        .expect("Kazma başlatılamadı");
}
