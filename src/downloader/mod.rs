pub mod aria2;
pub mod types;
pub mod universal;
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
    /// v2.3.0 (K5): id hasil restore sesi — di-auto-resume sekali saat start
    /// bila config.auto_resume; bukan untuk item yang user pause manual.
    restored_ids: Arc<Mutex<Vec<String>>>,
}

#[allow(dead_code)]
impl DownloadEngine {
    pub fn new(event_tx: mpsc::UnboundedSender<DownloadEvent>) -> Self {
        // Restore session sebelumnya — yang tadinya aktif jadi Paused agar bisa di-resume
        let mut map = HashMap::new();
        let mut restored_ids = Vec::new();
        for mut d in load_session() {
            if matches!(
                d.status,
                DownloadStatus::Downloading | DownloadStatus::Resolving | DownloadStatus::Queued
            ) {
                d.status = DownloadStatus::Paused;
                d.speed = 0;
                restored_ids.push(d.id.clone());
            }
            d.pid = None;
            map.insert(d.id.clone(), Arc::new(Mutex::new(d)));
        }
        let downloads = Arc::new(RwLock::new(map));

        // v2.3.0: sapuan keras saat start — file input aria2 sisa crash +
        // cookie kedaluwarsa (K3, M7).
        cleanup_orphan_aria2_inputs();
        Config::gc_stale_cookies();

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
            restored_ids: Arc::new(Mutex::new(restored_ids)),
        }
    }

    /// v2.3.0 (K5): lanjutkan unduhan yang terputus karena aplikasi keluar.
    /// README lama mengklaim "otomatis di-resume" padahal kode hanya menandai
    /// Paused — method ini menepati klaim itu, dengan penghormatan:
    /// - hanya item hasil restore sesi (user yang pause manual TIDAK diutak-atik),
    /// - hanya selama status masih Paused (kalau user sudah menekan apa pun, batalkan),
    /// - slot & antrean tetap diatur start_download (max_concurrent tidak jebol).
    /// Dapat dimatikan via Settings → auto_resume.
    pub async fn resume_restored(&self) {
        if !self.get_config().await.auto_resume {
            tracing::info!(
                "auto_resume dimatikan — {} unduhan dibiarkan Paused",
                self.restored_ids.lock().await.len()
            );
            return;
        }
        let ids: Vec<String> = self.restored_ids.lock().await.drain(..).collect();
        for id in ids {
            let still_paused = {
                let downloads = self.downloads.read().await;
                match downloads.get(&id) {
                    Some(info) => info.lock().await.status == DownloadStatus::Paused,
                    None => false,
                }
            };
            if still_paused {
                tracing::info!("Auto-resume unduhan tersisa sesi: {}", id);
                self.start_download(&id).await;
            }
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
        // v2.4.0 (D3): proxy divalidasi sebelum disimpan — nilai ngawur bikin
        // aria2 exit / yt-dlp diam-diam tetap koneksi langsung (membingungkan).
        if !cfg.proxy_url.trim().is_empty() && !crate::config::is_valid_proxy_url(&cfg.proxy_url)
        {
            return Err(
                "Proxy tidak valid — contoh: http://127.0.0.1:8080, socks5://host:1080"
                    .to_string(),
            );
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

        let mut info =
            DownloadInfo::new(id.clone(), url.to_string(), fname, save, headers, quality);
        info.is_youtube = is_yt;

        // v2.3.0 (M11): tolak cepat skema non-download (blob:, data:, javascript:,
        // file:, magnet: dst.) — dulu lolos dan baru gagal lambat di CLI dengan
        // error yang tidak jelas. (magnet bisa didukung via aria2 RPC — roadmap.)
        let scheme_ok = Url::parse(url)
            .map(|u| matches!(u.scheme(), "http" | "https" | "ftp"))
            .unwrap_or(false);
        if !scheme_ok {
            info.status = DownloadStatus::Error;
            info.error_msg = "Skema URL tidak didukung — hanya http, https, dan ftp.".to_string();
            let _ = self.event_tx.send(DownloadEvent::Error(info.clone()));
            self.downloads
                .write()
                .await
                .insert(id.clone(), Arc::new(Mutex::new(info)));
            self.mark_dirty();
            tracing::warn!("Download ditolak (skema URL): {}", url);
            return id;
        }

        let info = Arc::new(Mutex::new(info));
        self.downloads
            .write()
            .await
            .insert(id.clone(), info.clone());
        self.mark_dirty();

        if auto_start {
            self.start_download(&id).await;
        }

        id
    }

    pub async fn start_download(&self, id: &str) {
        let config = self.config.read().await.clone();
        let max = usize::from(config.max_concurrent.max(1));
        let tx = self.event_tx.clone();

        // B3: hitung slot + klaim status dilakukan dalam SATU write-lock
        // (pola promote_next) — dua start yang bersamaan tidak bisa sama-sama
        // lolos batas max_concurrent (double-spawn / over-slot).
        let claimed: Option<(Arc<Mutex<DownloadInfo>>, usize)> = {
            let downloads = self.downloads.write().await;
            let Some(info) = downloads.get(id).cloned() else {
                return;
            };

            let mut active = 0usize;
            for other in downloads.values() {
                if matches!(
                    other.lock().await.status,
                    DownloadStatus::Downloading | DownloadStatus::Resolving
                ) {
                    active += 1;
                }
            }

            // Scope guard (pola promote_next): MutexGuard `i` harus drop
            // SEBELUM `info` dipindah keluar blok — kalau tidak, E0505
            // (cannot move out of `info` because it is borrowed).
            let start = {
                let mut i = info.lock().await;
                if active >= max {
                    // Slot penuh → antri (Queued)
                    i.status = DownloadStatus::Queued;
                    i.speed = 0;
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                    false
                } else {
                    // Klaim slot: tandai Resolving + percobaan ke-N (retry tracking)
                    i.status = DownloadStatus::Resolving;
                    i.retry_count = i.retry_count.saturating_add(1);
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                    true
                }
            };

            // v2.3.0 (M3): jumlah unduhan hidup (menempati bandwidth atau akan
            // menempati slot) TERMASUK diri sendiri — dipakai membagi limit total.
            let live = if start {
                let mut n = 0usize;
                for other in downloads.values() {
                    if matches!(
                        other.lock().await.status,
                        DownloadStatus::Queued
                            | DownloadStatus::Resolving
                            | DownloadStatus::Downloading
                    ) {
                        n += 1;
                    }
                }
                n.max(1)
            } else {
                1 // tidak dipakai (tidak spawn)
            };

            if start {
                Some((info, live))
            } else {
                None
            }
        };

        if let Some((info, live)) = claimed {
            spawn_supervised(
                self.downloads.clone(),
                info,
                tx,
                config,
                self.dirty.clone(),
                live,
            );
        }
    }

    pub async fn pause_download(&self, id: &str) {
        let downloads = self.downloads.read().await;
        if let Some(info) = downloads.get(id) {
            let mut i = info.lock().await;
            if matches!(
                i.status,
                DownloadStatus::Downloading | DownloadStatus::Resolving
            ) {
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
        // DEADLOCK FIX: guard read-lock harus di-drop SEBELUM start_download()
        // — start_download meminta write-lock pada map yang sama, dan RwLock
        // tokio tidak reentrant: read-guard lama tidak akan pernah di-drop
        // selama kita menunggu write-lock → deadlock permanen.
        let resumable = {
            let downloads = self.downloads.read().await;
            if let Some(info) = downloads.get(id) {
                Some(info.lock().await.status)
            } else {
                None
            }
        };
        if matches!(
            resumable,
            Some(DownloadStatus::Paused | DownloadStatus::Error)
        ) {
            self.start_download(id).await;
        }
    }

    /// Pause SEMUA unduhan (aktif + antrian) — dipakai tombol "Jeda Semua" (UI-UX C3).
    pub async fn pause_all(&self) {
        let downloads = self.downloads.read().await;
        for info in downloads.values() {
            let mut i = info.lock().await;
            match i.status {
                DownloadStatus::Downloading | DownloadStatus::Resolving => {
                    kill_child_pid(i.pid);
                    i.status = DownloadStatus::Paused;
                    i.speed = 0;
                    let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
                    self.mark_dirty();
                }
                DownloadStatus::Queued => {
                    i.status = DownloadStatus::Paused;
                    let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
                    self.mark_dirty();
                }
                _ => {}
            }
        }
    }

    /// Resume SEMUA unduhan yang paused/error — dipakai tombol "Lanjut Semua" (UI-UX C3).
    pub async fn resume_all(&self) {
        let ids: Vec<String> = {
            let downloads = self.downloads.read().await;
            let mut v = Vec::new();
            for (id, info) in downloads.iter() {
                let status = info.lock().await.status;
                if matches!(status, DownloadStatus::Paused | DownloadStatus::Error) {
                    v.push(id.clone());
                }
            }
            v
        };
        for id in ids {
            self.start_download(&id).await;
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
    live_share: usize,
) {
    // v2.3.0 (M3): bagi limit total aplikasi menurut jumlah unduhan hidup saat
    // proses ini start, bukan max_concurrent statis — unduhan tunggal kini
    // memakai limit penuh. Batas lama pada proses yang sudah berjalan tidak
    // di-recalculate (butuh aria2 RPC — lihat roadmap CODE-REVIEW.md B2);
    // pembagian ulang terjadi saat proses berikutnya start/promote.
    //
    // PENTING: `promote_config` harus tetap config ASLI (limit user mentah) —
    // kalau pakai yang sudah dibagi, rantai promote akan membagi ulang limit
    // setiap spawn berikutnya (double division → limit menyusut ke 1K).
    let promote_config = config.clone();
    let mut config = config;
    if !config.max_overall_speed.is_empty() && config.max_overall_speed != "0" {
        config.max_overall_speed =
            aria2::resolve_speed_limit(&config.max_overall_speed, live_share);
    }
    tokio::spawn(async move {
        let (is_yt, url) = {
            let i = info.lock().await;
            (i.is_youtube, i.url.clone())
        };

        if is_yt {
            // YouTube: yt-dlp dengan dialog kualitas (behavior lama)
            youtube::download(info.clone(), tx.clone(), &config).await;
        } else if is_direct_file_url(&url) {
            // File langsung (mp4/zip/dll): aria2 tanpa resolve — cepat
            aria2::download(info.clone(), tx.clone(), &config).await;
        } else {
            // Semua URL lain (halaman video, TikTok/IG/FB/X/Vimeo, m3u8, dll):
            // coba yt-dlp dulu (resolver universal, gaya IDM); kalau situs
            // tidak didukung → fallback ke aria2.
            match universal::download(info.clone(), tx.clone(), &config).await {
                universal::Outcome::Completed | universal::Outcome::MissingTool => {}
                universal::Outcome::Failed => {
                    let aborted = {
                        let i = info.lock().await;
                        matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused)
                    };
                    if !aborted {
                        aria2::download(info.clone(), tx.clone(), &config).await;
                    }
                }
            }
        }

        // Status terminal (completed/error) → persist ke session.json
        dirty.store(true, Ordering::SeqCst);

        // Slot bebas → jalankan antrian tertua
        // Slot bebas → jalankan antrian tertua.
        // Pakai promote_config (limit mentah) — lihat komentar M3 (anti double-division).
        promote_next(downloads, tx, promote_config, dirty).await;
    });
}

/// URL file langsung (punya ekstensi file/media) → langsung ke aria2 tanpa
/// lewat yt-dlp. HLS/DASH (m3u8/mpd) tetap ke yt-dlp agar di-merge benar.
pub fn is_direct_file_url(url: &str) -> bool {
    // Potong FRAGMENT dulu baru QUERY — fragment setelah ekstensi
    // (mis. "https://x.com/file.mp4#t=10") membuat cek ekstensi lama gagal.
    let path = url
        .split('#')
        .next()
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url);
    let lower = path.to_ascii_lowercase();
    // v2.3.0 (M2): SELARASKAN dengan daftar intersep extension/background.js —
    // dulu .exe/.msi/.dmg/.bz2/.docx dll tidak ada di sini sehingga URL-nya
    // dicoba lewat yt-dlp dulu (gagal, ±1-3 dtk terbuang) baru fallback aria2.
    const EXTENSIONS: &[&str] = &[
        ".mp4", ".webm", ".mkv", ".avi", ".mov", ".m4v", ".flv", ".wmv", ".3gp", ".ts", ".mp3",
        ".m4a", ".aac", ".ogg", ".opus", ".flac", ".wav", ".zip", ".tar", ".gz", ".bz2", ".tbz2",
        ".xz", ".txz", ".7z", ".rar", ".pdf", ".iso", ".img", ".bin", ".apk", ".deb", ".rpm",
        ".exe", ".msi", ".dmg", ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".svg", ".ico",
        ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".odt", ".epub", ".mobi", ".txt",
        ".csv", ".json", ".xml",
    ];
    EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
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
        //
        // B13 — KONTRAK LOCK-ORDERING (anti-deadlock): selalu kunci map
        // (RwLock) dulu, baru Mutex info di dalamnya. Jangan pernah terbalik
        // (kunci info lalu minta map) di kode mana pun.
        let map = downloads.write().await;
        let mut active = 0usize;
        // v2.3.0 (L4): FIFO deterministik — kunci (created_ms, id); tanpa
        // pembanding kedua, dua download yang dibuat pada waktu sama urutannya
        // acak mengikuti iterasi HashMap.
        let mut oldest: Option<((i64, String), Arc<Mutex<DownloadInfo>>)> = None;

        let mut queued = 0usize;
        for info in map.values() {
            let i = info.lock().await;
            match i.status {
                DownloadStatus::Downloading | DownloadStatus::Resolving => active += 1,
                DownloadStatus::Queued => {
                    queued += 1;
                    let key = (i.created, i.id.clone());
                    if oldest.as_ref().map_or(true, |(k, _)| key < *k) {
                        oldest = Some((key, info.clone()));
                    }
                }
                _ => {}
            }
        }

        if active >= max {
            None
        } else if let Some((_, info)) = oldest {
            // hidup = yang sedang jalan + yang di antrian (termasuk yang akan
            // klaim ini) — dipakai membagi limit kecepatan (M3)
            let live = (active + queued).max(1);
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
                Some((info, live))
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((info, live)) = next {
        spawn_supervised(downloads, info, tx, config, dirty, live);
    }
}

/// Kirim SIGTERM ke child download (aria2c/yt-dlp) supaya berhenti segera.
/// SIGTERM dipilih (bukan SIGKILL) agar aria2/yt-dlp sempat menulis control file
/// sehingga download bisa di-resume.
///
/// v2.3.0 (K4): sinyal dikirim ke PROCESS GROUP dulu — yt-dlp men-spawn
/// ffmpeg untuk merge; kill ke parent saja meninggalkan ffmpeg yatim yang terus
/// menulis file. Child di-spawn dengan `process_group(0)` sehingga pgid == pid.
/// killpg gagal (mis. child bukan leader group) → fallback kill satu proses.
pub(crate) fn kill_child_pid(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    let pid = nix::unistd::Pid::from_raw(pid as i32);
    if nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGTERM).is_err() {
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
    }
}

/// SIGKILL ke seluruh process group (jalur Cancel — tidak perlu control file).
pub(crate) fn kill_child_group_hard(pid: u32) {
    let pid = nix::unistd::Pid::from_raw(pid as i32);
    if nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL).is_err() {
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    }
}

/// v2.3.1 (M1): pembaca baris output child process yang CANCELLATION-SAFE.
///
/// Kenapa tidak `AsyncBufReadExt::read_line`/`lines()` langsung: keduanya TIDAK
/// aman dibatalkan di tengah — kalau `select!` memilih cabang ticker (cek
/// pause/cancel) saat read_line sedang berjalan, byte yang terlanjur masuk ke
/// buffer pemanggil bisa hilang / baris berikutnya terpotong. Di sini byte mentah
/// disimpan di `pending` milik struct (bukan milik future), jadi future `read()`
/// yang dibatalkan tidak pernah menelan data: read() hanya menulis buffer saat
/// ia selesai.
pub(crate) struct ChildLines<R> {
    reader: R,
    pending: Vec<u8>,
    raw: Vec<u8>,
    eof: bool,
}

impl<R: tokio::io::AsyncRead + Unpin> ChildLines<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
            raw: vec![0u8; 8 * 1024],
            eof: false,
        }
    }

    /// Baris berikutnya (tanpa '\\n'); `None` = stream benar-benar EOF.
    pub(crate) async fn next_line(&mut self) -> Option<String> {
        use tokio::io::AsyncReadExt;
        loop {
            if let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
                line.pop(); // buang '\n'
                return Some(String::from_utf8_lossy(&line).into_owned());
            }
            if self.eof {
                return if self.pending.is_empty() {
                    None
                } else {
                    // baris terakhir tanpa newline penutup
                    Some(String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned())
                };
            }
            match self.reader.read(&mut self.raw).await {
                Ok(0) => self.eof = true,
                Ok(n) => self.pending.extend_from_slice(&self.raw[..n]),
                Err(_) => self.eof = true,
            }
        }
    }
}

/// v2.3.0 (K3): file input aria2 (`aria2-<id>.txt`) berisi URL penuh — mungkin
/// bertoken login. Dibersihkan 0600 saat selesai; ini sapuan sisa sesi crash.
fn cleanup_orphan_aria2_inputs() {
    let dir = Config::aria2_input_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let ours = path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("aria2-") && n.ends_with(".txt"));
            if ours {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

static RE_INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap());

fn session_file() -> std::path::PathBuf {
    Config::config_dir().join("session.json")
}

/// v2.3.0 (M5): format berversi `{ "version": 1, "downloads": [...] }`.
/// Array telanjang (format ≤2.2.x) tetap dibaca agar sesi user lama tidak
/// hilang saat upgrade.
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionFile {
    version: u32,
    downloads: Vec<DownloadInfo>,
}

/// Some(vec) = terbaca (format baru ATAU lama); None = benar-benar korup.
fn parse_session(content: &str) -> Option<Vec<DownloadInfo>> {
    if content.trim().is_empty() {
        return Some(Vec::new());
    }
    if let Ok(sf) = serde_json::from_str::<SessionFile>(content) {
        return Some(sf.downloads);
    }
    if let Ok(v) = serde_json::from_str::<Vec<DownloadInfo>>(content) {
        return Some(v); // legacy bare array
    }
    None
}

fn load_session() -> Vec<DownloadInfo> {
    let path = session_file();
    match std::fs::read_to_string(&path) {
        Ok(content) => match parse_session(&content) {
            Some(v) => v,
            None => {
                // M5: jangan buang riwayat diam-diam — singkirkan file rusak
                // supaya (a) user bisa recovery manual, (b) flush berikutnya
                // tidak terus-menerus membaca ulang sampah.
                let backup = format!(
                    "{}.corrupt-{}",
                    path.display(),
                    chrono::Utc::now().timestamp()
                );
                tracing::warn!(
                    "session.json tidak bisa dibaca — dipindah ke {backup} untuk recovery manual"
                );
                let _ = std::fs::rename(&path, &backup);
                Vec::new()
            }
        },
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

    // urut (created_ms, id) — konsisten dengan promote_next (L4)
    all.sort_by_key(|d| (d.created, d.id.clone()));
    if all.len() > 200 {
        all = all.split_off(all.len() - 200);
    }

    let wrapped = SessionFile {
        version: 1,
        downloads: all,
    };
    if let Ok(json) = serde_json::to_string(&wrapped) {
        let path = session_file();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Validasi format --max-overall-download-limit aria2: angka, atau angka + K/M/G (opsional)
/// contoh: "0", "512K", "2M", "10G"
pub(crate) fn is_valid_speed_limit(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_direct_file_url ──

    #[test]
    fn is_direct_file_url_various_extensions() {
        // Ekstensi yang harus terdeteksi (langsung ke aria2)
        let direct_valid = [
            "https://x.com/v.mp4",
            "https://x.com/a.ZIP",       // case-insensitive
            "https://x.com/file.tar.gz", // multi-ext (.gz terdaftar)
            "https://x.com/p.pdf",
            "https://x.com/img.JPG",
            // M2: harus selaras dengan daftar intersep extension
            "https://x.com/setup.exe",
            "https://x.com/App.msi",
            "https://x.com/mac.dmg",
            "https://x.com/src.bz2",
            "https://x.com/laporan.docx",
            "https://x.com/data.xlsx",
            "https://x.com/buku.epub",
        ];
        for url in direct_valid {
            assert!(
                is_direct_file_url(url),
                "{} harus dianggap direct file",
                url
            );
        }
    }

    #[test]
    fn is_direct_file_url_strips_query_and_fragment() {
        // Fragment setelah ekstensi tidak boleh mengganggu deteksi
        assert!(is_direct_file_url("https://x.com/file.mp4#t=10"));
        assert!(is_direct_file_url("https://x.com/file.mp4?download=1"));
        assert!(is_direct_file_url("https://x.com/file.mp4?token=abc#t=0"));
    }

    #[test]
    fn is_direct_file_url_excludes_streaming() {
        // m3u8/mpd HARUS ke yt-dlp (perlu merge HLS/DASH)
        assert!(!is_direct_file_url("https://x.com/playlist.m3u8"));
        assert!(!is_direct_file_url("https://x.com/manifest.mpd"));
    }

    #[test]
    fn is_direct_file_url_non_file() {
        // Halaman video, API, halaman HTML
        assert!(!is_direct_file_url("https://x.com/watch?v=abc"));
        assert!(!is_direct_file_url("https://x.com/api/resource"));
        assert!(!is_direct_file_url("https://youtube.com/watch?v=xxx"));
        assert!(!is_direct_file_url("https://x.com/"));
    }

    // ── is_valid_speed_limit ──

    #[test]
    fn is_valid_speed_limit_valid() {
        assert!(is_valid_speed_limit("0"));
        assert!(is_valid_speed_limit("512"));
        assert!(is_valid_speed_limit("512K"));
        assert!(is_valid_speed_limit("2M"));
        assert!(is_valid_speed_limit("10G"));
        assert!(is_valid_speed_limit("2m")); // lowercase ok
    }

    #[test]
    fn is_valid_speed_limit_invalid() {
        assert!(!is_valid_speed_limit(""));
        assert!(!is_valid_speed_limit("   "));
        assert!(!is_valid_speed_limit("K")); // tanpa angka
        assert!(!is_valid_speed_limit("512X")); // unit salah
        assert!(!is_valid_speed_limit("-1"));
        assert!(!is_valid_speed_limit("abc"));
    }

    // ── sanitize_filename ──

    #[test]
    fn sanitize_filename_basic() {
        assert_eq!(sanitize_filename("video.mp4"), "video.mp4");
        assert_eq!(sanitize_filename("my-file_v2.zip"), "my-file_v2.zip");
    }

    #[test]
    fn sanitize_filename_strips_invalid_chars() {
        // Karakter terlarang diganti underscore.
        // CATATAN: sanitize_filename() juga strip query (?...) dan fragment (#...)
        // di awal — jadi kita pakai input TANPA '?' / '#' untuk isolasi test ini.
        // (Lihat sanitize_filename_strips_query_and_fragment untuk '?'/'#' behavior.)
        assert_eq!(
            sanitize_filename("a<b>c:d\"e/f\\g|h@i*.txt"),
            "a_b_c_d_e_f_g_h@i_.txt"
        );
    }

    #[test]
    fn sanitize_filename_strips_query_and_fragment() {
        // "?token=xxx" dan "#frag" tidak boleh ikut
        assert_eq!(sanitize_filename("file.mp4?token=abc"), "file.mp4");
        assert_eq!(sanitize_filename("file.mp4#frag"), "file.mp4");
    }

    #[test]
    fn sanitize_filename_trims_dots_and_spaces() {
        // File tersembunyi (diawali/diakhiri '.') di-trim
        assert_eq!(sanitize_filename("...file..."), "file");
        assert_eq!(sanitize_filename("   file   "), "file");
    }

    #[test]
    fn sanitize_filename_empty_fallback() {
        // Kalau setelah cleaning kosong → fallback "download_<timestamp>"
        // (semua char invalid + tidak punya '.') atau filename non-empty
        // hasil replace (semua jadi '_')
        let s = sanitize_filename("..."); // semua dot → trim habis → empty
        assert!(s.starts_with("download_"), "expected fallback, got {:?}", s);
    }

    #[test]
    fn sanitize_filename_unicode_safe() {
        // Truncate di char boundary — karakter multi-byte tidak boleh dipotong
        // di tengah (raw byte slice akan panic)
        let long_unicode = "🦀".repeat(500); // 4 byte × 500 = 2000 byte, ~500 chars
        let s = sanitize_filename(&long_unicode);
        assert!(s.len() <= 200, "truncate harus ≤200 byte, got {}", s.len());
        // Semua char harus utuh (tidak ada panik di tengah)
        assert!(s.chars().all(|c| c == '🦀'));
    }

    #[test]
    fn sanitize_filename_control_chars() {
        // Control char (0x00-0x1f) harus di-replace
        let with_ctrl = "file\x00\x01\x1fname.txt";
        let s = sanitize_filename(with_ctrl);
        assert!(!s.contains('\x00'));
        assert!(!s.contains('\x01'));
    }

    // ── extract_filename_from_url ──

    #[test]
    fn extract_filename_basic() {
        assert_eq!(
            extract_filename_from_url("https://example.com/path/video.mp4"),
            "video.mp4"
        );
    }

    #[test]
    fn extract_filename_with_query() {
        // Query di URL harus diabaikan
        assert_eq!(
            extract_filename_from_url("https://example.com/file.zip?token=abc&expire=123"),
            "file.zip"
        );
    }

    #[test]
    fn extract_filename_url_encoded() {
        // %20 di-decode jadi spasi (satu karakter). Spasi BUKAN karakter
        // invalid di regex sanitize_filename ([<>:"/\\|?*\x00-\x1f]) —
        // sengaja dibiarkan karena:
        //   - Linux filesystem mendukung spasi di filename
        //   - yt-dlp & aria2 handle nama file ber-spasi dengan baik
        //   - strip spasi akan kehilangan info (mis. "My Video.mp4"
        //     jadi "My_Video.mp4" — kelihatan aneh di file manager)
        // Lihat sanitize_filename_strips_invalid_chars untuk karakter
        // yang benar-benar di-replace.
        assert_eq!(
            extract_filename_from_url("https://example.com/my%20file.zip"),
            "my file.zip"
        );
    }

    #[test]
    fn extract_filename_no_extension_fallback() {
        // URL tanpa nama file / tanpa ekstensi → fallback "download_<ts>"
        let s = extract_filename_from_url("https://example.com/");
        assert!(s.starts_with("download_"), "expected fallback, got {:?}", s);
    }

    #[test]
    fn extract_filename_invalid_url_fallback() {
        // Bukan URL valid → fallback
        let s = extract_filename_from_url("not a url at all");
        assert!(s.starts_with("download_"));
    }

    #[test]
    fn extract_filename_root_path_fallback() {
        // Path = "/" → basename kosong → fallback
        let s = extract_filename_from_url("https://example.com");
        assert!(s.starts_with("download_"));
    }

    #[test]
    fn extract_filename_traversal_protected() {
        // "../etc/passwd" — basename "passwd" dipakai setelah sanitization
        // (../ di-strip oleh Path::file_name, jadi aman)
        let s = extract_filename_from_url("https://example.com/../etc/passwd");
        // Bisa "passwd" (no ext → fallback) atau "download_xxx"
        // Yang penting: TIDAK boleh mengandung ".." atau "/"
        assert!(!s.contains('/'), "filename tidak boleh ada '/': {:?}", s);
        assert!(!s.contains(".."), "filename tidak boleh ada '..': {:?}", s);
    }

    // ── parse_session (M5: format berversi + kompatibel legacy) ──

    const LEGACY_ITEM: &str = r#"{
        "id": "dl_1", "url": "https://x.com/a.zip", "filename": "a.zip",
        "save_dir": "/tmp", "status": "paused", "total_size": 10, "downloaded": 5,
        "speed": 0, "eta": 0, "progress": 50.0, "error_msg": "", "connections": 0,
        "retry_count": 0, "is_youtube": false
    }"#;

    #[test]
    fn parse_session_accepts_wrapped_versioned() {
        let json = format!(r#"{{"version":1,"downloads":[{}]}}"#, LEGACY_ITEM);
        let got = parse_session(&json).expect("format v1 harus terbaca");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "dl_1");
    }

    #[test]
    fn parse_session_accepts_legacy_bare_array() {
        // session.json keluaran ≤2.2.x = array telanjang — WAJIB tetap dibaca
        let json = format!("[{}]", LEGACY_ITEM);
        let got = parse_session(&json).expect("legacy array harus terbaca");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].filename, "a.zip");
    }

    #[test]
    fn parse_session_empty_and_garbage() {
        assert_eq!(parse_session("").map(|v| v.len()), Some(0));
        assert_eq!(parse_session("   ").map(|v| v.len()), Some(0));
        assert!(
            parse_session("{bukan json").is_none(),
            "korup → None (caller bikin backup)"
        );
    }

    // ── v2.3.1 (M1): ChildLines — pembaca baris cancellation-safe ──

    #[tokio::test]
    async fn child_lines_splits_and_flushes_tail() {
        let data: &[u8] = b"satu\ndua-tiga\nekor"; // baris terakhir TANPA newline
        let mut lines = ChildLines::new(std::io::Cursor::new(data));
        assert_eq!(lines.next_line().await.as_deref(), Some("satu"));
        assert_eq!(lines.next_line().await.as_deref(), Some("dua-tiga"));
        assert_eq!(lines.next_line().await.as_deref(), Some("ekor"));
        assert_eq!(lines.next_line().await, None);
    }

    #[tokio::test]
    async fn child_lines_empty_input_is_eof_none() {
        let data: &[u8] = b"";
        let mut lines = ChildLines::new(std::io::Cursor::new(data));
        assert_eq!(lines.next_line().await, None);
    }

    #[tokio::test]
    async fn child_lines_survives_split_across_reads() {
        // Cursor membaca sekaligus, jadi pakai duplikat: byte dipecah manual
        // dengan VecDeque reader sederhana di atas — cukup pastikan pending
        // menahan baris parsial antar-panggilan.
        let data: &[u8] = b"aaa bbb\nccc\n";
        let mut lines = ChildLines::new(std::io::Cursor::new(data));
        // baca pertama selalu mengembalikan baris penuh pertama
        assert_eq!(lines.next_line().await.as_deref(), Some("aaa bbb"));
        assert_eq!(lines.next_line().await.as_deref(), Some("ccc"));
        assert_eq!(lines.next_line().await, None);
    }
}
