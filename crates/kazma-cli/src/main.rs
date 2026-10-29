//! Kazma komut satırı.
//!
//!   kazma-cli versions [--snapshots]
//!   kazma-cli launch <sürüm> <oyuncu> [--fabric [sürüm]] [--ram MB] [--join sunucu[:port]] [--java YOL] [--root KLASÖR]
//!   kazma-cli join <davet> <oyuncu> [--yes] [--ram MB] [--java YOL] [--root KLASÖR]
//!   kazma-cli make-key <dosya>
//!   kazma-cli make-manifest --name AD --game SÜRÜM [--fabric YÜKLEYİCİ] --mods KLASÖR --key DOSYA --out DOSYA [--base-url https://...]
//!   kazma-cli make-invite <sunucu[:port]> <manifest-adresi>

use kazma_core::invite::{self, Invite};
use kazma_core::manifest::Loader;
use kazma_core::sign;
use kazma_core::trust::fingerprint;
use kazma_engine::app;
use kazma_engine::install;
use kazma_engine::net::R;
use kazma_engine::report::StdoutReporter;
use kazma_engine::serverkit;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const USAGE: &str = "\
Kullanım:
  kazma-cli versions [--snapshots]
  kazma-cli launch <sürüm> <oyuncu> [--fabric [sürüm]] [--ram MB] [--join sunucu[:port]] [--java YOL] [--root KLASÖR]
  kazma-cli join <davet> <oyuncu> [--yes] [--ram MB] [--java YOL] [--root KLASÖR]
  kazma-cli make-key <dosya>
  kazma-cli make-manifest --name AD --game SÜRÜM [--fabric YÜKLEYİCİ] --mods KLASÖR --key DOSYA --out DOSYA [--base-url https://...]
  kazma-cli make-invite <sunucu[:port]> <manifest-adresi>";

// ---------- argüman ayrıştırma ----------

const VALUE_FLAGS: &[&str] = &["--ram", "--join", "--java", "--root", "--name", "--game", "--mods", "--key", "--out", "--base-url"];
const OPTIONAL_VALUE_FLAGS: &[&str] = &["--fabric"];
const BOOL_FLAGS: &[&str] = &["--snapshots", "--yes"];

#[derive(Debug, Default)]
struct Args {
    pos: Vec<String>,
    flags: HashMap<String, Option<String>>,
}

fn parse_args(a: &[String]) -> R<Args> {
    let mut out = Args::default();
    let mut i = 0;
    while i < a.len() {
        let t = &a[i];
        if !t.starts_with("--") {
            out.pos.push(t.clone());
        } else if BOOL_FLAGS.contains(&t.as_str()) {
            out.flags.insert(t.clone(), None);
        } else if OPTIONAL_VALUE_FLAGS.contains(&t.as_str()) {
            let v = a.get(i + 1).filter(|n| !n.starts_with("--")).cloned();
            if v.is_some() {
                i += 1;
            }
            out.flags.insert(t.clone(), v);
        } else if VALUE_FLAGS.contains(&t.as_str()) {
            let v = a.get(i + 1).filter(|n| !n.starts_with("--")).cloned().ok_or_else(|| format!("{t} için değer gerekli\n{USAGE}"))?;
            out.flags.insert(t.clone(), Some(v));
            i += 1;
        } else {
            return Err(format!("bilinmeyen seçenek: {t}\n{USAGE}"));
        }
        i += 1;
    }
    Ok(out)
}

impl Args {
    fn val(&self, k: &str) -> Option<String> {
        self.flags.get(k).cloned().flatten()
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
    fn need(&self, k: &str) -> R<String> {
        self.val(k).ok_or_else(|| format!("{k} gerekli\n{USAGE}"))
    }
}

#[derive(Debug, PartialEq)]
struct Common {
    ram: u32,
    java: Option<PathBuf>,
    root: PathBuf,
}

fn common(a: &Args) -> R<Common> {
    let ram = match a.val("--ram") {
        Some(r) => {
            let n: u32 = r.parse().map_err(|_| "--ram sayı olmalı (MB)".to_string())?;
            if !(1024..=65536).contains(&n) {
                return Err("--ram 1024 ile 65536 MB arasında olmalı".into());
            }
            n
        }
        None => 4096,
    };
    Ok(Common { ram, java: a.val("--java").map(PathBuf::from), root: a.val("--root").map(PathBuf::from).unwrap_or_else(app::default_root) })
}

#[derive(Debug, PartialEq)]
enum Cmd {
    Versions { snapshots: bool },
    Launch { game: String, player: String, fabric: Option<Option<String>>, join: Option<(String, u16)>, c: Common },
    Join { invite: String, player: String, yes: bool, c: Common },
    MakeKey { out: PathBuf },
    MakeManifest { name: String, game: String, fabric: Option<String>, mods: PathBuf, key: PathBuf, out: PathBuf, base_url: Option<String> },
    MakeInvite { server: String, manifest_url: String },
}

fn parse_cli(args: &[String]) -> R<Cmd> {
    let Some(cmd) = args.first() else { return Err(USAGE.into()) };
    let a = parse_args(&args[1..])?;
    let pos = |n: usize| -> R<&String> { a.pos.get(n).ok_or_else(|| USAGE.to_string()) };
    let exact = |n: usize| -> R<()> { if a.pos.len() == n { Ok(()) } else { Err(USAGE.to_string()) } };
    match cmd.as_str() {
        "versions" => {
            exact(0)?;
            Ok(Cmd::Versions { snapshots: a.has("--snapshots") })
        }
        "launch" => {
            exact(2)?;
            let join = match a.val("--join") {
                Some(j) => {
                    let i = invite::parse(&j).map_err(|e| e.to_string())?;
                    Some((i.host, i.port))
                }
                None => None,
            };
            Ok(Cmd::Launch { game: pos(0)?.clone(), player: pos(1)?.clone(), fabric: a.flags.get("--fabric").cloned(), join, c: common(&a)? })
        }
        "join" => {
            exact(2)?;
            Ok(Cmd::Join { invite: pos(0)?.clone(), player: pos(1)?.clone(), yes: a.has("--yes"), c: common(&a)? })
        }
        "make-key" => {
            exact(1)?;
            Ok(Cmd::MakeKey { out: PathBuf::from(pos(0)?) })
        }
        "make-manifest" => {
            exact(0)?;
            let fabric = match a.flags.get("--fabric") {
                None => None,
                Some(Some(v)) => Some(v.clone()),
                Some(None) => return Err("--fabric için yükleyici sürümü gerekli (örn. --fabric 0.16.9)".into()),
            };
            Ok(Cmd::MakeManifest {
                name: a.need("--name")?,
                game: a.need("--game")?,
                fabric,
                mods: PathBuf::from(a.need("--mods")?),
                key: PathBuf::from(a.need("--key")?),
                out: PathBuf::from(a.need("--out")?),
                base_url: a.val("--base-url"),
            })
        }
        "make-invite" => {
            exact(2)?;
            Ok(Cmd::MakeInvite { server: pos(0)?.clone(), manifest_url: pos(1)?.clone() })
        }
        _ => Err(USAGE.into()),
    }
}

// ---------- komutlar ----------


fn report_exit(e: kazma_engine::install::GameExit) -> i32 {
    if e.code != 0 {
        eprintln!("Günlük dosyası: {}", e.log.display());
    }
    e.code
}

fn prepare_root(c: &Common, player: &str) -> R<()> {
    fs::create_dir_all(&c.root).map_err(|e| format!("{}: {e}", c.root.display()))?;
    let mut cfg = app::load_config(&c.root);
    cfg.player = player.to_string();
    cfg.ram = c.ram;
    kazma_core::account::validate_player_name(player).map_err(|e| e.to_string())?;
    app::save_config(&c.root, &cfg)
}

fn run_launch(game: String, player: String, fabric: Option<Option<String>>, join: Option<(String, u16)>, c: Common) -> R<i32> {
    prepare_root(&c, &player)?;
    let info = match app::create_instance(&c.root, &format!("{game}{}", if fabric.is_some() { " Fabric" } else { "" }), &game, fabric.clone()) {
        Ok(i) => i,
        Err(e) if e.contains("zaten bir instance var") => {
            // aynı sürüm daha önce oluşturulmuş: kimliği yeniden hesapla
            let ver = match &fabric {
                Some(Some(v)) => Some(v.clone()),
                Some(None) => Some(install::latest_stable_fabric(&game)?),
                None => None,
            };
            let spec = kazma_core::instance::from_selection(&game, ver.as_deref()).map_err(|e| e.to_string())?;
            app::read_info(&c.root, &spec.id)?
        }
        Err(e) => return Err(e),
    };
    if join.is_some() {
        // sunucuya doğrudan bağlanma seçeneği: instance kaydını değiştirmeden tek seferlik
        let spec = app::spec_of(&info)?;
        let env = kazma_core::version::Env::current();
        let inst = install::ensure_installed(&c.root, &spec, &env, &StdoutReporter)?;
        let acct = kazma_core::launch::Account::offline(&player).map_err(|e| e.to_string())?;
        let opts = kazma_core::launch::LaunchOptions { max_mb: c.ram, join_server: join, ..Default::default() };
        return install::launch_game(&c.root, &spec, &inst, &env, &acct, &opts, &c.java, &StdoutReporter).map(report_exit);
    }
    app::launch_instance(&c.root, &info.id, &c.java, &StdoutReporter).map(report_exit)
}

fn confirm(question: &str, yes: bool) -> bool {
    if yes {
        println!("{question} e (--yes)");
        return true;
    }
    print!("{question} (e/H) ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).is_ok() && matches!(s.trim().to_lowercase().as_str(), "e" | "evet" | "y" | "yes")
}

fn run_join(inv_text: String, player: String, yes: bool, c: Common) -> R<i32> {
    prepare_root(&c, &player)?;
    let p = app::join_prepare(&c.root, &inv_text, &StdoutReporter)?;
    let v = app::join_view(&p);
    println!("Server: {} · Minecraft {} · {} {}", v.server_name, v.game, v.loader, v.loader_version);
    println!("Bu server için ayrı bir instance kullanılıyor: {}", v.instance_id);
    if v.is_new {
        println!("Bu server'ı ilk kez görüyorsun.\n  imza parmak izi: {}\n  Bu parmak izini server sahibinden başka bir kanaldan doğrula.", v.fingerprint);
        if !confirm("Bu server'a güveniyor musun?", yes) {
            return Err("Katılım iptal edildi.".into());
        }
    }
    if !v.noop {
        println!("Mod eşitleme planı:\n{}", kazma_engine::sync::summarize(&p.plan));
        println!("Modlar oyunda kod çalıştırır. Dosyalar yalnızca doğrulanmış kaynaklardan gelir ve SHA-512 ile kontrol edilir.");
        if !confirm("Devam edilsin mi?", yes) {
            return Err("Eşitleme iptal edildi.".into());
        }
    }
    app::join_run(&c.root, &p, true, &c.java, &StdoutReporter).map(report_exit)
}

fn run_make_manifest(name: String, game: String, fabric: Option<String>, mods: PathBuf, key: PathBuf, out: PathBuf, base_url: Option<String>) -> R<()> {
    let loader = match &fabric {
        Some(v) => Loader { kind: "fabric".into(), version: v.clone() },
        None => Loader { kind: "vanilla".into(), version: String::new() },
    };
    let seed = serverkit::read_seed(&key)?;
    let files = serverkit::scan_for_manifest(&mods)?;
    println!("{} mod bulundu, indirme adresleri Modrinth'ten aranıyor", files.len());
    let lookup = |h: &str| serverkit::modrinth_lookup(h);
    let mut m = serverkit::build_manifest(&name, &game, loader, &files, base_url.as_deref(), &lookup)?;
    let extra: Vec<String> = base_url.iter().filter_map(|b| url::Url::parse(b).ok().and_then(|u| u.host_str().map(String::from))).collect();
    let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();
    m.validate_with(&extra_refs).map_err(|e| e.to_string())?;
    sign::sign(&mut m, &seed);
    let json = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    fs::write(&out, json).map_err(|e| format!("{}: {e}", out.display()))?;
    let pk = sign::public_key_hex(&seed);
    println!("Yazıldı: {}\nİmza parmak izi: {}  (oyunculara bunu söyle)", out.display(), fingerprint(&pk));
    println!("Dosyayı https ile erişilebilir bir yere koy, sonra: kazma-cli make-invite <sunucu[:port]> <manifest-adresi>");
    Ok(())
}

fn run(cmd: Cmd) -> R<i32> {
    match cmd {
        Cmd::Versions { snapshots } => {
            for v in install::list_versions(snapshots)? {
                println!("{v}");
            }
            Ok(0)
        }
        Cmd::Launch { game, player, fabric, join, c } => run_launch(game, player, fabric, join, c),
        Cmd::Join { invite, player, yes, c } => run_join(invite, player, yes, c),
        Cmd::MakeKey { out } => {
            let pk = serverkit::make_key(&out)?;
            println!("Gizli anahtar yazıldı: {} (kimseyle paylaşma, yedekle)\nAçık anahtar parmak izi: {}", out.display(), fingerprint(&pk));
            Ok(0)
        }
        Cmd::MakeManifest { name, game, fabric, mods, key, out, base_url } => run_make_manifest(name, game, fabric, mods, key, out, base_url).map(|_| 0),
        Cmd::MakeInvite { server, manifest_url } => {
            let mut i: Invite = invite::parse(&server).map_err(|e| e.to_string())?;
            let u = url::Url::parse(&manifest_url).map_err(|_| "manifest adresi geçersiz".to_string())?;
            if u.scheme() != "https" {
                return Err("manifest adresi https olmalı".into());
            }
            i.manifest_url = Some(manifest_url);
            println!("{}", i.to_link());
            Ok(0)
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli(&args).and_then(run) {
        Ok(0) => {}
        Ok(code) => {
            eprintln!("Oyun {code} koduyla kapandı");
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("Hata: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn launch_variants() {
        match parse_cli(&a("launch 1.21.1 Mustafa --fabric --ram 6144 --join play.example.com:25570 --root C:\\k")).unwrap() {
            Cmd::Launch { game, player, fabric, join, c } => {
                assert_eq!((game.as_str(), player.as_str()), ("1.21.1", "Mustafa"));
                assert_eq!(fabric, Some(None), "sürümsüz --fabric = en son kararlı");
                assert_eq!(join, Some(("play.example.com".into(), 25570)));
                assert_eq!((c.ram, c.root), (6144, PathBuf::from("C:\\k")));
            }
            x => panic!("{x:?}"),
        }
        match parse_cli(&a("launch 1.20.1 X --fabric 0.15.11")).unwrap() {
            Cmd::Launch { fabric, .. } => assert_eq!(fabric, Some(Some("0.15.11".into()))),
            x => panic!("{x:?}"),
        }
        match parse_cli(&a("launch 1.20.1 X")).unwrap() {
            Cmd::Launch { fabric, .. } => assert_eq!(fabric, None, "Fabric yoksa vanilla"),
            x => panic!("{x:?}"),
        }
    }

    #[test]
    fn join_and_others() {
        match parse_cli(&a("join kazma://join/a.com?m=https%3A%2F%2Fa.com%2Fk.json Mustafa --yes")).unwrap() {
            Cmd::Join { invite, yes, .. } => {
                assert!(yes);
                assert!(invite.starts_with("kazma://join/a.com"));
            }
            x => panic!("{x:?}"),
        }
        assert_eq!(parse_cli(&a("versions --snapshots")).unwrap(), Cmd::Versions { snapshots: true });
        assert_eq!(parse_cli(&a("versions")).unwrap(), Cmd::Versions { snapshots: false });
        assert!(matches!(parse_cli(&a("make-invite a.com https://a.com/k.json")).unwrap(), Cmd::MakeInvite { .. }));
    }

    #[test]
    fn make_manifest_needs_everything() {
        let ok = a("make-manifest --name S --game 1.21.1 --fabric 0.16.9 --mods m --key k --out o");
        assert!(matches!(parse_cli(&ok).unwrap(), Cmd::MakeManifest { fabric: Some(_), .. }));
        assert!(parse_cli(&a("make-manifest --name S --game 1.21.1 --mods m --key k")).is_err());
        assert!(parse_cli(&a("make-manifest --name S --game 1.21.1 --fabric --mods m --key k --out o")).is_err(), "Fabric için sürüm şart");
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["", "play 1.21.1 X", "launch 1.21.1", "launch 1.21.1 X --ram abc", "launch 1.21.1 X --ram 10", "launch 1.21.1 X --join -bad",
            "launch 1.21.1 X --nope", "launch 1.21.1 X --ram", "join onlyone", "versions extra"] {
            assert!(parse_cli(&a(bad)).is_err(), "{bad:?} reddedilmeli");
        }
    }
}
