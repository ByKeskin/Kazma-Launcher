//! Mojang sürüm JSON'u (`1.21.1.json`) ve mod yükleyici profilleri (Fabric `profile/json`).
//! Ağ yok; yalnızca ayrıştırma, kural değerlendirme ve `inheritsFrom` birleştirme.

use crate::{Error, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Osx,
}

impl Os {
    pub fn mojang_name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Linux => "linux",
            Os::Osx => "osx",
        }
    }
}

/// Kuralların değerlendirildiği ortam.
#[derive(Debug, Clone)]
pub struct Env {
    pub os: Os,
    /// "x64", "x86" ya da "arm64". Mojang kurallarında yalnızca "x86" ayrıca anılır.
    pub arch: String,
    pub os_version: String,
    pub features: HashMap<String, bool>,
}

impl Env {
    pub fn windows_x64() -> Self {
        Env { os: Os::Windows, arch: "x64".into(), os_version: "10.0".into(), features: HashMap::new() }
    }
    pub fn current() -> Self {
        let os = if cfg!(windows) { Os::Windows } else if cfg!(target_os = "macos") { Os::Osx } else { Os::Linux };
        let arch = if cfg!(target_arch = "x86") { "x86" } else if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        Env { os, arch: arch.into(), os_version: "10.0".into(), features: HashMap::new() }
    }
    pub fn with_feature(mut self, k: &str, v: bool) -> Self {
        self.features.insert(k.into(), v);
        self
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct OsRule {
    pub name: Option<String>,
    pub version: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

fn rule_applies(r: &Rule, env: &Env) -> bool {
    if let Some(os) = &r.os {
        if os.name.as_deref().map_or(false, |n| n != env.os.mojang_name()) {
            return false;
        }
        if os.arch.as_deref().map_or(false, |a| a != env.arch) {
            return false;
        }
        if let Some(v) = &os.version {
            // Mojang düzenli ifade kullanır ("^10\\."); yalnızca bu önek biçimini destekliyoruz.
            let prefix = v.trim_start_matches('^').replace("\\.", ".");
            if !env.os_version.starts_with(&prefix) {
                return false;
            }
        }
    }
    if let Some(f) = &r.features {
        for (k, want) in f {
            if env.features.get(k).copied().unwrap_or(false) != *want {
                return false;
            }
        }
    }
    true
}

/// Kural yoksa izinli. Varsa varsayılan "yasak"; eşleşen son kuralın eylemi geçerlidir.
pub fn rules_allow(rules: &[Rule], env: &Env) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for r in rules {
        if rule_applies(r, env) {
            allowed = r.action == "allow";
        }
    }
    allowed
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum Arg {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: ArgValue },
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Arg>,
    #[serde(default)]
    pub jvm: Vec<Arg>,
}

pub fn expand_args(args: &[Arg], env: &Env) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Plain(s) => out.push(s.clone()),
            Arg::Ruled { rules, value } if rules_allow(rules, env) => match value {
                ArgValue::One(s) => out.push(s.clone()),
                ArgValue::Many(v) => out.extend(v.iter().cloned()),
            },
            Arg::Ruled { .. } => {}
        }
    }
    out
}

#[derive(Debug, Deserialize, Clone)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct LibDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: HashMap<String, Artifact>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibDownloads>,
    /// Fabric tarzı: yalnızca Maven kök adresi.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Eski biçim: işletim sistemi -> sınıflandırıcı adı.
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<Extract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArtifact {
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

/// `group:artifact:version[:classifier][@ext]` -> `group/yolu/artifact/version/artifact-version[-classifier].ext`
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = name.split_once('@').unwrap_or((name, "jar"));
    let p: Vec<&str> = coords.split(':').collect();
    if !(3..=4).contains(&p.len()) || p.iter().any(|s| s.is_empty()) || ext.is_empty() {
        return None;
    }
    let group = p[0].replace('.', "/");
    let (a, v) = (p[1], p[2]);
    let cls = p.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!("{group}/{a}/{v}/{a}-{v}{cls}.{ext}"))
}

/// Sürüm sayısından bağımsız kimlik: `group:artifact[:classifier]`.
fn lib_key(name: &str) -> String {
    let coords = name.split('@').next().unwrap_or(name);
    let p: Vec<&str> = coords.split(':').collect();
    match (p.first(), p.get(1), p.get(3)) {
        (Some(g), Some(a), Some(c)) => format!("{g}:{a}:{c}"),
        (Some(g), Some(a), None) => format!("{g}:{a}"),
        _ => name.to_string(),
    }
}

impl Library {
    pub fn allowed(&self, env: &Env) -> bool {
        rules_allow(&self.rules, env)
    }

    /// Sınıf yoluna giren ana jar.
    pub fn artifact(&self) -> Option<ResolvedArtifact> {
        if let Some(a) = self.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            let path = a.path.clone().or_else(|| maven_path(&self.name))?;
            return Some(ResolvedArtifact { path, url: a.url.clone(), sha1: a.sha1.clone(), size: a.size });
        }
        if self.downloads.is_none() {
            if let Some(base) = &self.url {
                let path = maven_path(&self.name)?;
                let url = format!("{}{}", base.trim_end_matches('/').to_string() + "/", path);
                return Some(ResolvedArtifact { path, url, sha1: self.sha1.clone(), size: self.size });
            }
        }
        None
    }

    /// Eski biçim yerel kütüphane (`natives` haritası): sınıf yoluna girmez, çıkarılır.
    pub fn legacy_native(&self, env: &Env) -> Option<ResolvedArtifact> {
        let cls = self.natives.as_ref()?.get(env.os.mojang_name())?;
        let cls = cls.replace("${arch}", if env.arch == "x86" { "32" } else { "64" });
        let a = self.downloads.as_ref()?.classifiers.get(&cls)?;
        let path = a.path.clone()?;
        Some(ResolvedArtifact { path, url: a.url.clone(), sha1: a.sha1.clone(), size: a.size })
    }

    /// Yeni biçim (1.19+): `...:natives-windows` sınıflandırıcılı normal kütüphane.
    pub fn is_modern_native(&self) -> bool {
        self.name.split('@').next().unwrap_or("").split(':').nth(3).map_or(false, |c| c.starts_with("natives-"))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
    #[serde(default)]
    pub server: Option<Artifact>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JavaVersion {
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VersionJson {
    pub id: String,
    #[serde(default, rename = "inheritsFrom")]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "mainClass")]
    pub main_class: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default, rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, rename = "assetIndex")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default, rename = "javaVersion")]
    pub java_version: Option<JavaVersion>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

pub fn parse(json: &str) -> Result<VersionJson> {
    serde_json::from_str(json).map_err(|e| Error::BadManifest(e.to_string()))
}

/// Birleştirilmiş, başlatmaya hazır sürüm.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// `${version_name}`; Fabric'te `fabric-loader-0.16.9-1.21.1`.
    pub id: String,
    /// İstemci jar'ının ait olduğu sürüm (`inheritsFrom` varsa üst sürüm).
    pub jar_id: String,
    pub main_class: String,
    pub libraries: Vec<Library>,
    pub game_args: Vec<Arg>,
    pub jvm_args: Vec<Arg>,
    pub legacy_args: Option<String>,
    pub asset_index: AssetIndexRef,
    pub assets: String,
    pub client: Option<Artifact>,
    pub java_major: u32,
    pub kind: String,
}

pub fn resolve(child: VersionJson, parent: Option<VersionJson>) -> Result<Resolved> {
    if child.inherits_from.is_some() && parent.is_none() {
        return Err(Error::BadManifest(format!("{} için üst sürüm dosyası gerekli", child.id)));
    }
    let mut libraries = child.libraries.clone();
    if let Some(p) = &parent {
        let mine: HashSet<String> = child.libraries.iter().map(|l| lib_key(&l.name)).collect();
        libraries.extend(p.libraries.iter().filter(|l| !mine.contains(&lib_key(&l.name))).cloned());
    }
    let cat = |a: Option<&Arguments>, b: Option<&Arguments>, jvm: bool| -> Vec<Arg> {
        let pick = |x: Option<&Arguments>| x.map(|x| if jvm { x.jvm.clone() } else { x.game.clone() }).unwrap_or_default();
        let mut v = pick(a);
        v.extend(pick(b));
        v
    };
    let (pa, ca) = (parent.as_ref().and_then(|p| p.arguments.as_ref()), child.arguments.as_ref());
    let game_args = cat(pa, ca, false);
    let jvm_args = cat(pa, ca, true);

    let pick = |f: fn(&VersionJson) -> Option<&String>| f(&child).or_else(|| parent.as_ref().and_then(|p| f(p))).cloned();
    let main_class = pick(|v| v.main_class.as_ref()).ok_or_else(|| Error::BadManifest("mainClass yok".into()))?;
    let asset_index = child
        .asset_index
        .clone()
        .or_else(|| parent.as_ref().and_then(|p| p.asset_index.clone()))
        .ok_or_else(|| Error::BadManifest("assetIndex yok".into()))?;
    let assets = pick(|v| v.assets.as_ref()).unwrap_or_else(|| asset_index.id.clone());
    let client = child
        .downloads
        .as_ref()
        .and_then(|d| d.client.clone())
        .or_else(|| parent.as_ref().and_then(|p| p.downloads.as_ref()).and_then(|d| d.client.clone()));
    let java_major = child
        .java_version
        .as_ref()
        .or_else(|| parent.as_ref().and_then(|p| p.java_version.as_ref()))
        .map(|j| j.major_version)
        .unwrap_or(8);
    let legacy_args = child.minecraft_arguments.clone().or_else(|| parent.as_ref().and_then(|p| p.minecraft_arguments.clone()));
    let kind = pick(|v| v.kind.as_ref()).unwrap_or_else(|| "release".into());
    let jar_id = parent.as_ref().map(|p| p.id.clone()).unwrap_or_else(|| child.id.clone());

    Ok(Resolved { id: child.id, jar_id, main_class, libraries, game_args, jvm_args, legacy_args, asset_index, assets, client, java_major, kind })
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// 1.21.1 biçiminde, kısaltılmış ama yapısı gerçek olan sürüm dosyası.
    pub const VANILLA: &str = r#"{
      "id": "1.21.1", "type": "release", "mainClass": "net.minecraft.client.main.Main",
      "assets": "17",
      "assetIndex": {"id":"17","sha1":"aa","size":10,"url":"https://launchermeta.mojang.com/v1/packages/aa/17.json"},
      "downloads": {"client": {"sha1":"cc","size":99,"url":"https://piston-data.mojang.com/v1/objects/cc/client.jar"}},
      "javaVersion": {"component":"java-runtime-delta","majorVersion":21},
      "arguments": {
        "game": [
          "--username","${auth_player_name}","--version","${version_name}","--gameDir","${game_directory}",
          "--assetsDir","${assets_root}","--assetIndex","${assets_index_name}","--uuid","${auth_uuid}",
          "--accessToken","${auth_access_token}","--clientId","${clientid}","--xuid","${auth_xuid}",
          "--userType","${user_type}","--versionType","${version_type}",
          {"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"},
          {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}","--height","${resolution_height}"]},
          {"rules":[{"action":"allow","features":{"is_quick_play_multiplayer":true}}],"value":["--quickPlayMultiplayer","${quickPlayMultiplayer}"]}
        ],
        "jvm": [
          {"rules":[{"action":"allow","os":{"name":"osx"}}],"value":["-XstartOnFirstThread"]},
          {"rules":[{"action":"allow","os":{"name":"windows"}}],"value":"-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"},
          {"rules":[{"action":"allow","os":{"name":"windows","version":"^10\\."}}],"value":["-Dos.name=Windows 10","-Dos.version=10.0"]},
          {"rules":[{"action":"allow","os":{"arch":"x86"}}],"value":"-Xss1M"},
          "-Djava.library.path=${natives_directory}",
          "-Dminecraft.launcher.brand=${launcher_name}",
          "-Dminecraft.launcher.version=${launcher_version}",
          "-cp","${classpath}"
        ]
      },
      "libraries": [
        {"name":"com.mojang:logging:1.2.7","downloads":{"artifact":{"path":"com/mojang/logging/1.2.7/logging-1.2.7.jar","sha1":"11","size":1,"url":"https://libraries.minecraft.net/com/mojang/logging/1.2.7/logging-1.2.7.jar"}}},
        {"name":"ca.weblite:java-objc-bridge:1.1","rules":[{"action":"allow","os":{"name":"osx"}}],"downloads":{"artifact":{"path":"ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar","sha1":"22","size":1,"url":"https://libraries.minecraft.net/ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar"}}},
        {"name":"org.lwjgl:lwjgl:3.3.3","downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar","sha1":"33","size":1,"url":"https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"}}},
        {"name":"org.lwjgl:lwjgl:3.3.3:natives-windows","rules":[{"action":"allow","os":{"name":"windows"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar","sha1":"44","size":1,"url":"https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"}}},
        {"name":"org.lwjgl:lwjgl:3.3.3:natives-windows-x86","rules":[{"action":"allow","os":{"name":"windows","arch":"x86"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows-x86.jar","sha1":"55","size":1,"url":"https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows-x86.jar"}}},
        {"name":"org.lwjgl:lwjgl:3.3.3:natives-linux","rules":[{"action":"allow","os":{"name":"linux"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar","sha1":"66","size":1,"url":"https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar"}}}
      ]
    }"#;

    /// Fabric meta `profile/json` biçimi: kütüphanelerde yalnızca ad ve Maven adresi var.
    pub const FABRIC: &str = r#"{
      "id": "fabric-loader-0.16.9-1.21.1", "inheritsFrom": "1.21.1", "type": "release",
      "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
      "arguments": {"game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]},
      "libraries": [
        {"name":"org.ow2.asm:asm:9.7","url":"https://maven.fabricmc.net/","md5":"x","sha1":"aaaa","size":123},
        {"name":"net.fabricmc:intermediary:1.21.1","url":"https://maven.fabricmc.net/"},
        {"name":"net.fabricmc:fabric-loader:0.16.9","url":"https://maven.fabricmc.net/"},
        {"name":"com.mojang:logging:1.2.7","url":"https://libraries.minecraft.net/"}
      ]
    }"#;

    /// 1.12 biçimi: `minecraftArguments` metni ve eski `natives` haritası.
    pub const LEGACY: &str = r#"{
      "id": "1.12.2", "type": "release", "mainClass": "net.minecraft.client.main.Main",
      "minecraftArguments": "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} --assetsDir ${assets_root} --assetIndex ${assets_index_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userType ${user_type} --versionType ${version_type}",
      "assets": "1.12",
      "assetIndex": {"id":"1.12","url":"https://launchermeta.mojang.com/v1/packages/bb/1.12.json"},
      "downloads": {"client": {"sha1":"dd","size":9,"url":"https://launcher.mojang.com/v1/objects/dd/client.jar"}},
      "libraries": [
        {"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4-nightly-20150209",
         "natives":{"linux":"natives-linux","windows":"natives-windows","osx":"natives-osx"},
         "extract":{"exclude":["META-INF/"]},
         "downloads":{"artifact":{"path":"org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209.jar","sha1":"1","size":1,"url":"https://libraries.minecraft.net/a.jar"},
           "classifiers":{"natives-windows":{"path":"org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar","sha1":"2","size":1,"url":"https://libraries.minecraft.net/b.jar"}}}}
      ]
    }"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(maven_path("org.ow2.asm:asm:9.7").unwrap(), "org/ow2/asm/asm/9.7/asm-9.7.jar");
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows").unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(maven_path("a.b:c:1@zip").unwrap(), "a/b/c/1/c-1.zip");
        assert!(maven_path("broken").is_none());
        assert!(maven_path("a::1").is_none());
    }

    #[test]
    fn rules_follow_mojang_semantics() {
        let win = Env::windows_x64();
        let osx_only: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","os":{"name":"osx"}}]"#).unwrap();
        assert!(!rules_allow(&osx_only, &win));
        assert!(rules_allow(&[], &win), "kural yoksa izinli");
        let deny_win: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"windows"}}]"#).unwrap();
        assert!(!rules_allow(&deny_win, &win));
        let x86: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","os":{"arch":"x86"}}]"#).unwrap();
        assert!(!rules_allow(&x86, &win), "x64'te x86 kuralı uygulanmaz");
        let feat: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#).unwrap();
        assert!(!rules_allow(&feat, &win));
        assert!(rules_allow(&feat, &Env::windows_x64().with_feature("is_demo_user", true)));
    }

    #[test]
    fn vanilla_libraries_for_windows() {
        let v = parse(VANILLA).unwrap();
        let win = Env::windows_x64();
        let names: Vec<_> = v.libraries.iter().filter(|l| l.allowed(&win)).map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["com.mojang:logging:1.2.7", "org.lwjgl:lwjgl:3.3.3", "org.lwjgl:lwjgl:3.3.3:natives-windows"]);
        assert!(v.libraries[3].is_modern_native());
        assert!(!v.libraries[2].is_modern_native());
    }

    #[test]
    fn jvm_args_expand_per_os() {
        let v = parse(VANILLA).unwrap();
        let jvm = expand_args(&v.arguments.as_ref().unwrap().jvm, &Env::windows_x64());
        assert!(jvm.contains(&"-Dos.name=Windows 10".to_string()), "os.version kuralı Windows 10'da çalışır");
        assert!(!jvm.contains(&"-XstartOnFirstThread".to_string()));
        assert!(!jvm.contains(&"-Xss1M".to_string()));
        assert_eq!(jvm.last().unwrap(), "${classpath}");
    }

    #[test]
    fn fabric_merges_with_parent() {
        let r = resolve(parse(FABRIC).unwrap(), Some(parse(VANILLA).unwrap())).unwrap();
        assert_eq!(r.id, "fabric-loader-0.16.9-1.21.1");
        assert_eq!(r.jar_id, "1.21.1");
        assert_eq!(r.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(r.java_major, 21);
        assert_eq!(r.asset_index.id, "17");
        assert!(r.client.is_some());
        let logging: Vec<_> = r.libraries.iter().filter(|l| l.name.starts_with("com.mojang:logging")).collect();
        assert_eq!(logging.len(), 1, "aynı kütüphane tekrarlanmaz, alt sürüm öncelikli");
        // Fabric'in Maven kütüphanesi adres ve yola çevrilir
        let asm = r.libraries.iter().find(|l| l.name.contains("asm")).unwrap().artifact().unwrap();
        assert_eq!(asm.path, "org/ow2/asm/asm/9.7/asm-9.7.jar");
        assert_eq!(asm.url, "https://maven.fabricmc.net/org/ow2/asm/asm/9.7/asm-9.7.jar");
        assert_eq!(asm.sha1.as_deref(), Some("aaaa"));
    }

    #[test]
    fn child_without_parent_is_rejected() {
        assert!(resolve(parse(FABRIC).unwrap(), None).is_err());
    }

    #[test]
    fn legacy_native_is_separate_from_classpath() {
        let v = parse(LEGACY).unwrap();
        let l = &v.libraries[0];
        let n = l.legacy_native(&Env::windows_x64()).unwrap();
        assert!(n.path.ends_with("natives-windows.jar"));
        assert!(l.artifact().unwrap().path.ends_with("2.9.4-nightly-20150209.jar"));
        assert!(l.legacy_native(&Env { os: Os::Osx, ..Env::windows_x64() }).is_none(), "osx sınıflandırıcısı dosyada yok");
    }
}
