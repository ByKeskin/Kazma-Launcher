//! İlerleme bildirimi. Arayüz olay gönderir, komut satırı ekrana yazar, testler hiçbir şey yapmaz.

pub trait Reporter: Sync {
    /// Yeni bir aşama başladı ("Java indiriliyor" gibi).
    fn stage(&self, text: &str);
    /// Bir aşamanın sayısal ilerlemesi.
    fn progress(&self, label: &str, done: usize, total: usize);
}

pub struct StdoutReporter;
impl Reporter for StdoutReporter {
    fn stage(&self, text: &str) {
        println!("{text}");
    }
    fn progress(&self, label: &str, done: usize, total: usize) {
        if done == total || done % 250 == 0 {
            println!("  {label}: {done}/{total}");
        }
    }
}

pub struct NullReporter;
impl Reporter for NullReporter {
    fn stage(&self, _: &str) {}
    fn progress(&self, _: &str, _: usize, _: usize) {}
}
