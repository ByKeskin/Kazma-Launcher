//! Ağ ve dosya yardımcıları: indirme, doğrulama, atomik yazma, zip açma.

use crate::report::Reporter;
use kazma_core::plan::DownloadItem;
use sha1::{Digest, Sha1};
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub type R<T> = Result<T, String>;

pub fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .user_agent("kazma-cli/0.1 (Minecraft launcher; https://github.com/ByKeskin/Kazma-Launcher)")
            .build()
    })
}

pub fn get_bytes(url: &str, max: u64) -> R<Vec<u8>> {
    let mut last = String::new();
    for attempt in 0..3 {
        match agent().get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                match resp.into_reader().take(max + 1).read_to_end(&mut buf) {
                    Ok(_) if buf.len() as u64 > max => return Err(format!("{url}: yanıt beklenenden büyük")),
                    Ok(_) => return Ok(buf),
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(500 * (attempt + 1)));
    }
    Err(format!("{url}: indirilemedi ({last})"))
}

pub fn get_text(url: &str, max: u64) -> R<String> {
    String::from_utf8(get_bytes(url, max)?).map_err(|_| format!("{url}: geçerli UTF-8 değil"))
}

pub fn sha1_hex(b: &[u8]) -> String {
    hex::encode(Sha1::digest(b))
}

fn file_ok(path: &Path, it: &DownloadItem) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    if let Some(sz) = it.size {
        return meta.len() == sz;
    }
    match (&it.sha1, fs::read(path)) {
        (Some(h), Ok(b)) => sha1_hex(&b).eq_ignore_ascii_case(h),
        _ => true,
    }
}

pub fn write_atomic(path: &Path, data: &[u8]) -> R<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let tmp = path.with_extension("part");
    fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn download_item(root: &Path, it: &DownloadItem) -> R<()> {
    let dest = root.join(&it.rel_path);
    if file_ok(&dest, it) {
        return Ok(());
    }
    let data = get_bytes(&it.url, 400 << 20)?;
    if let Some(h) = &it.sha1 {
        if !sha1_hex(&data).eq_ignore_ascii_case(h) {
            return Err(format!("{}: SHA-1 uyuşmuyor, dosya atıldı", it.rel_path));
        }
    }
    write_atomic(&dest, &data)
}

pub fn download_all(root: &Path, items: Vec<DownloadItem>, label: &str, rep: &dyn Reporter) -> R<()> {
    let total = items.len();
    if total == 0 {
        return Ok(());
    }
    rep.stage(&format!("{label}: {total} dosya kontrol ediliyor"));
    let queue = Mutex::new(VecDeque::from(items));
    let errors = Mutex::new(Vec::<String>::new());
    let done = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..12.min(total) {
            s.spawn(|| loop {
                let next = queue.lock().unwrap().pop_front();
                let Some(it) = next else { break };
                if let Err(e) = download_item(root, &it) {
                    errors.lock().unwrap().push(e);
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 25 == 0 || n == total {
                    rep.progress(label, n, total);
                }
            });
        }
    });
    let errs = errors.into_inner().unwrap();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("{} dosya indirilemedi, ilki: {}", errs.len(), errs[0]))
    }
}

pub fn unzip(zip_bytes: &[u8], dest: &Path) -> R<()> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue }; // yol kaçışı olan girişler atlanır
        let out = dest.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = out.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut w = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unzip_blocks_path_traversal() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("ok/a.txt", o).unwrap();
            w.write_all(b"a").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("kazma-unzip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        unzip(&buf, &dir).unwrap();
        assert!(dir.join("ok/a.txt").is_file());
        assert!(!dir.parent().unwrap().join("evil.txt").exists(), "yol kaçışı engellenmeli");
        let _ = fs::remove_dir_all(&dir);
    }
}
