//! Başlatma komutunu üretir: `java <jvm argümanları> <ana sınıf> <oyun argümanları>`.
//! Komut bir argüman listesidir; kabuktan geçmez, bu yüzden boşluklu yollar ve oyuncu adları güvenlidir.

use crate::plan::Plan;
use crate::version::{expand_args, Env, Os, Resolved};
use crate::{account, Result};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Account {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
}

impl Account {
    /// Alternatif (çevrimdışı) hesap. Server'ın `online-mode=false` olması gerekir.
    pub fn offline(name: &str) -> Result<Self> {
        Ok(Account { name: name.into(), uuid: account::offline_uuid(name)?, access_token: "0".into(), user_type: "msa".into() })
    }
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub min_mb: u32,
    pub max_mb: u32,
    pub launcher_name: String,
    pub launcher_version: String,
    pub resolution: Option<(u32, u32)>,
    /// Oyun açılır açılmaz bağlanılacak server.
    pub join_server: Option<(String, u16)>,
    pub extra_jvm: Vec<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        LaunchOptions {
            min_mb: 512,
            max_mb: 4096,
            launcher_name: "kazma".into(),
            launcher_version: env!("CARGO_PKG_VERSION").into(),
            resolution: None,
            join_server: None,
            extra_jvm: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dirs {
    pub root: PathBuf,
    pub game_dir: PathBuf,
    pub natives_dir: PathBuf,
}

fn subst(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

/// `java` çalıştırılabilirine verilecek argümanlar (program yolu hariç).
pub fn build_args(r: &Resolved, plan: &Plan, env: &Env, dirs: &Dirs, acct: &Account, o: &LaunchOptions) -> Result<Vec<String>> {
    let mut env = env.clone();
    env.features.insert("has_custom_resolution".into(), o.resolution.is_some());
    env.features.insert("is_quick_play_multiplayer".into(), o.join_server.is_some());

    let sep = if env.os == Os::Windows { ";" } else { ":" };
    let classpath = plan
        .classpath
        .iter()
        .map(|rel| dirs.root.join(rel).to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(sep);

    let (w, h) = o.resolution.unwrap_or((854, 480));
    let (qhost, qport) = o.join_server.clone().unwrap_or_default();
    let quick = if o.join_server.is_some() { format!("{qhost}:{qport}") } else { String::new() };
    let mut vars: HashMap<&str, String> = HashMap::new();
    for (k, v) in [
        ("auth_player_name", acct.name.clone()),
        ("auth_uuid", acct.uuid.clone()),
        ("auth_access_token", acct.access_token.clone()),
        ("auth_session", format!("token:{}:{}", acct.access_token, acct.uuid)),
        ("user_type", acct.user_type.clone()),
        ("user_properties", "{}".into()),
        ("clientid", String::new()),
        ("auth_xuid", String::new()),
        ("version_name", r.id.clone()),
        ("version_type", r.kind.clone()),
        ("game_directory", dirs.game_dir.to_string_lossy().into_owned()),
        ("assets_root", dirs.root.join("assets").to_string_lossy().into_owned()),
        ("game_assets", dirs.root.join("assets").join("virtual").join("legacy").to_string_lossy().into_owned()),
        ("assets_index_name", r.assets.clone()),
        ("natives_directory", dirs.natives_dir.to_string_lossy().into_owned()),
        ("library_directory", dirs.root.join("libraries").to_string_lossy().into_owned()),
        ("classpath_separator", sep.into()),
        ("launcher_name", o.launcher_name.clone()),
        ("launcher_version", o.launcher_version.clone()),
        ("classpath", classpath),
        ("resolution_width", w.to_string()),
        ("resolution_height", h.to_string()),
        ("quickPlayMultiplayer", quick),
        ("quickPlaySingleplayer", String::new()),
        ("quickPlayRealms", String::new()),
        ("quickPlayPath", String::new()),
    ] {
        vars.insert(k, v);
    }

    let mut args = vec![format!("-Xms{}M", o.min_mb), format!("-Xmx{}M", o.max_mb)];
    let jvm: Vec<String> = if r.jvm_args.is_empty() {
        ["-Djava.library.path=${natives_directory}", "-Dminecraft.launcher.brand=${launcher_name}", "-Dminecraft.launcher.version=${launcher_version}", "-cp", "${classpath}"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        expand_args(&r.jvm_args, &env)
    };
    args.extend(jvm.iter().map(|a| subst(a, &vars)));
    args.extend(o.extra_jvm.iter().cloned());
    args.push(r.main_class.clone());

    let game: Vec<String> = if !r.game_args.is_empty() {
        expand_args(&r.game_args, &env)
    } else {
        r.legacy_args.as_deref().unwrap_or("").split_whitespace().map(String::from).collect()
    };
    let mut game: Vec<String> = game.iter().map(|a| subst(a, &vars)).collect();
    if let Some((host, port)) = &o.join_server {
        if !game.iter().any(|a| a == "--quickPlayMultiplayer") {
            game.extend(["--server".into(), host.clone(), "--port".into(), port.to_string()]);
        }
    }
    if o.resolution.is_some() && !game.iter().any(|a| a == "--width") {
        game.extend(["--width".into(), w.to_string(), "--height".into(), h.to_string()]);
    }
    args.extend(game);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::build_plan;
    use crate::version::{fixtures::*, parse, resolve};

    fn dirs() -> Dirs {
        Dirs { root: "/k".into(), game_dir: "/k/instances/a".into(), natives_dir: "/k/natives/1.21.1".into() }
    }
    fn val<'a>(a: &'a [String], key: &str) -> &'a str {
        let i = a.iter().position(|x| x == key).unwrap_or_else(|| panic!("{key} yok: {a:?}"));
        &a[i + 1]
    }

    #[test]
    fn vanilla_offline_command() {
        let r = resolve(parse(VANILLA).unwrap(), None).unwrap();
        let env = Env::windows_x64();
        let plan = build_plan(&r, &env).unwrap();
        let acct = Account::offline("Efe_Kral").unwrap();
        let a = build_args(&r, &plan, &env, &dirs(), &acct, &LaunchOptions::default()).unwrap();

        assert_eq!(a[0], "-Xms512M");
        assert_eq!(a[1], "-Xmx4096M");
        let main = a.iter().position(|x| x == "net.minecraft.client.main.Main").unwrap();
        let cp = a.iter().position(|x| x == "-cp").unwrap();
        assert!(cp < main, "JVM argümanları ana sınıftan önce");
        assert_eq!(val(&a, "--username"), "Efe_Kral");
        assert_eq!(val(&a, "--uuid"), "f0180f74-7e22-35a1-8953-3717dee31846");
        assert_eq!(val(&a, "--accessToken"), "0");
        assert_eq!(val(&a, "--version"), "1.21.1");
        assert_eq!(val(&a, "--assetIndex"), "17");
        assert!(val(&a, "--assetsDir").ends_with("assets"));
        assert!(val(&a, "-cp").contains(';'), "Windows sınıf yolu ';' ile ayrılır");
        assert!(val(&a, "-cp").ends_with("1.21.1.jar"));
        assert!(a.contains(&"-Dos.name=Windows 10".to_string()));
        assert!(a.iter().any(|x| x.starts_with("-Djava.library.path=") && x.ends_with("1.21.1")));
        assert!(!a.iter().any(|x| x.contains("${")), "çözülmemiş yer tutucu kalmamalı: {a:?}");
        assert!(!a.contains(&"--demo".to_string()));
        assert!(!a.contains(&"--width".to_string()));
    }

    #[test]
    fn join_server_and_resolution() {
        let r = resolve(parse(VANILLA).unwrap(), None).unwrap();
        let env = Env::windows_x64();
        let plan = build_plan(&r, &env).unwrap();
        let o = LaunchOptions { join_server: Some(("play.example.com".into(), 25570)), resolution: Some((1280, 720)), ..Default::default() };
        let a = build_args(&r, &plan, &env, &dirs(), &Account::offline("Mustafa").unwrap(), &o).unwrap();
        assert_eq!(val(&a, "--quickPlayMultiplayer"), "play.example.com:25570");
        assert_eq!(val(&a, "--width"), "1280");
        assert_eq!(val(&a, "--height"), "720");
        assert!(!a.contains(&"--server".to_string()), "quickPlay varsa --server eklenmez");
    }

    #[test]
    fn fabric_uses_knot_and_parent_jar() {
        let r = resolve(parse(FABRIC).unwrap(), Some(parse(VANILLA).unwrap())).unwrap();
        let env = Env::windows_x64();
        let plan = build_plan(&r, &env).unwrap();
        let a = build_args(&r, &plan, &env, &dirs(), &Account::offline("Mustafa").unwrap(), &LaunchOptions::default()).unwrap();
        assert!(a.contains(&"net.fabricmc.loader.impl.launch.knot.KnotClient".to_string()));
        assert!(!a.contains(&"net.minecraft.client.main.Main".to_string()));
        assert_eq!(val(&a, "--version"), "fabric-loader-0.16.9-1.21.1");
        assert!(val(&a, "-cp").contains("fabric-loader-0.16.9.jar"));
        assert!(val(&a, "-cp").ends_with("versions/1.21.1/1.21.1.jar"));
        assert!(a.contains(&"-DFabricMcEmu= net.minecraft.client.main.Main ".to_string()), "boşluklu argüman tek parça kalır");
        assert!(!a.iter().any(|x| x.contains("${")));
    }

    #[test]
    fn legacy_version_uses_defaults_and_server_flags() {
        let r = resolve(parse(LEGACY).unwrap(), None).unwrap();
        let env = Env::windows_x64();
        let plan = build_plan(&r, &env).unwrap();
        let o = LaunchOptions { join_server: Some(("play.example.com".into(), 25565)), ..Default::default() };
        let a = build_args(&r, &plan, &env, &dirs(), &Account::offline("Mustafa").unwrap(), &o).unwrap();
        assert!(a.iter().any(|x| x.starts_with("-Djava.library.path=")), "JVM argümanı yoksa varsayılanlar eklenir");
        assert_eq!(val(&a, "--username"), "Mustafa");
        assert_eq!(val(&a, "--server"), "play.example.com");
        assert_eq!(val(&a, "--port"), "25565");
        assert!(!a.iter().any(|x| x.contains("${")));
    }

    #[test]
    fn linux_separator() {
        let r = resolve(parse(VANILLA).unwrap(), None).unwrap();
        let env = Env { os: Os::Linux, ..Env::windows_x64() };
        let plan = build_plan(&r, &env).unwrap();
        let a = build_args(&r, &plan, &env, &dirs(), &Account::offline("Mustafa").unwrap(), &LaunchOptions::default()).unwrap();
        assert!(val(&a, "-cp").contains(':') && !val(&a, "-cp").contains(';'));
    }

    #[test]
    fn bad_offline_name_is_rejected() {
        assert!(Account::offline("a b").is_err());
    }
}
