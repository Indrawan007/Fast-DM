pub mod aria2;
pub mod types;
pub mod youtube;

use crate::config::Config;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::{mpsc, Mutex, RwLock};
use types::*;
use url::Url;
use uuid::Uuid;

pub struct DownloadEngine {
    downloads: Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    config: Arc<RwLock<Config>>,
    dirty: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl DownloadEngine {
    pub fn new(event_tx: mpsc::UnboundedSender<DownloadEvent>) -> Self {
        // Restore session sebelumnya — yang tadinya aktif jadi Paused agar bisa di-resume
        let mut map = HashMap::new();
        for mut d in load_session() {
            if matches!(
                d.status,
                DownloadStatus::Downloading | DownloadStatus::Resolving | DownloadStatus::Queued
            ) {
                d.status = DownloadStatus::Paused;
                d.speed = 0;
            }
            d.pid = None;
            map.insert(d.id.clone(), Arc::new(Mutex::new(d)));
        }
        let downloads = Arc::new(RwLock::new(map));

        let dirty = Arc::new(AtomicBool::new(false));

        // Flusher: tulis session.json maks 1x/2 detik, hanya jika ada perubahan
        let downloads_flush = downloads.clone();
        let dirty_flush = dirty.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                flush_session(&downloads_flush, &dirty_flush).await;
            }
        });

        Self {
            downloads,
            event_tx,
            config: Arc::new(RwLock::new(Config::load().clone())),
            dirty,
        }
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    /// Simpan config ke disk + apply live (berlaku untuk download baru)
    pub async fn update_config(&self, cfg: Config) -> Result<(), String> {
        // Validasi input sebelum disimpan — nilai invalid bikin aria2 gagal start
        if cfg.max_connections == 0 || cfg.max_connections > 32 {
            return Err("Koneksi harus 1–32".into());
        }
        if cfg.max_concurrent == 0 || cfg.max_concurrent > 10 {
            return Err("Download bersamaan harus 1–10".into());
        }
        if cfg.timeout == 0 {
            return Err("Timeout harus > 0".into());
        }
        if cfg.max_overall_speed != "0" && !is_valid_speed_limit(&cfg.max_overall_speed) {
            return Err("Speed limit tidak valid (contoh: 0, 512K, 2M)".into());
        }
        cfg.save().map_err(|e| e.to_string())?;
        *self.config.write().await = cfg;
        Ok(())
    }

    pub async fn add_download(
        &self,
        url: &str,
        filename: Option<&str>,
        save_dir: Option<&str>,
        auto_start: bool,
        headers: HashMap<String, String>,
        quality: Option<String>,
    ) -> String {
        let id = format!("dl_{}", Uuid::new_v4().to_string()[..8].to_string());
        let save = match save_dir {
            Some(d) => d.to_string(),
            None => self.config.read().await.download_dir.clone(),
        };

        // Ensure save dir exists
        let _ = std::fs::create_dir_all(&save);

        let fname = filename
            .map(|f| sanitize_filename(f))
            .unwrap_or_else(|| extract_filename_from_url(url));

        // Deduplikasi: download live (url+dir+file sama) → kembalikan yang sudah ada.
        // Tanpa ini, dua proses aria2 bisa menulis file yang sama dan saling korup.
        let live = {
            let downloads = self.downloads.read().await;
            let mut found = None;
            for (existing_id, info) in downloads.iter() {
                let i = info.lock().await;
                let same = i.url == url && i.save_dir == save && i.filename == fname;
                let is_live = matches!(
                    i.status,
                    DownloadStatus::Queued
                        | DownloadStatus::Resolving
                        | DownloadStatus::Downloading
                        | DownloadStatus::Paused
                );
                if same && is_live {
                    found = Some(existing_id.clone());
                    break;
                }
            }
            found
        };
        if let Some(existing) = live {
            tracing::info!("Duplicate download ignored: {} ({})", fname, existing);
            return existing;
        }

        let is_yt = youtube::is_youtube_url(url);

        let mut info = DownloadInfo::new(
            id.clone(),
            url.to_string(),
            fname,
            save,
            headers,
            quality,
        );
        info.is_youtube = is_yt;

        let info = Arc::new(Mutex::new(info));
        self.downloads.write().await.insert(id.clone(), info.clone());
        self.mark_dirty();

        if auto_start {
            self.start_download(&id).await;
        }

        id
    }

    pub async fn start_download(&self, id: &str) {
        let info = {
            let downloads = self.downloads.read().await;
            match downloads.get(id) {
                Some(i) => i.clone(),
                None => return,
            }
        };

        // Tegakkan max_concurrent: jika slot penuh, antri (Queued)
        let max = usize::from(self.config.read().await.max_concurrent.max(1));
        let active = {
            let downloads = self.downloads.read().await;
            let mut n = 0usize;
            for other in downloads.values() {
                let status = other.lock().await.status;
                if matches!(status, DownloadStatus::Downloading | DownloadStatus::Resolving) {
                    n += 1;
                }
            }
            n
        };

        let tx = self.event_tx.clone();
        let config = self.config.read().await.clone();

        if active >= max {
            let mut i = info.lock().await;
            i.status = DownloadStatus::Queued;
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Progress(i.clone()));
            return;
        }

        // Tandai percobaan ke-N (dipakai retry tracking)
        {
            let mut i = info.lock().await;
            i.retry_count = i.retry_count.saturating_add(1);
        }

        spawn_supervised(self.downloads.clone(), info, tx, config, self.dirty.clone());
    }

    pub async fn pause_download(&self, id: &str) {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            if matches!(i.status, DownloadStatus::Downloading | DownloadStatus::Resolving) {
                // Kill child proses LANGSUNG (jangan menunggu baris output berikutnya,
                // bisa lama/hang kalau aria2/yt-dlp sedang stall).
                kill_child_pid(i.pid);
                i.status = DownloadStatus::Paused;
                i.speed = 0;
                let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
                self.mark_dirty();
            }
        }
    }

    pub async fn resume_download(&self, id: &str) {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let status = {
                let i = info.lock().await;
                i.status
            };
            if matches!(status, DownloadStatus::Paused | DownloadStatus::Error) {
                self.start_download(id).await;
            }
        }
    }

    pub async fn cancel_download(&self, id: &str) {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            kill_child_pid(i.pid);
            i.status = DownloadStatus::Cancelled;
            i.speed = 0;
            let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
            self.mark_dirty();
        }
    }

    pub async fn clear_download(&self, id: &str) {
        // Cancel dulu supaya background task (aria2/yt-dlp) berhenti
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            kill_child_pid(i.pid);
            i.status = DownloadStatus::Cancelled;
            i.speed = 0;
        }
        drop(downloads);
        self.downloads.write().await.remove(id);
        self.mark_dirty();
    }

    pub async fn get_download(&self, id: &str) -> Option<DownloadInfo> {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            Some(info.lock().await.clone())
        } else {
            None
        }
    }

    pub async fn get_all_downloads(&self) -> Vec<DownloadInfo> {
        let downloads = self.downloads.read().await;
        let mut result = Vec::with_capacity(downloads.len());
        for info in downloads.values() {
            result.push(info.lock().await.clone());
        }
        result
    }
}

/// Jalankan download lalu promote antrian berikutnya saat selesai
fn spawn_supervised(
    downloads: Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: Config,
    dirty: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let is_yt = { info.lock().await.is_youtube };

        if is_yt {
            youtube::download(info, tx.clone(), &config).await;
        } else {
            aria2::download(info, tx.clone(), &config).await;
        }

        // Status terminal (completed/error) → persist ke session.json
        dirty.store(true, Ordering::SeqCst);

        // Slot bebas → jalankan antrian tertua
        promote_next(downloads, tx, config, dirty).await;
    });
}

/// Cari download Queued tertua dan jalankan jika ada slot kosong
async fn promote_next(
    downloads: Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: Config,
    dirty: Arc<AtomicBool>,
) {
    let max = usize::from(config.max_concurrent.max(1));

    let next = {
        // write lock: pemilihan + penandaan status dilakukan ATOMIK,
        // sehingga dua task yang selesai bersamaan tidak bisa sama-sama
        // memilih antrian yang sama (over-slot / dobel spawn).
        let map = downloads.write().await;
        let mut active = 0usize;
        let mut oldest: Option<(i64, Arc<Mutex<DownloadInfo>>)> = None;

        for info in map.values() {
            let i = info.lock().await;
            match i.status {
                DownloadStatus::Downloading | DownloadStatus::Resolving => active += 1,
                DownloadStatus::Queued => {
                    if oldest.as_ref().map_or(true, |(t, _)| i.created < *t) {
                        oldest = Some((i.created, info.clone()));
                    }
                }
                _ => {}
            }
        }

        if active >= max {
            None
        } else if let Some((_, info)) = oldest {
            // Scope guard: MutexGuard harus drop SEBELUM `info` dipindah keluar
            let started = {
                let mut i = info.lock().await;
                if i.status != DownloadStatus::Queued {
                    false // sudah di-cancel / sudah diambil task lain
                } else {
                    i.status = DownloadStatus::Resolving;
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                    true
                }
            };
            if started {
                Some(info)
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(info) = next {
        spawn_supervised(downloads, info, tx, config, dirty);
    }
}

/// Kirim SIGTERM ke child download (aria2c/yt-dlp) supaya berhenti segera.
/// SIGTERM dipilih (bukan SIGKILL) agar aria2/yt-dlp sempat menulis control file
/// sehingga download bisa di-resume.
fn kill_child_pid(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}

static RE_INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap());

fn session_file() -> std::path::PathBuf {
    Config::config_dir().join("session.json")
}

fn load_session() -> Vec<DownloadInfo> {
    match std::fs::read_to_string(session_file()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Tulis snapshot session secara atomic (tmp + rename), dibatasi 200 entri terbaru
async fn flush_session(
    downloads: &Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    dirty: &AtomicBool,
) {
    if !dirty.swap(false, Ordering::SeqCst) {
        return;
    }

    let mut all: Vec<DownloadInfo> = {
        let map = downloads.read().await;
        let mut v = Vec::with_capacity(map.len());
        for info in map.values() {
            v.push(info.lock().await.clone());
        }
        v
    };

    all.sort_by_key(|d| d.created);
    if all.len() > 200 {
        all = all.split_off(all.len() - 200);
    }

    if let Ok(json) = serde_json::to_string(&all) {
        let path = session_file();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Validasi format --max-overall-download-limit aria2: angka, atau angka + K/M/G (opsional)
/// contoh: "0", "512K", "2M", "10G"
fn is_valid_speed_limit(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let (num, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()),
    );
    let num_ok = !num.is_empty() && num.parse::<u64>().is_ok();
    if !unit.is_empty() && !matches!(unit.to_ascii_uppercase().as_str(), "K" | "M" | "G") {
        return false;
    }
    num_ok
}

/// Sanitize filename
pub fn sanitize_filename(name: &str) -> String {
    let name = name.split('?').next().unwrap_or(name);
    let name = name.split('#').next().unwrap_or(name);

    let cleaned = RE_INVALID_CHARS.replace_all(name, "_").to_string();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ');

    if cleaned.is_empty() {
        format!("download_{}", chrono::Utc::now().timestamp())
    } else if cleaned.len() > 200 {
        // Truncate on a char boundary — raw byte slicing panics on multi-byte UTF-8
        let mut end = 200;
        while !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned[..end].to_string()
    } else {
        cleaned.to_string()
    }
}

/// Extract filename from URL
pub fn extract_filename_from_url(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        let path = parsed.path();
        let decoded = urlencoding::decode(path).unwrap_or_default();
        let basename = Path::new(decoded.as_ref())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let cleaned = sanitize_filename(basename);
        if !cleaned.is_empty() && cleaned.contains('.') {
            return cleaned;
        }
    }

    format!("download_{}", chrono::Utc::now().timestamp())
}
