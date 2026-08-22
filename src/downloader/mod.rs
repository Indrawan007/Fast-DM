pub mod aria2;
pub mod types;
pub mod youtube;

use crate::config::Config;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use tokio::sync::{mpsc, Mutex, RwLock};
use types::*;
use url::Url;
use uuid::Uuid;

pub struct DownloadEngine {
    downloads: Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &'static Config,
}

#[allow(dead_code)]
impl DownloadEngine {
    pub fn new(event_tx: mpsc::UnboundedSender<DownloadEvent>) -> Self {
        Self {
            downloads: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            config: Config::load(),
        }
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
        let save = save_dir
            .unwrap_or(&self.config.download_dir)
            .to_string();

        // Ensure save dir exists
        let _ = std::fs::create_dir_all(&save);

        let fname = filename
            .map(|f| sanitize_filename(f))
            .unwrap_or_else(|| extract_filename_from_url(url));

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
        let max = usize::from(self.config.max_concurrent.max(1));
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
        let config = self.config;

        if active >= max {
            let mut i = info.lock().await;
            i.status = DownloadStatus::Queued;
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Progress(i.clone()));
            return;
        }

        spawn_supervised(self.downloads.clone(), info, tx, config);
    }

    pub async fn pause_download(&self, id: &str) {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            if matches!(i.status, DownloadStatus::Downloading | DownloadStatus::Resolving) {
                i.status = DownloadStatus::Paused;
                i.speed = 0;
                let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
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
            i.status = DownloadStatus::Cancelled;
            i.speed = 0;
            let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
        }
    }

    pub async fn clear_download(&self, id: &str) {
        // Cancel dulu supaya background task (aria2/yt-dlp) berhenti
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            i.status = DownloadStatus::Cancelled;
            i.speed = 0;
        }
        drop(downloads);
        self.downloads.write().await.remove(id);
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
    config: &'static Config,
) {
    tokio::spawn(async move {
        let is_yt = { info.lock().await.is_youtube };

        if is_yt {
            youtube::download(info, tx.clone(), config).await;
        } else {
            aria2::download(info, tx.clone(), config).await;
        }

        // Slot bebas → jalankan antrian tertua
        promote_next(downloads, tx, config).await;
    });
}

/// Cari download Queued tertua dan jalankan jika ada slot kosong
async fn promote_next(
    downloads: Arc<RwLock<HashMap<String, Arc<Mutex<DownloadInfo>>>>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &'static Config,
) {
    let max = usize::from(config.max_concurrent.max(1));

    let next = {
        let map = downloads.read().await;
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
        } else {
            oldest.map(|(_, i)| i)
        }
    };

    if let Some(info) = next {
        {
            let mut i = info.lock().await;
            // Bisa saja sudah di-cancel/di-resume ke slot lain — skip
            if i.status != DownloadStatus::Queued {
                return;
            }
            i.status = DownloadStatus::Resolving;
            let _ = tx.send(DownloadEvent::Progress(i.clone()));
        }
        spawn_supervised(downloads, info, tx, config);
    }
}

static RE_INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap());

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
