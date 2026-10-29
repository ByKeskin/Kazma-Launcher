// Release derlemesinde Windows'ta konsol penceresi açılmasın.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kazma_core::{account, invite, manifest, mojang, sign};
use serde::Serialize;

/// Arayüze dönen hatalar düz metindir; Türkçe mesajlar kazma-core'dan gelir.
type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
struct OfflineAccount {
    name: String,
    uuid: String,
}

#[tauri::command]
fn offline_account(name: String) -> CmdResult<OfflineAccount> {
    let uuid = account::offline_uuid(&name).map_err(|e| e.to_string())?;
    Ok(OfflineAccount { name, uuid })
}

#[derive(Serialize)]
struct ParsedInvite {
    host: String,
    port: u16,
}

#[tauri::command]
fn parse_invite(input: String) -> CmdResult<ParsedInvite> {
    let i = invite::parse(&input).map_err(|e| e.to_string())?;
    Ok(ParsedInvite { host: i.host, port: i.port })
}

#[derive(Serialize)]
struct VersionInfo {
    id: String,
    url: String,
}

/// Arayüz Mojang manifestini indirir (CSP/HTTP kısıtı yüzünden Rust tarafında indirilecek, bkz. README),
/// metni buraya verir; burada ayrıştırılıp yalnızca sürüm listesi döner.
#[tauri::command]
fn list_release_versions(manifest_json: String) -> CmdResult<Vec<VersionInfo>> {
    let m = mojang::parse_manifest(&manifest_json).map_err(|e| e.to_string())?;
    Ok(m.releases().into_iter().map(|v| VersionInfo { id: v.id.clone(), url: v.url.clone() }).collect())
}

#[derive(Serialize)]
struct PlanView {
    ready: Vec<String>,
    download: Vec<String>,
    update: Vec<String>,
    disable: Vec<String>,
    keep: Vec<String>,
    download_bytes: u64,
    signer: String,
}

/// Server manifestini doğrular (şema, dosya adı, adres, imza) ve yerel modlarla karşılaştırır.
/// `pinned_signer`: daha önce bu server için saklanan açık anahtar (varsa).
#[tauri::command]
fn plan_sync(
    manifest_json: String,
    server_host: String,
    pinned_signer: Option<String>,
    local: Vec<LocalModArg>,
) -> CmdResult<PlanView> {
    let m = manifest::parse(&manifest_json).map_err(|e| e.to_string())?;
    m.validate(Some(&server_host)).map_err(|e| e.to_string())?;
    let signer = sign::verify(&m, pinned_signer.as_deref()).map_err(|e| e.to_string())?;
    let local: Vec<manifest::LocalMod> = local
        .into_iter()
        .map(|l| manifest::LocalMod { id: l.id, file: l.file, sha512: l.sha512, client_only: l.client_only })
        .collect();
    let p = manifest::plan(&m, &local);
    Ok(PlanView {
        download_bytes: p.download_bytes(),
        ready: p.ready,
        download: p.download.into_iter().map(|e| e.id).collect(),
        update: p.update.into_iter().map(|(_, e)| e.id).collect(),
        disable: p.disable.into_iter().map(|l| l.id).collect(),
        keep: p.keep.into_iter().map(|l| l.id).collect(),
        signer,
    })
}

#[derive(serde::Deserialize)]
struct LocalModArg {
    id: String,
    file: String,
    sha512: String,
    client_only: bool,
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![offline_account, parse_invite, list_release_versions, plan_sync])
        .run(tauri::generate_context!())
        .expect("Kazma başlatılamadı");
}
