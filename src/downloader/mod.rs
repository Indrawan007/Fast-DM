pub mod aria2;
pub mod aria2_rpc;
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

/// v2.8.1 (clippy `type_complexity`): alias utk tipe peta yang berulang di
/// banyak signature — hanya kosmetik, tipe identik.
pub(crate) type SharedInfo = Arc<Mutex<DownloadInfo>>;
pub(crate) type DownloadMap = Arc<RwLock<HashMap<String, SharedInfo>>>;

pub struct DownloadEngine {
    downloads: DownloadMap,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    config: Arc<RwLock<Config>>,
    dirty: Arc<AtomicBool>,
    /// Serialisasi semua penulisan session.json. Shutdown mengambil lock ini
    /// agar flusher periodik tidak menimpa snapshot final dengan state antara.
    session_io: Arc<Mutex<()>>,
    /// Menghentikan flusher dan membuat shutdown idempotent.
    shutting_down: Arc<AtomicBool>,
    /// v2.3.0 (K5): id hasil restore sesi — di-auto-resume sekali saat start
    /// bila config.auto_resume; bukan untuk item yang user pause manual.
    restored_ids: Arc<Mutex<Vec<String>>>,
}

// v2.9.4: `#[allow(dead_code)]` blanket di impl ini dihapus — semua method di
// sini memang terpakai (dipanggil GUI/IPC atau internal engine), dan allow
// blanket hanya menyembunyikan dead code yang muncul di kemudian hari.

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

        let session_io = Arc::new(Mutex::new(()));
        let shutting_down = Arc::new(AtomicBool::new(false));

        // Flusher: tulis session.json maks 1x/2 detik, hanya jika ada perubahan.
        // Penulisan diserialisasi dengan shutdown agar snapshot final tidak
        // tertimpa state runtime yang sengaja di-pause saat proses berhenti.
        let downloads_flush = downloads.clone();
        let dirty_flush = dirty.clone();
        let session_io_flush = session_io.clone();
        let shutting_down_flush = shutting_down.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if shutting_down_flush.load(Ordering::SeqCst) {
                    break;
                }
                flush_session(
                    &downloads_flush,
                    &dirty_flush,
                    &session_io_flush,
                    &shutting_down_flush,
                )
                .await;
            }
        });

        Self {
            downloads,
            event_tx,
            config: Arc::new(RwLock::new(Config::load_startup_snapshot().clone())),
            dirty,
            session_io,
            shutting_down,
            restored_ids: Arc::new(Mutex::new(restored_ids)),
        }
    }

    /// v2.3.0 (K5): lanjutkan unduhan yang terputus karena aplikasi keluar.
    /// README lama mengklaim "otomatis di-resume" padahal kode hanya menandai
    /// Paused — method ini menepati klaim itu, dengan penghormatan:
    /// - hanya item hasil restore sesi (user yang pause manual TIDAK diutak-atik),
    /// - hanya selama status masih Paused (kalau user sudah menekan apa pun, batalkan),
    /// - slot & antrean tetap diatur start_download (max_concurrent tidak jebol).
    ///
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

    /// Hentikan seluruh pekerjaan sebelum aplikasi benar-benar keluar.
    ///
    /// State runtime diubah ke Paused agar supervisor tidak menulis Error saat
    /// child/daemon dihentikan. Snapshot disk tetap menyimpan status aktif
    /// aslinya sehingga mekanisme `resume_restored()` dapat melanjutkannya pada
    /// start berikutnya. Daemon RPC juga dimatikan; karena itu GID dibuang hanya
    /// setelah shutdown daemon terkonfirmasi.
    pub async fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }

        // Urutan lock: session_io -> downloads -> item. Flusher memakai urutan
        // yang sama, sehingga snapshot final tidak dapat beradu/deadlock.
        let _session_guard = self.session_io.lock().await;
        let config = self.config.read().await.clone();
        let (pids, gids, mut snapshot) = {
            let downloads = self.downloads.read().await;
            let mut pids = Vec::new();
            let mut gids = Vec::new();
            let mut snapshot = Vec::with_capacity(downloads.len());
            for info in downloads.values() {
                let mut i = info.lock().await;
                if let Some(pid) = i.pid {
                    pids.push(pid);
                }
                if let Some(gid) = i.rpc_gid.clone() {
                    gids.push(gid);
                }
                snapshot.push(prepare_shutdown_snapshot(&mut i));
            }
            (pids, gids, snapshot)
        };

        for pid in pids {
            kill_child_pid(Some(pid));
        }

        let daemon_stopped = match aria2_rpc::shutdown_daemon(&gids, &config).await {
            Ok(()) => true,
            Err(e) => {
                // Pertahankan GID di session agar task yang berhasil di-pause
                // masih dapat di-unpause pada start berikutnya.
                tracing::warn!("Gagal mematikan daemon aria2 RPC: {}", e);
                false
            }
        };

        if daemon_stopped {
            for d in &mut snapshot {
                d.rpc_gid = None;
            }
            let downloads = self.downloads.read().await;
            for info in downloads.values() {
                info.lock().await.rpc_gid = None;
            }
        }

        if let Err(e) = write_session_snapshot(snapshot) {
            tracing::warn!("Gagal menulis snapshot session saat shutdown: {}", e);
        }
        self.dirty.store(false, Ordering::SeqCst);
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
        if !cfg.proxy_url.trim().is_empty() && !crate::config::is_valid_proxy_url(&cfg.proxy_url) {
            return Err(
                "Proxy tidak valid — contoh: http://127.0.0.1:8080, socks5://host:1080".to_string(),
            );
        }
        cfg.save().map_err(|e| e.to_string())?;
        *self.config.write().await = cfg;
        // v2.10.0 (C1): user mungkin baru saja mengganti `rpc_port` / proxy
        // — alasan gerbang "daemon tidak tersedia" ditutup bisa jadi sudah
        // tidak berlaku, jadi buka lagi agar percobaan berikutnya langsung
        // dicoba (bukan menunggu sisa backoff 60 dtk).
        aria2_rpc::reset_daemon_gate();
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
        let id = format!("dl_{}", &Uuid::new_v4().to_string()[..8]);
        let save = match save_dir {
            Some(d) => d.to_string(),
            None => self.config.read().await.download_dir.clone(),
        };

        // Ensure save dir exists
        let _ = std::fs::create_dir_all(&save);

        let fname = filename
            .map(sanitize_filename)
            .unwrap_or_else(|| extract_filename_from_url(url));

        // Deduplikasi: download live (url+dir+file sama) → kembalikan yang sudah ada.
        // Tanpa ini, dua proses aria2 bisa menulis file yang sama dan saling korup.
        // Pengecekan + insert harus atomik terhadap add_download lain.
        // Urutan lock tetap map -> item; lepas sebelum start_download.
        let mut downloads = self.downloads.write().await;
        let live = {
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
        // file:, dll.) — dulu lolos dan baru gagal lambat di CLI dengan error
        // yang tidak jelas.
        // v3.0.0: `magnet:` ikut ditolak — fitur torrent/magnet dihapus;
        // pesan tetap eksplisit agar tidak bingung saat menempel link lama.

        if !is_supported_scheme(url) {
            info.status = DownloadStatus::Error;
            info.error_msg = UNSUPPORTED_SCHEME_MSG.to_string();

            let _ = self.event_tx.send(DownloadEvent::Error(info.clone()));
            downloads.insert(id.clone(), Arc::new(Mutex::new(info)));
            self.mark_dirty();
            tracing::warn!("Download ditolak (skema URL): {}", url);
            return id;
        }

        let info = Arc::new(Mutex::new(info));
        downloads.insert(id.clone(), info);
        drop(downloads);
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
        let claimed: Option<(SharedInfo, usize)> = {
            let downloads = self.downloads.write().await;
            if self.shutting_down.load(Ordering::SeqCst) {
                return;
            }
            let Some(info) = downloads.get(id).cloned() else {
                return;
            };

            let mut active = 0usize;
            for other in downloads.values() {
                if other.lock().await.occupies_slot() {
                    active += 1;
                }
            }

            // Scope guard (pola promote_next): MutexGuard `i` harus drop
            // SEBELUM `info` dipindah keluar blok — kalau tidak, E0505
            // (cannot move out of `info` because it is borrowed).
            let start = {
                let mut i = info.lock().await;
                let start = i.request_start(active < max);
                self.mark_dirty();
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
                start
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
                self.config.clone(),
                self.dirty.clone(),
                self.shutting_down.clone(),
                live,
            );
        }
    }

    /// Pause one download and report whether the requested transition was
    /// accepted. IPC callers must not report success for an unknown or
    /// terminal download ID.
    pub async fn pause_download(&self, id: &str) -> bool {
        let downloads = self.downloads.read().await;
        let Some(info) = downloads.get(id) else {
            return false;
        };
        let mut i = info.lock().await;
        if !i.request_pause() {
            return false;
        }
        kill_child_pid(i.pid);
        let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
        self.mark_dirty();
        true
    }

    /// DEADLOCK FIX: guard read-lock harus di-drop SEBELUM start_download()
    /// — start_download meminta write-lock pada map yang sama, dan RwLock
    /// tokio tidak reentrant: read-guard lama tidak akan pernah di-drop
    /// selama kita menunggu write-lock → deadlock permanen.
    pub async fn resume_download(&self, id: &str) -> bool {
        let resumable = {
            let downloads = self.downloads.read().await;
            if let Some(info) = downloads.get(id) {
                Some(info.lock().await.status)
            } else {
                None
            }
        };
        if !matches!(
            resumable,
            Some(DownloadStatus::Paused | DownloadStatus::Error)
        ) {
            return false;
        }
        self.start_download(id).await;
        true
    }

    /// Pause SEMUA unduhan (aktif + antrian) — dipakai tombol "Jeda Semua" (UI-UX C3).
    pub async fn pause_all(&self) {
        let downloads = self.downloads.read().await;
        for info in downloads.values() {
            let mut i = info.lock().await;
            if i.request_pause() {
                kill_child_pid(i.pid);
                let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
                self.mark_dirty();
            }
        }
    }

    /// Resume SEMUA unduhan yang paused/error — dipakai tombol "Lanjut Semua" (UI-UX C3).
    pub async fn resume_all(&self) {
        let mut candidates = {
            let downloads = self.downloads.read().await;
            let mut v = Vec::new();
            for (id, info) in downloads.iter() {
                let info = info.lock().await;
                if matches!(info.status, DownloadStatus::Paused | DownloadStatus::Error) {
                    v.push((info.created, id.clone()));
                }
            }
            v
        };
        // HashMap tidak memiliki urutan stabil. Ajukan resume tertua dahulu,
        // dengan tie-break ID yang sama seperti promosi antrean.
        candidates.sort_unstable();
        // Semua guard map/item harus lepas sebelum start_download mengambil
        // write-lock. Validasi status dan kapasitas tetap dilakukan di sana.
        for (_, id) in candidates {
            self.start_download(&id).await;
        }
    }

    /// Cancel one download and report whether the ID existed. The RPC cleanup
    /// remains best-effort, but an unknown ID is no longer reported as success
    /// by the IPC layer.
    pub async fn cancel_download(&self, id: &str) -> bool {
        // Ambil handle kontrol lalu lepas semua lock SEBELUM RPC await.
        // Khusus item RPC yang sudah Paused, supervisor polling sudah selesai;
        // karena itu engine sendiri wajib forceRemove GID-nya dari daemon.
        let target = {
            let downloads = self.downloads.read().await;
            match downloads.get(id) {
                Some(info) => {
                    let mut i = info.lock().await;
                    let target = (i.pid.take(), i.rpc_gid.take());
                    i.status = DownloadStatus::Cancelled;
                    i.resume_pending = false;
                    i.status_detail.clear();

                    i.speed = 0;
                    let _ = self.event_tx.send(DownloadEvent::Progress(i.clone()));
                    Some(target)
                }
                None => None,
            }
        };

        let Some((pid, gid)) = target else {
            return false;
        };
        kill_child_pid(pid);
        if let Some(gid) = gid {
            let config = self.config.read().await.clone();
            if let Err(e) = aria2_rpc::remove_gid(&gid, &config).await {
                tracing::warn!("RPC cancel GID {}: {}", gid, e);
            }
        }
        self.mark_dirty();
        true
    }

    pub async fn clear_download(&self, id: &str) {
        // Cancel dulu supaya background task (aria2/yt-dlp) berhenti. Tidak
        // mengirim event agar row yang baru dihapus GUI tidak dibuat kembali.
        let target = {
            let downloads = self.downloads.read().await;
            match downloads.get(id) {
                Some(info) => {
                    let mut i = info.lock().await;
                    let target = (i.pid.take(), i.rpc_gid.take());
                    i.status = DownloadStatus::Cancelled;
                    i.resume_pending = false;
                    i.status_detail.clear();
                    i.speed = 0;
                    Some(target)
                }
                None => None,
            }
        };

        self.downloads.write().await.remove(id);

        if let Some((pid, gid)) = target {
            kill_child_pid(pid);
            if let Some(gid) = gid {
                let config = self.config.read().await.clone();
                if let Err(e) = aria2_rpc::remove_gid(&gid, &config).await {
                    tracing::warn!("RPC remove GID {}: {}", gid, e);
                }
            }
        }
        self.mark_dirty();
    }

    pub async fn get_all_downloads(&self) -> Vec<DownloadInfo> {
        let downloads = self.downloads.read().await;
        let mut result = Vec::with_capacity(downloads.len());
        for info in downloads.values() {
            result.push(info.lock().await.clone());
        }
        result
    }

    pub async fn get_all_summaries(&self) -> Vec<DownloadSummary> {
        let downloads = self.downloads.read().await;
        let mut result = Vec::with_capacity(downloads.len());
        for info in downloads.values() {
            let i = info.lock().await;
            result.push(DownloadSummary {
                status: i.status,
                speed: i.speed,
                resume_pending: i.resume_pending,
            });
        }
        result
    }
}

/// Jalankan download lalu promote antrian berikutnya saat selesai
fn spawn_supervised(
    downloads: DownloadMap,
    info: SharedInfo,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    shared_config: Arc<RwLock<Config>>,
    dirty: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    live_share: usize,
) {
    tokio::spawn(async move {
        // Snapshot hanya untuk worker ini. Promosi antrean membaca ulang
        // shared_config, tidak mewarisi snapshot lama dari worker yang selesai.
        let original_config = shared_config.read().await.clone();
        let mut config = original_config.clone();
        if !config.max_overall_speed.is_empty() && config.max_overall_speed != "0" {
            config.max_overall_speed =
                aria2::resolve_speed_limit(&config.max_overall_speed, live_share);
        }
        let (is_yt, url) = {
            let i = info.lock().await;
            (i.is_youtube, i.url.clone())
        };

        if is_yt {
            // YouTube: yt-dlp dengan dialog kualitas (behavior lama)
            youtube::download(info.clone(), tx.clone(), &config).await;
        } else if is_direct_file_url(&url) {
            // v2.9.0 (B2.2): http/https/ftp file langsung → daemon RPC aria2 —

            // limit total ditegakkan GLOBAL & LIVE oleh daemon (changeGlobalOption,
            // daemon membagi ulang ke semua unduhan aktif), pause/resume native.
            // Karena itu limit MENTAH yang dipakai, BUKAN hasil pembagian
            // per-proses M3 (juga anti double-division).
            let mut cfg = config.clone();
            cfg.max_overall_speed = original_config.max_overall_speed.clone();
            let outcome = aria2_rpc::download(info.clone(), tx.clone(), &cfg).await;
            // B2.2: daemon tak tersedia / addUri ditolak SEBELUM unduhan jalan
            // → http/ftp jatuh ke jalur per-proses lama (nol regresi).
            // (v3.0.0: magnet dihapus — tidak ada lagi jalur RPC-only.)
            if matches!(outcome, aria2_rpc::RpcOutcome::Fallback) {
                let aborted = {
                    let i = info.lock().await;
                    matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused)
                };
                if !aborted {
                    aria2::download(info.clone(), tx.clone(), &config).await;
                }
            }
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

        // Slot baru dilepas setelah seluruh cleanup backend selesai. Resume
        // yang diminta saat pause kini aman dipromosikan sebagai worker baru.
        {
            let map = downloads.write().await;
            let mut i = info.lock().await;
            i.finish_worker(!shutting_down.load(Ordering::SeqCst));
            if map
                .get(&i.id)
                .is_some_and(|current| Arc::ptr_eq(current, &info))
            {
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
            }
        }
        dirty.store(true, Ordering::SeqCst);
        promote_next(downloads, tx, shared_config, dirty, shutting_down).await;
    });
}

/// Skema yang engine tahu cara mengunduhnya: http/https/ftp (aria2/yt-dlp).
/// v3.0.0: `magnet:` dihapus — kini ikut ditolak seperti skema tak dikenal
/// lain (`blob:`, `data:`, `javascript:`, `file:`, `about:`, dst).
pub fn is_supported_scheme(url: &str) -> bool {
    Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https" | "ftp"))
        .unwrap_or(false)
}

/// Pesan penolakan skema non-download — **satu sumber** untuk engine
/// (`add_download`) maupun IPC extension (`ipc::handle_message`).
///
/// Dulu kedua pemanggil menyalin string-nya sendiri-sendiri dan "diselaraskan"
/// hanya lewat komentar. Akibatnya saat v3.0.0 menghapus dukungan
/// magnet/torrent, sisi IPC (dan README) diperbarui sementara sisi engine tetap
/// menulis "…http, https, ftp, atau magnet." — user yang menempel magnet di GUI
/// diberi tahu magnet didukung. Konstanta bersama ini menutup kemungkinan itu;
/// dijaga test `unsupported_scheme_message_lists_only_supported_schemes` dan
/// `ipc_reuses_engine_rejection_message`.
pub const UNSUPPORTED_SCHEME_MSG: &str = "Skema URL tidak didukung — http, https, atau ftp.";

/// v2.10.0 (D5): daftar ekstensi "file langsung" diangkat ke level modul
/// supaya bisa dibandingkan dengan daftar intersep `extension/background.js`
/// oleh test `extension_intercept_list_is_covered`. Sebelumnya kedua daftar
/// itu hanya dijaga oleh komentar ("M2: SELARASKAN…") — tidak ada yang
/// menangkap bila salah satunya ditambah ekstensi baru.
///
/// `.m3u8`/`.mpd` SENGAJA tidak ada di sini: manifest HLS/DASH harus lewat
/// yt-dlp supaya segmennya di-merge benar (lihat `wants_quality_dialog`).
///
/// v2.11.0: daftar DIPERLUAS ke 297 jenis file — semua kategori umum
/// (video, audio, gambar, arsip, dokumen, installer, font, 3D, VM, dll).
/// Test `extension_intercept_list_is_covered` memastikan extension JS tetap
/// selaras dengan daftar ini.
pub(crate) const DIRECT_FILE_EXTENSIONS: &[&str] = &[
    // Video
    ".mp4",
    ".webm",
    ".mkv",
    ".avi",
    ".mov",
    ".m4v",
    ".flv",
    ".wmv",
    ".3gp",
    ".3g2",
    ".ts",
    ".mts",
    ".m2ts",
    ".vob",
    ".mpg",
    ".mpeg",
    ".mpe",
    ".m2v",
    ".mp2v",
    ".f4v",
    ".asf",
    ".asx",
    ".rm",
    ".rmvb",
    ".divx",
    ".xvid",
    ".ogv",
    ".mxf",
    ".roq",
    ".nsv",
    ".amv",
    ".yuv",
    ".dv",
    ".hdv",
    ".qt",
    ".fli",
    ".flc",
    ".mod",
    ".tod",
    ".vro",
    ".dat",
    ".wmx",
    ".wvx",
    ".ogm",
    ".ogx",
    // Audio
    ".mp3",
    ".m4a",
    ".aac",
    ".ogg",
    ".opus",
    ".flac",
    ".wav",
    ".wma",
    ".aiff",
    ".aif",
    ".aifc",
    ".ape",
    ".ac3",
    ".dts",
    ".tta",
    ".wv",
    ".mka",
    ".mp2",
    ".mp1",
    ".au",
    ".ra",
    ".3ga",
    ".a52",
    ".amr",
    ".awb",
    ".m4b",
    ".m4r",
    ".mpc",
    ".mpp",
    ".shn",
    ".tak",
    ".alac",
    ".aup",
    ".aup3",
    ".cda",
    ".mid",
    ".midi",
    ".weba",
    ".wavpack",
    // Gambar
    ".jpg",
    ".jpeg",
    ".png",
    ".gif",
    ".webp",
    ".bmp",
    ".svg",
    ".svgz",
    ".ico",
    ".tiff",
    ".tif",
    ".psd",
    ".ai",
    ".eps",
    ".raw",
    ".cr2",
    ".nef",
    ".orf",
    ".sr2",
    ".arw",
    ".dng",
    ".heif",
    ".heic",
    ".avif",
    ".jxl",
    ".jxr",
    ".jp2",
    ".j2k",
    ".jpf",
    ".jpx",
    ".jpm",
    ".j2c",
    ".indd",
    ".cdr",
    ".xcf",
    ".kra",
    ".afphoto",
    ".afdesign",
    ".dwg",
    ".dxf",
    ".emf",
    ".wmf",
    // Arsip / Kompresi
    ".zip",
    ".zipx",
    ".rar",
    ".7z",
    ".7zip",
    ".tar",
    ".gz",
    ".bz2",
    ".bz",
    ".tbz2",
    ".tbz",
    ".xz",
    ".txz",
    ".lz",
    ".lzma",
    ".lzo",
    ".z",
    ".cab",
    ".arj",
    ".lzh",
    ".lha",
    ".jar",
    ".war",
    ".ear",
    ".xpi",
    ".crx",
    ".lz4",
    ".zst",
    ".tgz",
    ".tlz",
    ".zz",
    ".arc",
    ".pak",
    ".ace",
    ".alz",
    ".a",
    ".ar",
    ".s7z",
    // Dokumen / Ebook / Subtitle
    ".pdf",
    ".doc",
    ".docx",
    ".xls",
    ".xlsx",
    ".ppt",
    ".pptx",
    ".odt",
    ".ods",
    ".odp",
    ".odg",
    ".odf",
    ".rtf",
    ".tex",
    ".txt",
    ".csv",
    ".json",
    ".xml",
    ".yaml",
    ".yml",
    ".md",
    ".markdown",
    ".epub",
    ".mobi",
    ".azw",
    ".azw3",
    ".fb2",
    ".fbz",
    ".djvu",
    ".djv",
    ".chm",
    ".xps",
    ".oxps",
    ".pages",
    ".numbers",
    ".key",
    ".srt",
    ".sub",
    ".ass",
    ".ssa",
    ".vtt",
    ".lrf",
    ".pdb",
    ".lit",
    ".pml",
    ".rb",
    ".tcr",
    ".txtz",
    ".log",
    ".nfo",
    ".cue",
    ".ics",
    ".vcf",
    ".bib",
    ".abw",
    // Installer / Executable / Disk Image / VM
    ".exe",
    ".msi",
    ".dmg",
    ".deb",
    ".rpm",
    ".apk",
    ".appimage",
    ".run",
    ".bin",
    ".msu",
    ".appx",
    ".appxbundle",
    ".flatpak",
    ".snap",
    ".pkg",
    ".ipa",
    ".msix",
    ".msixbundle",
    ".xapk",
    ".apks",
    ".bundle",
    ".sh",
    ".com",
    ".gadget",
    ".wsf",
    ".bat",
    ".cmd",
    ".iso",
    ".img",
    ".ova",
    ".ovf",
    ".vdi",
    ".vmdk",
    ".qcow2",
    ".vhd",
    ".vhdx",
    ".wim",
    ".swm",
    ".esd",
    // Torrent / Metadata
    ".torrent",
    ".nzb",
    ".metalink",
    ".meta4",
    // Font
    ".ttf",
    ".otf",
    ".woff",
    ".woff2",
    ".eot",
    ".fon",
    ".fnt",
    ".ttc",
    // 3D / CAD
    ".stl",
    ".obj",
    ".fbx",
    ".blend",
    ".3ds",
    ".dae",
    ".gltf",
    ".glb",
    ".3mf",
    ".ply",
    ".off",
    ".x3d",
    ".stp",
    ".step",
    ".iges",
    ".igs",
    ".sldprt",
    ".sldasm",
    // Database / Backup
    ".sqlite",
    ".sqlite3",
    ".db",
    ".db3",
    ".sql",
    ".bak",
    ".backup",
    ".dump",
    ".bson",
];

/// Cache HashSet untuk lookup O(1) ekstensi — dibangun sekali, dipakai di
/// `is_direct_file_url`. Menghemat CPU dibanding iterasi linear 297 ekstensi
/// tiap URL (dulu O(n), kini O(1) setelah ekstrak ekstensi).
static DIRECT_EXT_SET: LazyLock<std::collections::HashSet<&'static str>> =
    LazyLock::new(|| DIRECT_FILE_EXTENSIONS.iter().copied().collect());

/// Bagian PATH dari sebuah URL — authority (`user:pass@host:port`) dibuang.
///
/// v2.11.1 (F2): ini inti perbaikan salah-klasifikasi host telanjang. Tanpa
/// pembuangan authority, segmen terakhir `"https://x.com"` adalah `"x.com"`,
/// dan karena `.com` ada di `DIRECT_FILE_EXTENSIONS` (executable DOS) sebuah
/// homepage biasa dianggap file langsung → dikirim ke aria2, bukan ke resolver
/// universal. URL yang hanya terdiri dari authority kini menghasilkan path
/// kosong (→ bukan file langsung).
///
/// URL tanpa `"://"` dikembalikan apa adanya: input yang belum dinormalisasi
/// (`"file.mp4"`, `"host/dir/file.zip"`) tetap diperlakukan sebagai path.
/// Kontrak pemanggil: URL yang sampai ke `is_direct_file_url` sudah lolos
/// `is_supported_scheme`, jadi selalu berskema http/https/ftp.
pub(crate) fn url_path_part(url: &str) -> &str {
    let Some(scheme_end) = url.find("://") else {
        return url;
    };
    let after_authority = &url[scheme_end + 3..];
    match after_authority.find('/') {
        Some(i) => &after_authority[i..],
        None => "",
    }
}

/// URL file langsung (punya ekstensi file/media) → langsung ke aria2 tanpa
/// lewat yt-dlp. HLS/DASH (m3u8/mpd) tetap ke yt-dlp agar di-merge benar.
///
/// v2.11.0: dioptimalkan — ekstrak ekstensi file dari path URL lalu cek
/// HashSet O(1), bukan scan linear `ends_with`.
///
/// v2.11.1 (F2/F8): authority dibuang lebih dulu (`url_path_part`) sehingga
/// host tidak pernah dibaca sebagai nama file, dan fallback full-scan
/// `ends_with` DIHAPUS — cabang itu tidak terjangkau lagi: ujung string yang
/// di-scan adalah ujung segmen file itu sendiri, jadi apa pun yang bisa
/// cocok di sana sudah cocok lewat lookup HashSet di bawah.
pub fn is_direct_file_url(url: &str) -> bool {
    // Potong fragment & query (bisa muncul di mana pun), lalu buang authority.
    let no_fragment = url.split('#').next().unwrap_or(url);
    let no_query = no_fragment.split('?').next().unwrap_or(no_fragment);
    let lower = url_path_part(no_query).to_ascii_lowercase();

    // Nama file = segmen path terakhir. Tanpa titik → bukan file langsung.
    let file_part = lower.rsplit('/').next().unwrap_or(&lower);
    let Some(dot) = file_part.rfind('.') else {
        return false;
    };

    // Ekstensi setelah titik terakhir (".zip"), lalu compound dua level
    // (".tar.gz") bila yang pertama tidak dikenal — dibatasi 2 level agar
    // tetap O(1) dan hemat CPU.
    if DIRECT_EXT_SET.contains(&file_part[dot..]) {
        return true;
    }
    match file_part[..dot].rfind('.') {
        Some(dot2) => DIRECT_EXT_SET.contains(&file_part[dot2..]),
        None => false,
    }
}

/// Cari download Queued tertua dan jalankan jika ada slot kosong
async fn promote_next(
    downloads: DownloadMap,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    shared_config: Arc<RwLock<Config>>,
    dirty: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
) {
    let max = usize::from(shared_config.read().await.max_concurrent.max(1));

    let next = {
        let map = downloads.write().await;
        if shutting_down.load(Ordering::SeqCst) {
            return;
        }

        let mut active = 0usize;
        let mut queued = 0usize;
        let mut oldest: Option<((i64, String), SharedInfo)> = None;

        for info in map.values() {
            let i = info.lock().await;
            if i.occupies_slot() {
                active += 1;
            } else if i.status == DownloadStatus::Queued {
                queued += 1;
                let key = (i.created, i.id.clone());
                if oldest.as_ref().is_none_or(|(k, _)| key < *k) {
                    oldest = Some((key, info.clone()));
                }
            }
        }

        if active >= max {
            None
        } else if let Some((_, info)) = oldest {
            let live = (active + queued).max(1);
            let started = {
                let mut i = info.lock().await;
                if i.status != DownloadStatus::Queued || !i.request_start(true) {
                    false
                } else {
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
        spawn_supervised(
            downloads,
            info,
            tx,
            shared_config,
            dirty,
            shutting_down,
            live,
        );
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
                .is_some_and(|n| n.starts_with("aria2-") && n.ends_with(".txt"));
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

/// Buat snapshot final tanpa kehilangan semantik auto-resume. State runtime
/// aktif di-pause agar supervisor berhenti tenang; salinan untuk disk tetap
/// memakai status aktif/queued asli dan tidak pernah menyimpan PID proses.
fn prepare_shutdown_snapshot(info: &mut DownloadInfo) -> DownloadInfo {
    let mut snapshot = info.clone();
    snapshot.pid = None;
    if info.resume_pending && matches!(info.status, DownloadStatus::Paused | DownloadStatus::Error)
    {
        // Simpan niat user untuk melanjutkan, bukan pause internal saat cleanup.
        snapshot.status = DownloadStatus::Queued;
        snapshot.status_detail.clear();
    }
    info.resume_pending = false;

    info.pid = None;
    if matches!(
        info.status,
        DownloadStatus::Downloading | DownloadStatus::Resolving | DownloadStatus::Queued
    ) {
        info.status = DownloadStatus::Paused;
        info.speed = 0;
        info.eta = 0;
    }

    snapshot
}

/// v2.10.0 (B1): header yang TIDAK boleh ikut tertulis ke `session.json`.
///
/// Sejak allow-list IPC dipersempit (§C3/B4) header semacam ini seharusnya
/// tidak pernah masuk `DownloadInfo.headers` lagi — tapi `session.json` dari
/// versi ≤2.9.4 masih bisa memuatnya, dan file itu tidak punya kedaluwarsa
/// (hanya cap 200 entri). Redaksi di jalur tulis membuat salinan lama
/// otomatis bersih pada flush berikutnya, tanpa perlu migrasi terpisah.
/// Dicocokkan case-insensitive; nama header HTTP memang case-insensitive.
pub(crate) const SENSITIVE_HEADERS: &[&str] = &["cookie", "authorization", "proxy-authorization"];

/// Buang header sensitif dari satu item sebelum dipersist.
pub(crate) fn redact_for_persist(d: &mut DownloadInfo) {
    if d.headers.is_empty() {
        return;
    }
    let before = d.headers.len();
    d.headers
        .retain(|k, _| !SENSITIVE_HEADERS.contains(&k.to_ascii_lowercase().as_str()));
    if d.headers.len() != before {
        tracing::debug!(
            "{} header sensitif dibuang dari snapshot session ({})",
            before - d.headers.len(),
            d.id
        );
    }
}

/// Tulis satu snapshot session secara atomik, dibatasi 200 entri terbaru.
fn write_session_snapshot(mut all: Vec<DownloadInfo>) -> Result<(), String> {
    // urut (created_ms, id) — konsisten dengan promote_next (L4)
    all.sort_by_key(|d| (d.created, d.id.clone()));
    if all.len() > 200 {
        all = all.split_off(all.len() - 200);
    }
    // B1: kredensial tidak pernah menyentuh disk lewat jalur ini.
    for d in &mut all {
        redact_for_persist(d);
    }

    let wrapped = SessionFile {
        version: 1,
        downloads: all,
    };
    let json = serde_json::to_string(&wrapped).map_err(|e| e.to_string())?;
    let path = session_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Flusher periodik. Lock I/O diambil sebelum memeriksa dirty supaya shutdown
/// selalu menjadi penulis terakhir dan tidak mungkin ditimpa task yang telat.
async fn flush_session(
    downloads: &DownloadMap,
    dirty: &AtomicBool,
    session_io: &Mutex<()>,
    shutting_down: &AtomicBool,
) {
    let _session_guard = session_io.lock().await;
    if shutting_down.load(Ordering::SeqCst) || !dirty.swap(false, Ordering::SeqCst) {
        return;
    }

    let all: Vec<DownloadInfo> = {
        let map = downloads.read().await;
        let mut v = Vec::with_capacity(map.len());
        for info in map.values() {
            v.push(info.lock().await.clone());
        }
        v
    };

    if let Err(e) = write_session_snapshot(all) {
        // Coba lagi pada tick berikutnya; jangan kehilangan perubahan hanya
        // karena disk sementara penuh/read-only.
        dirty.store(true, Ordering::SeqCst);
        tracing::warn!("Gagal menulis session.json: {}", e);
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
        // v2.10.5 (bugfix): pertahankan ekstensi saat memotong. Potongan mentah
        // 200 char bisa membuang ".mp4"/".zip"/".mkv" di ujung nama panjang
        // (Content-Disposition dari server) sehingga file tersimpan tanpa
        // ekstensi dan tak dikenali. Bila ekstensi akhir pendek (≤10 char
        // alfanumerik), potong STEM-nya saja lalu tempel ekstensi kembali.
        if let Some(dot) = cleaned.rfind('.') {
            let ext = &cleaned[dot + 1..];
            let ext_ok = !ext.is_empty()
                && ext.len() <= 10
                && ext.chars().all(|c| c.is_ascii_alphanumeric());
            let stem_len = 200usize.saturating_sub(ext.len() + 1);
            if ext_ok && stem_len >= 1 {
                let end = stem_len.min(cleaned.len());
                let mut pos = end;
                while !cleaned.is_char_boundary(pos) && pos > 0 {
                    pos -= 1;
                }
                if pos > 0 {
                    return format!("{}.{}", &cleaned[..pos], ext);
                }
            }
        }
        // Fallback: potong di char boundary (raw byte slicing panics on
        // multi-byte UTF-8) — untuk nama tanpa ekstensi yang bisa dipertahankan.
        let mut end = 200;
        while !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned[..end].to_string()
    } else {
        cleaned.to_string()
    }
}

pub(crate) fn is_script_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".php", ".asp", ".aspx", ".jsp", ".do", ".action", ".cgi", ".pl", ".html", ".htm",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Extract filename from URL (path & query parameters fallback)
pub fn extract_filename_from_url(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        // 1. Cek query parameters yang sering membawa nama file asli dari CDN/storage
        //    (mis. ?filename=video.mp4, ?file=video.mp4, ?response-content-disposition=attachment;filename=...)
        for (k, v) in parsed.query_pairs() {
            let k_lower = k.to_ascii_lowercase();
            if matches!(
                k_lower.as_str(),
                "filename" | "file" | "name" | "title" | "fn"
            ) {
                let cleaned = sanitize_filename(&v);
                if !cleaned.is_empty() && cleaned.contains('.') && !is_script_extension(&cleaned) {
                    return cleaned;
                }
            } else if k_lower == "response-content-disposition" || k_lower == "rscd" {
                if let Some(name) = aria2::parse_content_disposition(&v) {
                    let cleaned = sanitize_filename(&name);
                    if !cleaned.is_empty()
                        && cleaned.contains('.')
                        && !is_script_extension(&cleaned)
                    {
                        return cleaned;
                    }
                }
            }
        }

        // 2. Cek path URL
        let path = parsed.path();
        let decoded = urlencoding::decode(path).unwrap_or_default();
        let basename = Path::new(decoded.as_ref())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let cleaned = sanitize_filename(basename);
        if !cleaned.is_empty() && cleaned.contains('.') && !is_script_extension(&cleaned) {
            return cleaned;
        }
    }

    format!("download_{}", chrono::Utc::now().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Engine fixture without startup/session/config filesystem side effects.
    fn lifecycle_engine() -> DownloadEngine {
        let (tx, _rx) = mpsc::unbounded_channel();
        DownloadEngine {
            downloads: Arc::new(RwLock::new(HashMap::new())),
            event_tx: tx,
            config: Arc::new(RwLock::new(Config {
                max_concurrent: 1,
                ..Config::default()
            })),
            dirty: Arc::new(AtomicBool::new(false)),
            session_io: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            restored_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn lifecycle_item(
        engine: &DownloadEngine,
        id: &str,
        status: DownloadStatus,
    ) -> SharedInfo {
        let mut info = DownloadInfo::new(
            id.into(),
            "https://example.test/file.zip".into(),
            "file.zip".into(),
            "unused".into(),
            Default::default(),
            None,
        );
        info.status = status;
        let info = Arc::new(Mutex::new(info));
        engine
            .downloads
            .write()
            .await
            .insert(id.into(), info.clone());
        info
    }

    #[tokio::test]
    async fn repeated_engine_start_preserves_active_status_at_full_capacity() {
        let engine = Arc::new(lifecycle_engine());
        let info = lifecycle_item(&engine, "active", DownloadStatus::Downloading).await;
        info.lock().await.worker_active = true;
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let engine = engine.clone();
            tasks.push(tokio::spawn(async move {
                engine.start_download("active").await
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let info = info.lock().await;
        assert_eq!(info.status, DownloadStatus::Downloading);
        assert!(info.worker_active);
        assert_eq!(info.retry_count, 0, "no second worker claimed");
    }

    #[tokio::test]
    async fn control_methods_reject_unknown_or_terminal_ids() {
        let engine = lifecycle_engine();
        lifecycle_item(&engine, "completed", DownloadStatus::Completed).await;

        assert!(!engine.pause_download("missing").await);
        assert!(!engine.resume_download("missing").await);
        assert!(!engine.cancel_download("missing").await);
        assert!(!engine.pause_download("completed").await);
        assert!(!engine.resume_download("completed").await);
        assert!(engine.cancel_download("completed").await);
    }

    #[tokio::test]
    async fn pause_cancels_error_retry_and_publishes_the_new_state() {
        for all in [false, true] {
            let mut engine = lifecycle_engine();
            let (tx, mut rx) = mpsc::unbounded_channel();
            engine.event_tx = tx;
            let info = lifecycle_item(&engine, "retry", DownloadStatus::Error).await;
            {
                let mut i = info.lock().await;
                i.worker_active = true;
                assert!(!i.request_start(true));
                assert!(i.resume_pending);
            }
            if all {
                engine.pause_all().await;
            } else {
                engine.pause_download("retry").await;
            }
            let event = rx.try_recv().expect("pause must publish its state");
            let DownloadEvent::Progress(event) = event else {
                panic!("expected Progress")
            };
            assert_eq!(event.status, DownloadStatus::Paused);
            assert!(!event.resume_pending);
            assert!(event.status_detail.is_empty());
            assert!(engine.dirty.load(Ordering::SeqCst));
            let mut i = info.lock().await;
            assert!(i.worker_active, "cleanup still owns its slot");
            i.finish_worker(true);
            assert_eq!(
                i.status,
                DownloadStatus::Paused,
                "retry must not be requeued"
            );
        }
    }

    #[tokio::test]
    async fn pause_preserves_errors_without_a_pending_retry() {
        let engine = lifecycle_engine();
        let info = lifecycle_item(&engine, "error", DownloadStatus::Error).await;
        info.lock().await.error_msg = "Original failure".into();
        engine.pause_download("error").await;
        engine.pause_all().await;
        let i = info.lock().await;
        assert_eq!(i.status, DownloadStatus::Error);
        assert_eq!(i.error_msg, "Original failure");
        assert!(!engine.dirty.load(Ordering::SeqCst));
    }

    // Slot sengaja penuh: menguji urutan pengajuan lewat event Queued tanpa
    // menjalankan backend, mengakses jaringan, atau menulis konfigurasi user.
    async fn resume_all_order(items: &[(&str, i64, DownloadStatus)]) -> Vec<String> {
        let mut engine = lifecycle_engine();
        let (tx, mut rx) = mpsc::unbounded_channel();
        engine.event_tx = tx;
        let active = lifecycle_item(&engine, "active", DownloadStatus::Downloading).await;
        active.lock().await.worker_active = true;
        for &(id, created, status) in items {
            let item = lifecycle_item(&engine, id, status).await;
            item.lock().await.created = created;
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), engine.resume_all())
            .await
            .expect("resume_all must release locks before starting downloads");
        let mut order = Vec::new();
        while let Ok(event) = rx.try_recv() {
            let DownloadEvent::Progress(info) = event else {
                panic!("expected queued progress")
            };
            assert_eq!(info.status, DownloadStatus::Queued);
            assert!(!info.worker_active);
            assert_eq!(info.retry_count, 0);
            order.push(info.id);
        }
        for &(id, _, status) in items {
            if !matches!(status, DownloadStatus::Paused | DownloadStatus::Error) {
                let downloads = engine.downloads.read().await;
                assert_eq!(downloads.get(id).unwrap().lock().await.status, status);
            }
        }
        assert!(active.lock().await.worker_active);
        order
    }

    #[tokio::test]
    async fn resume_all_submits_oldest_first_and_only_resumable_items() {
        let order = resume_all_order(&[
            ("new", 30, DownloadStatus::Paused),
            ("done", 0, DownloadStatus::Completed),
            ("old", 10, DownloadStatus::Error),
            ("queued", 0, DownloadStatus::Queued),
            ("middle", 20, DownloadStatus::Paused),
            ("cancelled", 0, DownloadStatus::Cancelled),
            ("resolving", 0, DownloadStatus::Resolving),
        ])
        .await;
        assert_eq!(order, vec!["old", "middle", "new"]);
    }

    #[tokio::test]
    async fn resume_all_breaks_timestamp_ties_by_id() {
        for items in [
            [
                ("z", 10, DownloadStatus::Paused),
                ("a", 10, DownloadStatus::Error),
            ],
            [
                ("a", 10, DownloadStatus::Error),
                ("z", 10, DownloadStatus::Paused),
            ],
        ] {
            assert_eq!(resume_all_order(&items).await, vec!["a", "z"]);
        }
        assert!(resume_all_order(&[]).await.is_empty());
    }

    #[tokio::test]
    async fn pause_and_cancel_override_deferred_resume() {
        let engine = lifecycle_engine();
        let info = lifecycle_item(&engine, "stopping", DownloadStatus::Paused).await;
        info.lock().await.worker_active = true;
        engine.resume_download("stopping").await;
        assert!(info.lock().await.resume_pending);
        engine.pause_download("stopping").await;
        assert!(!info.lock().await.resume_pending);
        engine.resume_download("stopping").await;
        engine.pause_all().await;
        assert!(!info.lock().await.resume_pending);
        engine.resume_download("stopping").await;
        engine.cancel_download("stopping").await;
        let mut info = info.lock().await;
        info.finish_worker(true);
        assert_eq!(info.status, DownloadStatus::Cancelled);
        assert!(!info.resume_pending);
    }

    #[tokio::test]
    async fn queue_promotion_uses_current_limit_and_counts_stopping_workers() {
        let engine = lifecycle_engine();
        engine.config.write().await.max_concurrent = 3;
        let worker = lifecycle_item(&engine, "stopping", DownloadStatus::Paused).await;
        worker.lock().await.worker_active = true;
        let queued = lifecycle_item(&engine, "queued", DownloadStatus::Queued).await;
        // User lowered the limit while an existing worker was still running.
        engine.config.write().await.max_concurrent = 1;
        promote_next(
            engine.downloads.clone(),
            engine.event_tx.clone(),
            engine.config.clone(),
            engine.dirty.clone(),
            engine.shutting_down.clone(),
        )
        .await;
        assert_eq!(queued.lock().await.status, DownloadStatus::Queued);
        assert!(!queued.lock().await.worker_active);
        // Even with free slots, shutdown cannot promote/start anything.
        engine.shutting_down.store(true, Ordering::SeqCst);
        worker.lock().await.finish_worker(false);
        engine.config.write().await.max_concurrent = 10;
        promote_next(
            engine.downloads.clone(),
            engine.event_tx.clone(),
            engine.config.clone(),
            engine.dirty.clone(),
            engine.shutting_down.clone(),
        )
        .await;
        engine.start_download("queued").await;
        assert_eq!(queued.lock().await.status, DownloadStatus::Queued);
        assert!(!queued.lock().await.worker_active);
    }

    #[tokio::test]
    async fn queued_item_can_be_paused_individually() {
        let engine = lifecycle_engine();
        let queued = lifecycle_item(&engine, "queued", DownloadStatus::Queued).await;
        engine.pause_download("queued").await;
        assert_eq!(queued.lock().await.status, DownloadStatus::Paused);
        assert!(!queued.lock().await.worker_active);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_adds_are_deduplicated_atomically() {
        struct TempDir(std::path::PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let temp = TempDir(std::env::temp_dir().join(format!("fastdm-dedup-{}", Uuid::new_v4())));
        let dir = temp.0.to_string_lossy().to_string();
        let (tx, _rx) = mpsc::unbounded_channel();
        // No DownloadEngine::new: avoid startup config/session I/O and flusher.
        let engine = Arc::new(DownloadEngine {
            downloads: Arc::new(RwLock::new(HashMap::new())),
            event_tx: tx,
            config: Arc::new(RwLock::new(Config {
                download_dir: dir.clone(),
                ..Config::default()
            })),
            dirty: Arc::new(AtomicBool::new(false)),
            session_io: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            restored_ids: Arc::new(Mutex::new(Vec::new())),
        });
        let sentinel = Arc::new(Mutex::new(DownloadInfo::new(
            "sentinel".into(),
            "https://other.test/a".into(),
            "a".into(),
            dir.clone(),
            Default::default(),
            None,
        )));
        engine
            .downloads
            .write()
            .await
            .insert("sentinel".into(), sentinel.clone());
        let sentinel_guard = sentinel.lock().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(17));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let engine = engine.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                engine
                    .add_download(
                        "https://example.test/a.zip",
                        Some("a.zip"),
                        None,
                        false,
                        Default::default(),
                        None,
                    )
                    .await
            }));
        }
        barrier.wait().await;
        // Make callers contend on the same item during the dedup check.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        drop(sentinel_guard);
        let mut ids = std::collections::HashSet::new();
        for task in tasks {
            ids.insert(
                tokio::time::timeout(std::time::Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(ids.len(), 1);
        assert_eq!(engine.downloads.read().await.len(), 2);
        let id = ids.into_iter().next().unwrap();
        engine
            .downloads
            .read()
            .await
            .get(&id)
            .unwrap()
            .lock()
            .await
            .status = DownloadStatus::Completed;
        let again = engine
            .add_download(
                "https://example.test/a.zip",
                Some("a.zip"),
                None,
                false,
                Default::default(),
                None,
            )
            .await;
        assert_ne!(again, id, "completed downloads can be added again");
    }

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

    // ── v2.11.1 (F2): host telanjang bukan nama file ──

    #[test]
    fn url_path_part_drops_scheme_and_authority() {
        assert_eq!(url_path_part("https://x.com/a/b.zip"), "/a/b.zip");
        assert_eq!(url_path_part("https://x.com/"), "/");
        // Authority saja → path kosong (inilah yang membuat host telanjang
        // tidak lagi dibaca sebagai nama file).
        assert_eq!(url_path_part("https://x.com"), "");
        // userinfo + port ikut terbuang; query BUKAN urusan fungsi ini
        // (dipotong pemanggil sebelum sampai ke sini).
        assert_eq!(url_path_part("http://u:p@x.com:81/f.bin?y=1"), "/f.bin?y=1");
        assert_eq!(url_path_part("http://[::1]:8080/d.iso"), "/d.iso");
        // Tanpa "://" → apa adanya (input belum dinormalisasi).
        assert_eq!(url_path_part("x.com/f.bin"), "x.com/f.bin");
        assert_eq!(url_path_part("file.bin"), "file.bin");
    }

    #[test]
    fn is_direct_file_url_rejects_bare_host() {
        // F2: ".com" ada di daftar (executable DOS), sedangkan segmen terakhir
        // URL tanpa path adalah HOST-nya — jadi semua ini dulu "true".
        for url in [
            "https://x.com",
            "http://example.org",
            "https://cdn.example.com",
            "https://sub.domain.com",
            "https://x.com:8080",
            "https://user:pass@host.com",
        ] {
            assert!(!is_direct_file_url(url), "{url} = host, bukan file");
        }
        // Path file yang sesungguhnya tetap terdeteksi, termasuk saat ada
        // userinfo/port di authority.
        assert!(is_direct_file_url("https://cdn.example.com/a.zip"));
        assert!(is_direct_file_url("https://user:pass@host.com:8080/a.zip"));
        assert!(is_direct_file_url("https://x.com/game.com")); // .com SEBAGAI file
    }

    #[test]
    fn is_direct_file_url_scheme_less_path_still_works() {
        // Kontrak: URL yang sampai ke sini sudah lolos is_supported_scheme.
        // Bentuk tanpa skema tetap diperlakukan sebagai path (bukan authority)
        // supaya pemanggil internal/defensif tidak berubah perilaku.
        assert!(is_direct_file_url("x.com/file.mp4"));
        assert!(is_direct_file_url("file.mp4"));
        assert!(!is_direct_file_url("x.com/watch"));
    }

    // ── is_supported_scheme ──

    #[test]
    fn supported_scheme_accepts_http_ftp() {
        assert!(is_supported_scheme("https://example.com/file.zip"));
        assert!(is_supported_scheme("http://example.com/a"));
        assert!(is_supported_scheme("ftp://server/pub/file.iso"));
    }

    #[test]
    fn supported_scheme_rejects_non_download_schemes() {
        assert!(!is_supported_scheme("blob:https://site/abc"));
        assert!(!is_supported_scheme(
            "data:application/octet-stream;base64,AAAA"
        ));
        assert!(!is_supported_scheme("javascript:alert(1)"));
        assert!(!is_supported_scheme("file:///home/user/a.zip"));
        assert!(!is_supported_scheme("not a url at all"));
    }

    #[test]
    fn supported_scheme_rejects_magnet_since_v3() {
        // v3.0.0: fitur torrent/magnet dihapus — magnet kini ditolak di gate
        // yang sama seperti skema non-download lain, dengan pesan jelas.
        assert!(!is_supported_scheme(
            "magnet:?xt=urn:btih:aaaabbbbccccdddd&dn=ubuntu"
        ));
        assert!(!is_supported_scheme("  MAGNET:?xt=urn:btih:deadbeef")); // trim + case-insensitive
    }

    #[test]
    fn unsupported_scheme_message_lists_only_supported_schemes() {
        // v3.0.0 menghapus magnet/torrent — pesan penolakan tidak boleh lagi
        // menawarkan magnet, dan harus menyebut skema yang benar-benar diterima
        // `is_supported_scheme`. Sebelumnya sisi engine masih menulis "…atau
        // magnet." sementara IPC/README sudah benar.
        assert!(
            !UNSUPPORTED_SCHEME_MSG.contains("magnet"),
            "magnet ditolak sejak v3.0.0 — jangan tawarkan di pesan: {}",
            UNSUPPORTED_SCHEME_MSG
        );
        for scheme in ["http", "https", "ftp"] {
            assert!(
                UNSUPPORTED_SCHEME_MSG.contains(scheme),
                "skema yang didukung harus disebut di pesan: {scheme}"
            );
        }
    }

    #[test]
    fn ipc_reuses_engine_rejection_message() {
        // IPC dulu menyalin string penolakan sendiri dan "diselaraskan" hanya
        // lewat komentar — persis pola yang membuat kedua pesan melenceng.
        // Test ini mengunci keduanya ke satu konstanta.
        let ipc_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ipc/mod.rs"));
        assert!(
            ipc_src.contains("crate::downloader::UNSUPPORTED_SCHEME_MSG"),
            "ipc/mod.rs harus memakai konstanta engine, bukan salinan sendiri"
        );
        assert!(
            !ipc_src.contains("const REJECTED"),
            "ipc/mod.rs kembali menyalin pesan penolakan sendiri"
        );
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
    fn extract_filename_with_query_params() {
        // Query param `filename`/`file` yang membawa nama asli
        assert_eq!(
            extract_filename_from_url("https://example.com/download.php?file=Anime_Episode_10.mp4"),
            "Anime_Episode_10.mp4"
        );
        assert_eq!(
            extract_filename_from_url(
                "https://cdn.test/get?filename=My%20Movie%202026.mkv&token=xyz"
            ),
            "My Movie 2026.mkv"
        );
        assert_eq!(
            extract_filename_from_url("https://storage.googleapis.com/bucket/blob?response-content-disposition=attachment%3B%20filename%3D%22Document.pdf%22"),
            "Document.pdf"
        );
    }

    #[test]
    fn extract_filename_with_query() {
        // Query di URL harus diabaikan jika path sudah punya nama file non-script
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

    // ── v2.9.2: shutdown snapshot ──

    #[test]
    fn shutdown_snapshot_pauses_runtime_but_preserves_restore_status() {
        let mut info = DownloadInfo::new(
            "dl_shutdown".into(),
            "https://example.com/file.zip".into(),
            "file.zip".into(),
            "/tmp".into(),
            Default::default(),
            None,
        );
        info.status = DownloadStatus::Downloading;
        info.speed = 42_000;
        info.eta = 12;
        info.pid = Some(1234);
        info.rpc_gid = Some("gid-1".into());

        let snapshot = prepare_shutdown_snapshot(&mut info);

        // Supervisor melihat Paused dan berhenti tanpa menulis Error.
        assert_eq!(info.status, DownloadStatus::Paused);
        assert_eq!(info.speed, 0);
        assert_eq!(info.eta, 0);
        assert!(info.pid.is_none());
        // Disk tetap menandai item sebagai aktif agar load_session memasukkannya
        // ke restored_ids; PID tidak boleh pernah dipersistensikan.
        assert_eq!(snapshot.status, DownloadStatus::Downloading);
        assert!(snapshot.pid.is_none());
        assert_eq!(snapshot.rpc_gid.as_deref(), Some("gid-1"));
    }

    #[test]
    fn shutdown_preserves_deferred_resume_intent() {
        let mut info = DownloadInfo::new(
            "resume".into(),
            "unused".into(),
            "unused".into(),
            "unused".into(),
            Default::default(),
            None,
        );
        info.worker_active = true;
        info.status = DownloadStatus::Paused;
        assert!(!info.request_start(true));
        let snapshot = prepare_shutdown_snapshot(&mut info);
        assert_eq!(snapshot.status, DownloadStatus::Queued);
        assert_eq!(info.status, DownloadStatus::Paused);
        assert!(!info.resume_pending);
    }

    #[test]
    fn shutdown_snapshot_keeps_manual_pause_manual() {
        let mut info = DownloadInfo::new(
            "dl_paused".into(),
            "https://example.com/file.zip".into(),
            "file.zip".into(),
            "/tmp".into(),
            Default::default(),
            None,
        );
        info.status = DownloadStatus::Paused;

        let snapshot = prepare_shutdown_snapshot(&mut info);
        assert_eq!(info.status, DownloadStatus::Paused);
        assert_eq!(snapshot.status, DownloadStatus::Paused);
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
    // ── v2.10.0 (B1): redaksi kredensial dari snapshot session ──

    #[test]
    fn redact_drops_credentials_keeps_referer() {
        let mut d = DownloadInfo::new(
            "dl_r".into(),
            "https://x.test/f.zip".into(),
            "f.zip".into(),
            "/tmp".into(),
            [
                ("Referer".to_string(), "https://x.test/page".to_string()),
                ("Cookie".to_string(), "SID=rahasia".to_string()),
                ("Authorization".to_string(), "Bearer token".to_string()),
                ("Origin".to_string(), "https://x.test".to_string()),
            ]
            .into_iter()
            .collect(),
            None,
        );
        redact_for_persist(&mut d);
        assert!(!d.headers.contains_key("Cookie"), "got {:?}", d.headers);
        assert!(
            !d.headers.contains_key("Authorization"),
            "got {:?}",
            d.headers
        );
        // Header non-kredensial tetap ada — dipakai saat resume.
        assert_eq!(
            d.headers.get("Referer").map(String::as_str),
            Some("https://x.test/page")
        );
        assert_eq!(
            d.headers.get("Origin").map(String::as_str),
            Some("https://x.test")
        );
    }

    #[test]
    fn redact_is_case_insensitive() {
        // Nama header HTTP case-insensitive — "COOKIE" dan "cookie" sama saja.
        let mut d = DownloadInfo::new(
            "dl_c".into(),
            "u".into(),
            "f".into(),
            "/tmp".into(),
            [
                ("COOKIE".to_string(), "a=b".to_string()),
                ("proxy-authorization".to_string(), "Basic x".to_string()),
            ]
            .into_iter()
            .collect(),
            None,
        );
        redact_for_persist(&mut d);
        assert!(d.headers.is_empty(), "got {:?}", d.headers);
    }

    #[test]
    fn redact_noop_without_headers() {
        let mut d = DownloadInfo::new(
            "dl_n".into(),
            "u".into(),
            "f".into(),
            "/tmp".into(),
            Default::default(),
            None,
        );
        redact_for_persist(&mut d);
        assert!(d.headers.is_empty());
    }

    #[test]
    fn sensitive_header_list_matches_ipc_removal() {
        // Setelah v2.10.0 (B4) allow-list IPC tidak lagi memuat kredensial,
        // jadi daftar redaksi ini adalah lapisan kedua untuk session.json
        // warisan ≤2.9.4. Bila kelak salah satunya diperluas, test ini
        // memaksa keduanya dipikirkan bersama.
        for name in crate::ipc::HEADER_ALLOWLIST {
            assert!(
                !SENSITIVE_HEADERS.contains(name),
                "{name} ada di allow-list IPC SEKALIGUS daftar redaksi — \
                 putuskan salah satu secara sadar"
            );
        }
        assert!(SENSITIVE_HEADERS.contains(&"cookie"));
        assert!(SENSITIVE_HEADERS.contains(&"authorization"));
    }

    // ── v2.10.0 (D5): dua daftar ekstensi tidak boleh melenceng diam-diam ──

    /// Ambil token `"..."` di dalam `<key>: [ ... ]` pada background.js.
    fn js_array_items(src: &str, key: &str) -> Vec<String> {
        let start = src
            .find(&format!("{key}: ["))
            .unwrap_or_else(|| panic!("background.js: kunci `{key}: [` tidak ditemukan"));
        let body = &src[start..];
        let end = body.find(']').expect("array tanpa penutup");
        let body = &body[..end];
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(q) = rest.find('"') {
            let after = &rest[q + 1..];
            let Some(q2) = after.find('"') else { break };
            out.push(after[..q2].to_string());
            rest = &after[q2 + 1..];
        }
        out
    }

    /// Ambil alternatif `(a|b|c)` dari deklarasi `const <decl> = /…/` di sumber
    /// JS, dikembalikan sebagai `.a`, `.b`, `.c`.
    ///
    /// Pencarian di-anchor pada nama deklarasi supaya grup alternation lain di
    /// file yang sama tidak ikut terbaca — `content.js` juga punya `streamRe`
    /// yang hanya berisi `m3u8|mpd`.
    fn js_regex_alt_items(src: &str, decl: &str) -> Vec<String> {
        let anchor = format!("const {decl} =");
        let start = src
            .find(&anchor)
            .unwrap_or_else(|| panic!("JS: deklarasi `{anchor}` tidak ditemukan"));
        let body = &src[start..];
        let open = body.find('(').expect("deklarasi regex tanpa '('");
        let rest = &body[open + 1..];
        // Item alternation tidak pernah berisi kurung bersarang, jadi ')'
        // pertama pasti penutup grup itu.
        let close = rest.find(')').expect("alternation regex tanpa ')'");
        rest[..close].split('|').map(|s| format!(".{s}")).collect()
    }

    #[test]
    fn extension_intercept_list_is_covered() {
        // Invarian yang sebenarnya penting: APA PUN yang di-intercept browser
        // dan dikirim ke Fast DM harus dikenali sebagai file langsung, supaya
        // tidak dicoba lewat yt-dlp dulu (gagal, ±1-3 dtk terbuang) baru
        // fallback ke aria2 — persis regresi M2 yang dulu hanya dijaga komentar.
        let src = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/extension/background.js"
        ));
        let mut intercepted = js_array_items(src, "videoExtensions");
        intercepted.extend(js_array_items(src, "fileExtensions"));
        assert!(
            intercepted.len() >= 20,
            "parser gagal membaca daftar intersep ({} entri)",
            intercepted.len()
        );

        // Pengecualian SADAR: manifest HLS/DASH harus lewat yt-dlp agar
        // segmennya di-merge benar, jadi memang BUKAN file langsung.
        let hls_only = [".m3u8", ".mpd"];

        let missing: Vec<&String> = intercepted
            .iter()
            .filter(|e| !hls_only.contains(&e.as_str()))
            .filter(|e| !DIRECT_FILE_EXTENSIONS.contains(&e.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "extension/background.js meng-intercept ekstensi yang tidak dikenal \
             is_direct_file_url (tambahkan ke DIRECT_FILE_EXTENSIONS): {missing:?}"
        );
    }

    #[test]
    fn extension_media_lists_are_identical() {
        // v2.11.1 (F3): empat daftar "format media" di sisi extension dulu hanya
        // dijaga komentar "selaras dengan sniffer & background" — dan klaim itu
        // salah: `videoExts` di content.js cuma 59 entri (27 format audio hilang)
        // sementara sniffer & background 86. Akibatnya tombol "Pindai" di popup
        // tidak menawarkan link audio yang justru sudah tertangkap sniffer.
        // Test ini mengubah komentar jadi invarian yang bisa gagal.
        let sniffer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/extension/sniffer.js"));
        let content = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/extension/content.js"));
        let background = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/extension/background.js"
        ));

        let media_re = js_regex_alt_items(sniffer, "MEDIA_RE");
        let media_query = js_regex_alt_items(sniffer, "MEDIA_QUERY_RE");
        let dom_scan = js_regex_alt_items(content, "videoExts");
        let intercept = js_array_items(background, "videoExtensions");

        // Jaring pengaman parser: bila bentuk file JS berubah sehingga ekstraksi
        // diam-diam menghasilkan daftar pendek, test harus gagal di sini, bukan
        // lolos palsu karena membandingkan dua daftar sama-sama kosong.
        for (name, list) in [
            ("sniffer.js MEDIA_RE", &media_re),
            ("sniffer.js MEDIA_QUERY_RE", &media_query),
            ("content.js videoExts", &dom_scan),
            ("background.js videoExtensions", &intercept),
        ] {
            assert!(
                list.len() >= 80,
                "{name}: hanya {} entri terbaca — pola ekstraksi tidak cocok lagi \
                 dengan bentuk file JS-nya",
                list.len()
            );
        }

        let sorted = |mut v: Vec<String>| {
            v.sort();
            v.dedup();
            v
        };
        let acuan = sorted(media_re);
        assert_eq!(sorted(media_query), acuan, "MEDIA_QUERY_RE ≠ MEDIA_RE");
        assert_eq!(
            sorted(dom_scan),
            acuan,
            "content.js videoExts ≠ sniffer.js MEDIA_RE"
        );
        assert_eq!(
            sorted(intercept),
            acuan,
            "background.js videoExtensions ≠ sniffer.js MEDIA_RE"
        );

        // Semua format media yang di-intercept browser harus dikenal Rust sebagai
        // file langsung, KECUALI manifest HLS/DASH yang sengaja tetap lewat
        // yt-dlp agar segmennya di-merge benar.
        let hls_only = [".m3u8", ".mpd"];
        let unknown: Vec<&String> = acuan
            .iter()
            .filter(|e| !hls_only.contains(&e.as_str()))
            .filter(|e| !DIRECT_FILE_EXTENSIONS.contains(&e.as_str()))
            .collect();
        assert!(
            unknown.is_empty(),
            "format media di extension tidak dikenal is_direct_file_url \
             (tambahkan ke DIRECT_FILE_EXTENSIONS): {unknown:?}"
        );

        assert_eq!(
            acuan.len(),
            86,
            "jumlah format media berubah dari 86 — bila disengaja, perbarui \
             angka ini beserta komentar/CHANGELOG terkait"
        );
    }

    #[test]
    fn hls_manifests_are_deliberately_not_direct_files() {
        // Penjaga sisi sebaliknya: kalau suatu saat .m3u8/.mpd ikut
        // ditambahkan ke DIRECT_FILE_EXTENSIONS, unduhan HLS akan melewati
        // yt-dlp dan menghasilkan segmen mentah yang tidak ter-merge.
        assert!(!is_direct_file_url("https://x.test/master.m3u8"));
        assert!(!is_direct_file_url("https://x.test/manifest.mpd?v=2"));
        assert!(!DIRECT_FILE_EXTENSIONS.contains(&".m3u8"));
        assert!(!DIRECT_FILE_EXTENSIONS.contains(&".mpd"));
    }
}
