# Kazma Launcher

Minecraft Java Edition için launcher: server kurma, server'a mod kurma ve katılan oyunculara
modları otomatik indirme. Microsoft hesabı ve alternatif (çevrimdışı) hesaplar desteklenir.
Temalar: Obsidian, Terminal.

## Yapı

| Klasör | İçerik | Durum |
|---|---|---|
| `crates/kazma-core` | Saf Rust çekirdek: sürüm JSON'u ve kurallar, Fabric profil birleştirme, indirme planı, başlatma komutu, Java (Adoptium), çevrimdışı UUID, davet ayrıştırma, mod manifesti + eşitleme planı, SHA-512 doğrulama, Ed25519 imza | 41 test geçiyor (`cargo test`) |
| `crates/kazma-cli` | Komut satırı: sürüm seç, server'a katıl, server sahibi araçları | 4 birim testi geçiyor |
| `crates/kazma-engine` | Kurulum, başlatma, instance kaydı, server'a katılma, mod eşitleme, Modrinth arama/kurma. İlerleme bir `Reporter` ile bildirilir | 21 test geçiyor |
| `src-tauri` | Tauri 2 kabuğu, motor fonksiyonlarını arayüze komut olarak açar (ince sarmalayıcılar) | Bu ortamda derlenmez; GitHub Actions'ta Windows'ta derlenir |
| `ui` | Arayüz (Obsidian ve Terminal temalı). `index.html` prototip, `live.js` Tauri içinde gerçek veriyi bağlar | Sahte arka uçla denendi |

## Windows'ta çalıştırma

1. Rust: https://rustup.rs (MSVC araç zinciri için "Visual Studio Build Tools" gerekir)
2. WebView2 çalışma zamanı (Windows 11'de hazır gelir)
3. `cargo install tauri-cli --version "^2"`
4. Proje klasöründe:
   ```
   cargo test                         # çekirdek testleri
   cd src-tauri
   cargo tauri icon ..\ui\logo.png    # ilk seferde: simge üret (512x512 PNG ver)
   cargo tauri dev                    # geliştirme penceresi
   cargo tauri build                  # NSIS kurulum dosyası
   ```

## Oyunu çevrimdışı hesapla başlatma

```
cargo run -p kazma-cli --release -- launch 1.21.1 Mustafa --fabric --ram 4096
cargo run -p kazma-cli --release -- launch 1.21.1 Mustafa --join play.example.com:25565
```

İlk çalıştırmada sürüm dosyaları, kütüphaneler, asset'ler ve Java (Eclipse Temurin) indirilir; sonraki
çalıştırmalar yalnızca eksikleri tamamlar. Her dosya SHA-1 ile, Java paketi SHA-256 ile doğrulanır.
Dosyalar `%APPDATA%\.kazma` altına yazılır (`--root` ile değişir). Java'yı kendin vermek için `--java YOL`.

Alternatif (çevrimdışı) hesapla yalnızca `online-mode=false` server'lara ve tek kişilik dünyaya girilebilir.

## Desteklenen yükleyiciler

| Yükleyici | Kurulum | Not |
|---|---|---|
| Vanilla | Mojang dosyaları | tüm sürümler |
| Fabric | meta.fabricmc.net profili | |
| Quilt | meta.quiltmc.org profili | |
| Forge | resmi kurucu (`--installClient`), SHA-1 doğrulamalı | 1.13 ve sonrası; ilk kurulum birkaç dakika sürer |
| NeoForge | resmi kurucu, SHA-1 doğrulamalı | yalnızca 1.20.2 ve sonrası |

Forge/NeoForge kurucuları bu geliştirme ortamında canlı denenemedi; ilk gerçek kurulumda hata çıkarsa
`<kök>\installers\last.log` dosyasına bakın.

## Aternos ve davetsiz server'lar

Arayüzde **Server'a Katıl → Sunucu adresi**: Kazma server'a Minecraft'ın "Server List Ping" protokolüyle sorar,
sürümünü ve Forge ipucunu bulur, o server için ayrı bir instance açar. Modları oyuncu Mod Gezgini'nden kendisi ekler.
Çevrimdışı hesapla yalnızca "cracked" açık server'lara girilir (Aternos'ta server sahibi açar).
Server sahibi isterse `kazma-cli make-manifest` ile mod listesini yayınlayıp **davet bağlantısıyla** otomatik eşitleme sağlar.

## Paket kodu (barındırma gerektirmeyen otomatik eşitleme)

Aternos gibi mod listesi yayınlamayan server'lar için: server sahibi mods klasöründen **paket kodu** üretir
(arayüzde *Server'a Katıl → Server sahibi*, ya da `kazma-cli make-code ... --mods KLASÖR`). Kod yalnızca
Modrinth sürüm kimliklerini taşır (60 mod ≈ 1 KB). Oyuncu kodu *Davet / paket kodu* alanına yapıştırınca Kazma
server'ın sürümünü kurar, modları Modrinth'ten indirip SHA-512 ile doğrular ve bağlanır. Kod imzasızdır;
bu yüzden arayüz her seferinde onay ister. Modrinth'te olmayan modlar koda girmez, ayrıca listelenir.
Sahibi isterse imzalı manifest + davet bağlantısı yolunu da kullanabilir (aşağıda).

## Sürüm seçimi

```
kazma-cli versions [--snapshots]                 # seçilebilir sürümler
kazma-cli launch 1.20.1 Mustafa                  # vanilla
kazma-cli launch 1.20.1 Mustafa --fabric         # en son kararlı Fabric
kazma-cli launch 1.20.1 Mustafa --fabric 0.15.11 # belirli Fabric sürümü
kazma-cli launch 1.20.1 Mustafa --forge          # Forge (ayrıca --quilt, --neoforge)
kazma-cli launch 1.20.1 Mustafa --forge --join my_world.aternos.me:12345
```

Her sürüm ve yükleyici kendi instance'ında çalışır (`instances/<sürüm>-<yükleyici>`).

## Server'a katılma (server'ın sürümüne geçerek)

```
kazma-cli join "kazma://join/play.example.com:25565?m=https%3A%2F%2F..." Mustafa
```

Launcher manifesti okur, **server hangi Minecraft ve Fabric sürümünü istiyorsa** o sürümü bu server'a
özel bir instance'ta (`instances/server-<adres>-<port>`) kurar, modları orada eşitler ve doğrudan bağlanır.
Oyuncunun kendi seçtiği sürüm ve modları etkilenmez. Fazla modlar silinmez, `.jar.disabled` olarak kenara alınır.

Server sahibi için:

```
kazma-cli make-key anahtar.txt                   # bir kez, gizli tut
kazma-cli make-manifest --name "Dostlar SMP" --game 1.21.1 --fabric 0.16.9 --mods C:\server\mods --key anahtar.txt --out kazma.json
kazma-cli make-invite play.example.com:25565 https://adresin/kazma.json
```

`make-manifest`, modların indirme adreslerini Modrinth'ten SHA-512 ile bulur. Modrinth'te olmayanlar için `--base-url`.
Manifest, anahtarınla imzalanır; oyuncular ilk katılımda parmak izini onaylar ve sonraki katılımlarda anahtar
değişirse katılım reddedilir.

## Mod eşitleme akışı

1. Server sahibi launcher'dan manifest üretir ve Ed25519 anahtarıyla imzalar (`sign::sign`).
2. Oyuncu `kazma://join/adres[:port]` açar (`invite::parse`).
3. Launcher manifesti çeker, `validate` şemayı, dosya adlarını ve indirme adreslerini denetler,
   `sign::verify` imzayı doğrular. Anahtar ilk katılımda saklanır; değişirse katılım reddedilir.
4. `manifest::plan` yerel `mods/` ile karşılaştırır: hazır, indirilecek, güncellenecek, devre dışı, yerelde kalacak.
5. Kullanıcı onaylar, dosyalar indirilir, her biri `verify::verify_sha512` ile doğrulanır.

Güvenlik kuralları testlerle sabitlenmiştir: yol atlatma (`../x.jar`), http adresleri,
`cdn.modrinth.com.evil.example` türü alan adı hileleri, özeti bozuk dosya, imzası bozulmuş manifest.

## Sıradaki işler

0. Doğrulama: komut satırı aracı Windows'ta gerçek bir 1.21.1 ve Fabric başlatmasıyla denenecek; çıkan hatalar düzeltilecek.
1. Microsoft girişi: device code akışı (Microsoft → Xbox Live → XSTS → Minecraft hizmetleri). Kendi Azure uygulama kaydın gerekir.
2. Tauri tarafına ağ katmanı: kazma-cli'deki indirme kodunu ortak bir kütüphaneye taşı, ilerleme olaylarını arayüze ilet.
3. NeoForge/Forge/Quilt kurulumu.
4. Eski sürümler: `map_to_resources` asset düzeni.
5. Server yöneticisi: jar indirme, süreç başlatma, konsol akışı, UPnP/tünel, karma giriş eklentisi.
6. Mod gezgini: Modrinth API v2.
