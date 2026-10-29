# Kazma Launcher

Minecraft Java Edition için launcher: server kurma, server'a mod kurma ve katılan oyunculara
modları otomatik indirme. Microsoft hesabı ve alternatif (çevrimdışı) hesaplar desteklenir.
Temalar: Obsidian, Terminal.

## Yapı

| Klasör | İçerik | Durum |
|---|---|---|
| `crates/kazma-core` | Saf Rust çekirdek: Mojang manifesti, çevrimdışı UUID, davet ayrıştırma, mod manifesti + eşitleme planı, SHA-512 doğrulama, Ed25519 imza | 19 test geçiyor (`cargo test`) |
| `src-tauri` | Tauri 2 kabuğu, çekirdek fonksiyonları arayüze komut olarak açar | Yazıldı, bu ortamda derlenmedi (Windows gerekir) |
| `ui` | Arayüz (Obsidian ve Terminal temalı), şimdilik örnek verilerle | Prototip |

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

1. Microsoft girişi: device code akışı (Microsoft → Xbox Live → XSTS → Minecraft hizmetleri). Kendi Azure uygulama kaydın gerekir.
2. Sürüm kurulumu: `version.json` indirme, kütüphane kuralları, asset'ler, natives, Java 21 (Adoptium) kurulumu, başlatma komutu.
3. Fabric/NeoForge kurulumu (`mojang::fabric_profile_url` hazır).
4. Ağ katmanı: Tauri tarafında `reqwest` ile manifest ve dosya indirme (arayüz doğrudan indirmez).
5. Server yöneticisi: jar indirme, süreç başlatma, konsol akışı, UPnP/tünel, karma giriş eklentisi.
6. Mod gezgini: Modrinth API v2.
