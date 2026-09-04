//! v2.7.0 (B2.1): klien JSON-RPC aria2 — daemon bersama + magnet/torrent.
//! v2.9.0 (B2.2): http/https/ftp juga bermigrasi ke daemon (migrasi penuh).
//!
//! Mengapa daemon: jalur proses-per-unduh (`aria2.rs`) tidak bisa mengubah
//! limit setelah proses lahir, dan tidak dapat magnet sama sekali. Melalui
//! `aria2.addUri`/`changeGlobalOption`:
//! - limit total di-tegakkan LIVE oleh daemon (satu budget global untuk semua
//!   unduhan aktif — bukan pembagian statis per-proses ala M3);
//! - `magnet:?xt=urn:btih:…` bisa diunduh;
//! - pause/resume = forcePause/unpause (state & file parsial utuh di daemon);
//! - koneksi/DNS di-reuse antar-unduhan satu daemon.
//!
//! Keamanan RPC: bind loopback (`--rpc-listen-all=false`) + secret acak
//! per-installasi di `~/.config/fast-dm/rpc.secret` (mode 600) — daemon yatim
//! dari sesi app sebelumnya tetap bisa dipakai (secret sama → probe cocok)
//! dan tidak bisa dikendalikan proses lain.
//!
//! PERILAKU B2.2: http/https/ftp melewati pipeline `aria2.rs` (resolve
//! filename + tolak HTML/non-2xx + pre-check disk) SEBELUM `addUri`, lalu
//! cookie per-domain & header (mis. Referer) dikirim sebagai OPSI PER-URI
//! (`cookie`/`header`) — daemon global tidak menyentuh domain lain. Bila
//! daemon tak tersedia (mis. `rpc_port` bentrok) atau `addUri` ditolak
//! SEBELUM unduhan berjalan, `download` return `RpcOutcome::Fallback` dan
//! pemanggil boleh jatuh ke jalur per-proses lama. Magnet tetap RPC-only.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, Mutex};

use super::aria2;
use super::types::{DownloadEvent, DownloadInfo, DownloadStatus};
use crate::config::Config;

static RPC_ID: AtomicU64 = AtomicU64::new(1);

/// Deteksi awal magnet (trim + case-insensitive). Hanya awalan `magnet:`.
pub fn is_magnet(url: &str) -> bool {
    url.trim_start().to_ascii_lowercase().starts_with("magnet:")
}

/// Hasil `download` — keputusan Fallback ada di TANGAN pemanggil
/// (`spawn_supervised`), karena hanya dia yang tahu jalur per-proses tersedia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcOutcome {
    /// Jalur RPC menangani unduhan sampai terminal — Completed/Error sudah
    /// dikirim, atau user pause/cancel. Pemanggil tidak perlu aksi lain.
    Done,
    /// Daemon tak tersedia ATAU `addUri` ditolak, SEMUA sebelum unduhan
    /// berjalan — pemanggil boleh fallback ke jalur per-proses (http/ftp
    /// saja; magnet tidak pernah menghasilkan variant ini).
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GidOrigin {
    /// GID lama sudah berhasil di-unpause.
    Reused,
    /// GID baru dari addUri lahir dengan opsi pause=true.
    Added,
}

impl GidOrigin {
    fn needs_initial_unpause(self) -> bool {
        self == Self::Added
    }
}

#[derive(Clone)]
struct Rpc {
    port: u16,
    secret: String,
    http: reqwest::Client,
}

impl Rpc {
    fn new(port: u16, secret: String) -> Self {
        Self {
            port,
            secret,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(6))
                .build()
                .unwrap_or_default(),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/jsonrpc", self.port)
    }

    /// Satu pemanggilan RPC. `secret` disisipkan sebagai param pertama
    /// sesuai protokol aria2 (`token:<secret>`).
    async fn call(&self, method: &str, mut params: Vec<Value>) -> Result<Value, String> {
        let req = build_request(method, &mut params, &self.secret);
        let resp = self
            .http
            .post(self.url())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&req).unwrap_or_default())
            .send()
            .await
            .map_err(|e| format!("HTTP ke aria2 RPC: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("aria2 RPC HTTP {}", resp.status()));
        }
        // NB: fitur "json" reqwest tidak diaktifkan → parse manual via text.
        let txt = resp
            .text()
            .await
            .map_err(|e| format!("aria2 RPC read: {e}"))?;
        let v: Value = serde_json::from_str(&txt).map_err(|e| format!("aria2 RPC JSON: {e}"))?;

        parse_response(v)
    }

    /// Probe murah + autentikasi: getVersion dengan token. Ok = daemon hidup
    /// SEKALIGUS secret kita cocok (daemon asing tanpa/beda secret → Err).
    async fn probe(&self) -> Result<(), String> {
        let quick = reqwest::Client::builder()
            .timeout(Duration::from_millis(400))
            .build()
            .unwrap_or_default();
        let req = build_request("aria2.getVersion", &mut Vec::new(), &self.secret);
        let resp = quick
            .post(self.url())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&req).unwrap_or_default())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let txt = resp.text().await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        parse_response(v).map(|_| ())
    }

    async fn wait_ready(&self, budget: Duration) -> Result<(), String> {
        let t0 = tokio::time::Instant::now();
        loop {
            if self.probe().await.is_ok() {
                return Ok(());
            }
            if t0.elapsed() > budget {
                return Err(format!(
                    "daemon tidak siap dalam {} dtk (port {} mungkin dipakai daemon asing — ubah rpc_port di config.json)",
                    budget.as_secs(),
                    self.port
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Body request JSON-RPC 2.0; token disisipkan di depan params bila ada.
/// Method boleh dipanggil dengan atau tanpa prefiks `aria2.` — dinormal.
fn build_request(method: &str, params: &mut Vec<Value>, secret: &str) -> Value {
    if !secret.is_empty() {
        params.insert(0, Value::String(format!("token:{}", secret)));
    }
    json!({
        "jsonrpc": "2.0",
        "id": RPC_ID.fetch_add(1, Ordering::Relaxed),
        "method": format!(
            "aria2.{}",
            method.strip_prefix("aria2.").unwrap_or(method)
        ),
        "params": Value::Array(std::mem::take(params)),
    })
}

/// `{"result": …}` → Ok; `{"error": {code,message}}` → Err. Pesan mentah
/// diteruskan (dipakai pencocokan kata kunci oleh pemanggil).
fn parse_response(v: Value) -> Result<Value, String> {
    if let Some(err) = v.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        return Err(format!("{} (code {})", msg, code));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// Child daemon kita simpan global supaya hidup selama app (bukan per-tugas);
/// kill_on_drop → ikut mati saat runtime dimatikan; control file
/// `--auto-save-interval=20` menjaga parsian tetap bisa di-resume.
static DAEMON: tokio::sync::Mutex<Option<tokio::process::Child>> =
    tokio::sync::Mutex::const_new(None);

/// Argumen `aria2c` untuk mode daemon. Urutan stabil — diuji unit.
pub(crate) fn daemon_args(port: u16, secret: &str, cfg: &Config) -> Vec<String> {
    let mut v = vec![
        "--enable-rpc".into(),
        "--rpc-listen-all=false".into(),
        format!("--rpc-listen-port={}", port),
        format!("--rpc-secret={}", secret),
        format!("--dir={}", cfg.download_dir),
        "--auto-save-interval=20".into(),
        format!("--max-concurrent-downloads={}", cfg.max_concurrent.max(1)),
        format!(
            "--max-connection-per-server={}",
            cfg.max_connections.clamp(1, 16)
        ),
        format!("--disk-cache={}", cfg.disk_cache_size),
        format!("--file-allocation={}", cfg.file_allocation),
        format!("--user-agent={}", aria2::CHROME_UA),
        "--summary-interval=0".into(),
        // lanjutkan dari control file lintas sesi app; cek hash utk yang lengkap
        "--continue=true".into(),
    ];
    if !cfg.max_overall_speed.is_empty() && cfg.max_overall_speed != "0" {
        v.push(format!(
            "--max-overall-download-limit={}",
            cfg.max_overall_speed
        ));
    }
    if !cfg.verify_tls {
        v.push("--check-certificate=false".into());
    }
    if !cfg.proxy_url.trim().is_empty() {
        v.push(format!("--all-proxy={}", cfg.proxy_url.trim()));
    }
    v
}

/// B2.2: opsi per-URI untuk `aria2.addUri` — murni, teruji.
///
/// Mirror flag per-proses `aria2.rs` yang valid sebagai "URI option"
/// (lihat manual aria2). `pause` selalu "true" dulu; pemanggil memanggil
/// `unpause` setelah `addUri` (pola B2.1 — hindari balapan "sudah jalan"
/// sebelum tick pertama). `min-split-size`/`piece-length` mengikuti jalur
/// per-proses; koneksi-per-server memakai nilai GLOBAL daemon (daemon_args).
pub(crate) fn adduri_options(
    save_dir: &str,
    filename: Option<&str>,
    cookie: Option<&str>,
    headers: &HashMap<String, String>,
    cfg: &Config,
) -> Value {
    let mut o: Map<String, Value> = Map::new();
    o.insert("dir".into(), json!(save_dir));
    o.insert("pause".into(), json!("true"));
    o.insert("continue".into(), json!("true"));
    // `out` hanya bermakna untuk http/ftp — magnet: nama dari metadata torrent.
    if let Some(f) = filename.filter(|f| !f.is_empty()) {
        o.insert("out".into(), json!(f));
    }
    // Cookie per-domain (walk-up dari file extension) — daemon global tidak
    // boleh memakai cookie domain lain untuk URI ini.
    if let Some(c) = cookie.map(str::trim).filter(|c| !c.is_empty()) {
        o.insert("cookie".into(), json!(c));
    }
    // Header (mis. Referer) — strip \r\n anti injeksi, sama dengan jalur CLI.
    let hs: Vec<String> = headers
        .iter()
        .map(|(k, v)| (k.replace(['\r', '\n'], ""), v.replace(['\r', '\n'], "")))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    if !hs.is_empty() {
        o.insert("header".into(), json!(hs));
    }
    o.insert("timeout".into(), json!(cfg.timeout.to_string()));
    o.insert("connect-timeout".into(), json!("15"));
    o.insert("max-tries".into(), json!(cfg.retry_count.to_string()));
    o.insert("retry-wait".into(), json!(cfg.retry_wait.to_string()));
    o.insert("min-split-size".into(), json!("1M"));
    o.insert("piece-length".into(), json!("1M"));
    // Auto-rename (default ON) → JANGAN overwrite: tabrakan jadi "file (1).ext".
    o.insert(
        "allow-overwrite".into(),
        json!((!cfg.auto_file_renaming).to_string()),
    );
    Value::Object(o)
}

/// Pastikan daemon RPC siap: reuse milik sendiri (probe ber-token sukses),
/// atau spawn `aria2c` baru. Err = pesan siap-tampil.
async fn ensure_daemon(cfg: &Config) -> Result<Rpc, String> {
    let rpc = Rpc::new(cfg.rpc_port, Config::rpc_secret());
    // Sudah hidup (milik kita — token cocok) → pakai.
    if rpc.probe().await.is_ok() {
        return Ok(rpc);
    }
    let mut guard = DAEMON.lock().await;
    // Setelah dapat lock, cek ulang: tugas lain mungkin baru saja spawn.
    if rpc.probe().await.is_ok() {
        return Ok(rpc);
    }
    // Child lama masih hidup? Biarkan (probe mungkin kalah cepat dari bind)
    // dan tunggu; kalau mati → spawn baru.
    let mut child_alive = false;
    if let Some(ch) = guard.as_mut() {
        match ch.try_wait() {
            Ok(None) => child_alive = true,
            Ok(Some(_)) | Err(_) => {
                *guard = None;
            }
        }
    }
    if !child_alive {
        let args = daemon_args(cfg.rpc_port, &rpc.secret, cfg);
        let mut cmd = tokio::process::Command::new("aria2c");
        cmd.args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().map_err(|e| {
            format!("aria2c gagal dijalankan: {e} (pasang aria2 untuk unduh magnet)")
        })?;
        *guard = Some(child);
    }
    drop(guard);
    rpc.wait_ready(Duration::from_secs(6)).await?;
    Ok(rpc)
}

/// Hasil proyeksi satu `tellStatus` → field patch GUI (murni, teruji).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Patch {
    pub status: String,
    pub total: u64,
    pub completed: u64,
    pub speed: u64,
    pub seeders: u64,
    pub peers: u64,
    pub first_file: Option<String>,
    pub error: Option<String>,
}

fn field_u64(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn patch_from_status(v: &Value) -> Patch {
    let status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut error = None;
    if status == "error" || status == "removed" {
        let code = v
            .get("errorCode")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let msg = v
            .get("errorMessage")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        if !msg.is_empty() || !code.is_empty() {
            error = Some(format!("[{}] {}", code, msg).trim().to_string());
        }
    }
    let first_file = v
        .get("files")
        .and_then(|f| f.get(0))
        .and_then(|f| f.get("path"))
        .and_then(|p| p.as_str())
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
        .filter(|s| !s.is_empty());
    Patch {
        status,
        total: field_u64(v, "totalLength"),
        completed: field_u64(v, "completedLength"),
        speed: field_u64(v, "downloadSpeed"),
        seeders: field_u64(v, "numSeeders"),
        peers: field_u64(v, "numPeers"),
        first_file,
        error,
    }
}

const STATUS_KEYS: &[&str] = &[
    "status",
    "totalLength",
    "completedLength",
    "downloadSpeed",
    "files",
    "numSeeders",
    "numPeers",
    "errorCode",
    "errorMessage",
];

/// Terminal kegagalan yang hormat-cancel: jangan menimpa keputusan user.
async fn fail(
    info: &Arc<Mutex<DownloadInfo>>,
    tx: &mpsc::UnboundedSender<DownloadEvent>,
    msg: String,
) {
    let mut i = info.lock().await;
    if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
        return;
    }
    i.status = DownloadStatus::Error;
    i.error_msg = msg;
    i.speed = 0;
    i.status_detail.clear();
    let _ = tx.send(DownloadEvent::Error(i.clone()));
}

async fn forget(rpc: &Rpc, gid: &str) -> Result<(), String> {
    let remove = rpc.call("forceRemove", vec![json!(gid)]).await;
    let cleanup = rpc.call("removeDownloadResult", vec![json!(gid)]).await;

    // Idempotent terhadap race dengan supervisor lain: bila salah satu
    // operasi berhasil, task tidak lagi dapat berjalan/meninggalkan result.
    if remove.is_ok() || cleanup.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "forceRemove: {}; removeDownloadResult: {}",
            remove.unwrap_err(),
            cleanup.unwrap_err()
        ))
    }
}

/// Hapus task RPC berdasarkan GID tanpa men-spawn daemon baru. Ini penting
/// untuk Cancel/Hapus pada item yang sudah Paused: supervisor polling-nya
/// sudah selesai, jadi tidak ada task lain yang akan memanggil forceRemove.
pub(crate) async fn remove_gid(gid: &str, cfg: &Config) -> Result<(), String> {
    let rpc = Rpc::new(cfg.rpc_port, Config::rpc_secret());
    // Langsung lakukan operasi dengan timeout Rpc::call. Probe terpisah dapat
    // memberi false-negative ketika daemon sedang sibuk dan justru melewatkan
    // satu-satunya kesempatan membersihkan task paused.
    forget(&rpc, gid).await
}

/// Pause seluruh GID lalu matikan daemon milik Fast-DM secara graceful.
/// Daemon yang di-spawn proses ini juga di-reap; bila tidak merespons dalam
/// timeout terbatas, process group diakhiri agar tidak menjadi orphan.
pub(crate) async fn shutdown_daemon(gids: &[String], cfg: &Config) -> Result<(), String> {
    let rpc = Rpc::new(cfg.rpc_port, Config::rpc_secret());

    // Satu RPC menghentikan seluruh task daemon, termasuk task paused yatim
    // yang tidak lagi punya DownloadInfo. Ini juga menghindari timeout serial
    // hingga ratusan GID saat session penuh.
    let pause_all =
        tokio::time::timeout(Duration::from_secs(2), rpc.call("forcePauseAll", vec![])).await;
    // Error method tidak selalu berarti daemon mati (misalnya respons JSON-RPC
    // ditolak setelah request mencapai daemon). Probe hanya pada jalur error;
    // jangan lewatkan shutdown daemon yang sebenarnya masih terautentikasi.
    let reachable = matches!(pause_all, Ok(Ok(_))) || rpc.probe().await.is_ok();
    let mut shutdown_error = None;

    if reachable {
        let graceful =
            tokio::time::timeout(Duration::from_secs(2), rpc.call("shutdown", vec![])).await;
        if !matches!(graceful, Ok(Ok(_))) {
            // `shutdown` normal seharusnya cukup setelah forcePauseAll. Gunakan
            // forceShutdown sebagai jaring pengaman bila ada task daemon lain.
            let force =
                tokio::time::timeout(Duration::from_secs(2), rpc.call("forceShutdown", vec![]))
                    .await;
            if !matches!(force, Ok(Ok(_))) {
                shutdown_error = Some(format!(
                    "shutdown gagal untuk {} GID yang dikenal",
                    gids.len()
                ));
            }
        }
    }

    // Child hanya Some bila daemon dibuat proses ini. Daemon yatim yang
    // berhasil di-reuse tidak punya Child handle, tetapi sudah menerima RPC
    // shutdown di atas.
    let child = { DAEMON.lock().await.take() };
    if let Some(mut child) = child {
        let pid = child.id();
        if !matches!(
            tokio::time::timeout(Duration::from_secs(3), child.wait()).await,
            Ok(Ok(_))
        ) {
            if let Some(pid) = pid {
                super::kill_child_group_hard(pid);
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        return Ok(());
    }

    if !reachable {
        // Tanpa GID tidak ada pekerjaan RPC yang perlu dipertahankan. Jika ada
        // GID, jangan menganggapnya stale hanya karena probe dua detik gagal:
        // simpan GID agar start berikutnya masih bisa mencoba unpause.
        return if gids.is_empty() {
            Ok(())
        } else {
            Err("daemon tidak merespons forcePauseAll".into())
        };
    }

    // Untuk daemon yatim/reuse kita tidak memiliki Child handle. Konfirmasi
    // port terautentikasi benar-benar berhenti sebelum caller menghapus GID
    // dari snapshot; respons "OK" dapat tiba sebelum proses selesai exit.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut consecutive_misses = 0;
    while tokio::time::Instant::now() < deadline {
        if rpc.probe().await.is_err() {
            consecutive_misses += 1;
            if consecutive_misses >= 2 {
                return Ok(());
            }
        } else {
            consecutive_misses = 0;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Err(shutdown_error.unwrap_or_else(|| "daemon masih hidup setelah shutdown".into()))
}

/// Jalur unduh via daemon RPC — v2.7.0 (B2.1) magnet; v2.9.0 (B2.2)
/// http/https/ftp juga (dengan pipeline resolve `aria2.rs` sebelum `addUri`).
pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    cfg: &Config,
) -> RpcOutcome {
    // Guard ala aria2.rs: user bisa cancel/pause sebelum daemon lahir
    // (pid tak ada — kill_child_pid adalah no-op).
    {
        let i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return RpcOutcome::Done;
        }
    }

    let (url, save_dir) = {
        let i = info.lock().await;
        (i.url.clone(), i.save_dir.clone())
    };
    let is_mag = is_magnet(&url);

    // B2.2: http/https/ftp — resolve filename + tolak halaman HTML +
    // pre-check disk (identik dengan pipeline per-proses; tanpa ini "file
    // .php" bisa masuk antrean RPC dan nama Content-Disposition/redirect
    // terlewat).
    if !is_mag {
        if let Err(msg) = aria2::resolve_filename(&info, cfg.verify_tls).await {
            fail(&info, &tx, msg).await;
            return RpcOutcome::Done;
        }
        let (size, dir) = {
            let i = info.lock().await;
            (i.total_size, i.save_dir.clone())
        };
        if size > 0 && !aria2::has_space(&dir, size) {
            fail(
                &info,
                &tx,
                format!(
                    "Ruang disk tidak cukup — butuh {}",
                    super::types::format_size(size)
                ),
            )
            .await;
            return RpcOutcome::Done;
        }
        // User bisa pause/cancel selama resolve (±10 dtk) — hormati.
        {
            let i = info.lock().await;
            if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
                return RpcOutcome::Done;
            }
        }
    }

    let rpc = match ensure_daemon(cfg).await {
        Ok(r) => r,
        Err(e) => {
            if is_mag {
                fail(&info, &tx, format!("Aria2 RPC: {e}")).await;
                return RpcOutcome::Done;
            }
            // B2.2: http/ftp — biarkan pemanggil coba jalur per-proses lama.
            tracing::warn!("B2.2: daemon RPC tak tersedia — fallback per-proses: {e}");
            return RpcOutcome::Fallback;
        }
    };

    // B2 inti: limit total diterapkan GLOBAL ke daemon dan berubah live
    // setiap unduhan baru start (daemon yang membagi ke semua yang aktif —
    // tidak perlu pembagian statis per-proses lagi; B2.2 menjadikannya
    // berlaku untuk SEMUA unduhan http/ftp juga).
    let limit = if cfg.max_overall_speed.is_empty() {
        "0".to_string()
    } else {
        cfg.max_overall_speed.clone()
    };
    let _ = rpc
        .call(
            "changeGlobalOption",
            vec![json!({ "max-overall-download-limit": limit })],
        )
        .await; // best-effort

    // B2.2: opsi per-URI — cookie per-domain + header (mis. Referer) +
    // timeout/retry mengikuti Pengaturan. `out` hanya untuk http/ftp.
    let (out, cookie, headers) = {
        let i = info.lock().await;
        (
            if is_mag {
                None
            } else {
                Some(i.filename.clone())
            },
            aria2::cookie_header_for(&url),
            i.headers.clone(),
        )
    };
    let options = adduri_options(&save_dir, out.as_deref(), cookie.as_deref(), &headers, cfg);

    // pause-true dulu: hindari balapan "sudah jalan" sebelum tick pertama.
    // v2.9.1: pause/resume NATIVE — bila unduhan ini sebelumnya dijeda
    // (forcePause), task-nya masih hidup di daemon dengan GID yang kita
    // simpan. Resume = unpause GID yang SAMA; addUri baru akan menduplikat
    // task (GID paused lama macet di daemon + potensi dua penulis file).
    let mut gid: Option<String> = info.lock().await.rpc_gid.clone();
     // Origin menjadi Reused hanya bila GID lama BENAR-BENAR berhasil
    // di-unpause. Bila GID stale lalu addUri membuat pengganti, origin tetap
    // Added sehingga task baru (pause=true) wajib di-unpause.
    let mut gid_origin = GidOrigin::Added;
    if let Some(ref g) = gid {
        // Task yang dijeda pasti state paused → unpause cukup.
        if let Err(e) = rpc.call("unpause", vec![json!(g.as_str())]).await {
            // GID hilang (daemon di-restart/manual di-reset) → buang GID
            // basi dan tambah task baru dari nol (addUri di bawah).
            tracing::warn!("RPC: GID {g} tak bisa di-unpause ({e}) — addUri ulang");
            info.lock().await.rpc_gid = None;
            gid = None;
        } else {
            gid_origin = GidOrigin::Reused;
        }
    }
    let gid = match gid {
        Some(g) => g,
        None => {
            // pause-true dulu: hindari balapan "sudah jalan" sebelum tick
            // pertama; unpause menyusul setelah addUri.
            match rpc.call("addUri", vec![json!([url]), json!(options)]).await {

                Ok(Value::String(g)) => g,
                Ok(other) => other
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                Err(e) => {
                    if is_mag {
                        fail(&info, &tx, format!("addUri: {e}")).await;
                        return RpcOutcome::Done;
                    }
                    tracing::warn!("B2.2: addUri ditolak daemon — fallback per-proses: {e}");
                    return RpcOutcome::Fallback;
                }
            }
        }
    };
    if gid.is_empty() {
        if is_mag {
            fail(&info, &tx, "addUri: gid kosong dari aria2".into()).await;
            return RpcOutcome::Done;
        }
        return RpcOutcome::Fallback;
    }
    if gid_origin.needs_initial_unpause() {
        // GID baru lahir dalam keadaan paused (opsi `pause:true`) → wajib
        // berhasil di-unpause. Kalau gagal, bersihkan task sebelum fallback;
        // membiarkannya akan membuat UI "Downloading" pada task paused abadi.
        if let Err(e) = rpc.call("unpause", vec![json!(gid.as_str())]).await {
            let _ = forget(&rpc, &gid).await;
            info.lock().await.rpc_gid = None;
            if is_mag {
                fail(&info, &tx, format!("unpause: {e}")).await;
                return RpcOutcome::Done;
            }
            tracing::warn!("B2.2: unpause GID baru gagal — fallback per-proses: {e}");
            return RpcOutcome::Fallback;
        }
    }

    // Putuskan state secara atomik di bawah Mutex, tetapi jangan pegang
    // Mutex DownloadInfo selama network await: tombol pause/cancel harus
    // tetap responsif ketika RPC sedang lambat.
    let aborted = {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            if i.status == DownloadStatus::Paused {
                // Publish GID sebelum network await agar Cancel/Hapus yang
                // menyusul dapat forceRemove task ini sendiri.
                i.rpc_gid = Some(gid.clone());
            }
            Some(i.status)
        } else {
            i.rpc_gid = Some(gid.clone());
            i.status = DownloadStatus::Downloading;
            i.error_msg.clear();
            let _ = tx.send(DownloadEvent::Progress(i.clone()));
            None
        }
    };
    if let Some(user_status) = aborted {
        // Semboyan pause/cancel yang datang saat kita menyiapkan daemon
        // (unpause sudah terlanjur) — forcePause lagi supaya state daemon
        // konsisten dengan keputusan user; GID tetap tersimpan untuk
        // resume native berikutnya.
        if user_status == DownloadStatus::Paused {
            let _ = rpc.call("forcePause", vec![json!(gid.as_str())]).await;
        } else {
            let _ = forget(&rpc, &gid).await;
            info.lock().await.rpc_gid = None;
        }
        return RpcOutcome::Done;
    }

    let mut tick = tokio::time::interval(Duration::from_millis(600));
    loop {
        tick.tick().await;

        // Hormati user dulu (pause engine = kill proses di jalur lain; di
        // sini = forcePause/forceRemove — tanpa pid).
        let user = {
            let i = info.lock().await;
            i.status // DownloadStatus: Copy — clone tak perlu (clippy)
        };
        match user {
            DownloadStatus::Paused => {
                // v2.9.1: task dibiarkan hidup di daemon dalam keadaan
                // paused; GID sudah tersimpan di info.rpc_gid sehingga
                // resume (start_download → download() lagi) tinggal
                // unpause GID yang sama — bukan addUri duplikat.
                let _ = rpc.call("forcePause", vec![json!(gid.as_str())]).await;
                return RpcOutcome::Done;
            }
            DownloadStatus::Cancelled => {
                // Task dilepas dari daemon (parsial dibiarkan, sama seperti
                // jalur proses) — GID tak valid lagi untuk resume.
                let _ = forget(&rpc, &gid).await;

                info.lock().await.rpc_gid = None;
                return RpcOutcome::Done;
            }
            _ => {}
        }

        let st = match rpc
            .call("tellStatus", vec![json!(gid.as_str()), json!(STATUS_KEYS)])

            .await
        {
            Ok(v) => v,
            Err(e) => {
                // GID hilang (daemon di-restart/manual) → tak ada yang
                // bisa di-resume; laporkan apa adanya.
                fail(&info, &tx, format!("tellStatus: {e}")).await;
                return RpcOutcome::Done;
            }
        };
        let p = patch_from_status(&st);
        match p.status.as_str() {
            "complete" => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {

                    continue; // aksi user diproses tick berikutnya
                }
                i.rpc_gid = None; // hasil dihapus dari daftar daemon
                if let Some(f) = p.first_file.as_deref() {
                    if aria2::is_generic_filename(&i.filename) {
                        i.filename = f.to_string();
                    }
                }
                i.downloaded = p.completed;
                i.total_size = p.total.max(p.completed);
                i.progress = 100.0;
                i.speed = 0;
                i.eta = 0;
                i.status_detail.clear();
                i.error_msg.clear();
                i.status = DownloadStatus::Completed;
                let _ = tx.send(DownloadEvent::Completed(i.clone()));
                drop(i);
                let _ = rpc
                    .call("removeDownloadResult", vec![json!(gid.as_str())])
                    .await;
                return RpcOutcome::Done;
            }
            "error" | "removed" => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
                    continue;
                }
                // "error" yang disebabkan forceRemove hasil cancel user
                // sudah dibersihkan di cabang Cancelled di atas; di sini
                // task benar-benar mati → GID tak bisa di-resume.
                i.rpc_gid = None;
                i.status = DownloadStatus::Error;
                i.error_msg = p
                    .error
                    .unwrap_or_else(|| "aria2: download berhenti (error)".into());
                i.speed = 0;
                i.status_detail.clear();
                let _ = tx.send(DownloadEvent::Error(i.clone()));
                drop(i);
                let _ = rpc
                    .call("removeDownloadResult", vec![json!(gid.as_str())])
                    .await;
                return RpcOutcome::Done;
            }
            _ => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {

                    continue;
                }
                if !matches!(i.status, DownloadStatus::Downloading) {
                    i.status = DownloadStatus::Downloading;
                }
                if let Some(f) = p.first_file.as_deref() {
                    if aria2::is_generic_filename(&i.filename) {
                        i.filename = f.to_string();
                    }
                }
                i.total_size = p.total;
                i.downloaded = p.completed;
                i.speed = p.speed;
                i.eta = if p.speed > 0 && p.total > p.completed {
                    (p.total - p.completed) / p.speed
                } else {
                    0
                };
                i.progress = if p.total > 0 {
                    (p.completed as f64 * 100.0 / p.total as f64).min(99.5)
                } else {
                    0.0
                };
                // seeders/peers hanya bermakna untuk torrent — http/ftp biarkan
                // kosong (tellStatus tidak punya field koneksi aktif).
                if is_mag {
                    i.status_detail = format!("seeders: {} · peers: {}", p.seeders, p.peers);
                    i.connections = p.peers.min(255) as u8;
                }
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_magnet_detects_scheme_only() {
        assert!(is_magnet("magnet:?xt=urn:btih:0123abcd"));
        assert!(is_magnet("  MAGNET:?xt=urn:btih:0123"));
        assert!(!is_magnet("https://site/magnet:1"));
        assert!(!is_magnet("http"));
    }

    #[test]
    fn only_added_gid_needs_initial_unpause() {
        // Regresi v2.9.2: GID saved yang gagal dipakai lalu diganti addUri
        // harus diperlakukan Added, bukan Reused hanya karena awalnya Some.
        assert!(GidOrigin::Added.needs_initial_unpause());
        assert!(!GidOrigin::Reused.needs_initial_unpause());
    }


    #[test]
    fn build_request_prefixes_token() {
        let mut p = vec![json!(["magnet:?x"])];
        let r = build_request("addUri", &mut p, "s3cr3t");
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["method"], "aria2.addUri");
        assert_eq!(r["params"][0], "token:s3cr3t");
        assert_eq!(r["params"][1], json!(["magnet:?x"]));
    }

    #[test]
    fn build_request_without_secret() {
        let mut p = vec![json!(1)];
        let r = build_request("tellStatus", &mut p, "");
        assert_eq!(r["params"][0], json!(1));
    }

    #[test]
    fn method_prefix_idempotent() {
        let r = build_request("aria2.getVersion", &mut vec![], "t");
        assert_eq!(r["method"], "aria2.getVersion");
    }

    #[test]
    fn parse_response_shapes() {
        assert_eq!(parse_response(json!({"result": 5})).unwrap(), json!(5));
        assert_eq!(parse_response(json!({})).unwrap(), Value::Null);
        let e = parse_response(json!({"error": {"code": 1, "message": "Unauthorized"}}));

        assert!(e.unwrap_err().contains("Unauthorized"));
    }

    #[test]
    fn daemon_args_core_flags() {
        let cfg = Config {
            proxy_url: "http://127.0.0.1:8118".into(),
            max_overall_speed: "5M".into(),
            verify_tls: false,
            ..Config::default()
        };
        let a = daemon_args(6800, "sec", &cfg);
        let j = a.join(" ");
        assert!(a[0] == "--enable-rpc" && a[1] == "--rpc-listen-all=false");
        assert!(j.contains("--rpc-listen-port=6800"));
        assert!(j.contains("--rpc-secret=sec"));
        assert!(j.contains("--auto-save-interval=20"));
        assert!(j.contains("--max-overall-download-limit=5M"));
        assert!(j.contains("--check-certificate=false"));
        assert!(j.contains("--all-proxy=http://127.0.0.1:8118"));
        // kosong/0 → tanpa flag limit; proxy kosong → tanpa flag
        let plain = Config::default();
        let j2 = daemon_args(6800, "s", &plain).join(" ");
        assert!(!j2.contains("--max-overall-download-limit"));
        assert!(!j2.contains("--all-proxy"));
        assert!(!j2.contains("--check-certificate"));
    }

    // ── B2.2: adduri_options ──

    #[test]
    fn adduri_options_base_flags() {
        let o = adduri_options("/dl", None, None, &HashMap::new(), &Config::default());
        assert_eq!(o["dir"], "/dl");
        assert_eq!(o["pause"], "true");
        assert_eq!(o["continue"], "true");
        // Default Config::default(): timeout 30, retry 5, wait 3, renaming ON.
        assert_eq!(o["timeout"], "30");
        assert_eq!(o["connect-timeout"], "15");
        assert_eq!(o["max-tries"], "5");
        assert_eq!(o["retry-wait"], "3");
        assert_eq!(o["min-split-size"], "1M");
        assert_eq!(o["piece-length"], "1M");
        assert_eq!(o["allow-overwrite"], "false"); // auto_file_renaming default true
        assert!(o.get("out").is_none());
        assert!(o.get("cookie").is_none());
        assert!(o.get("header").is_none());
    }

    #[test]
    fn adduri_options_http_adds_out_cookie_header() {
        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://site.com/page".to_string());
        let o = adduri_options(
            "/dl",
            Some("video.mp4"),
            Some("a=1; b=2"),
            &headers,
            &Config::default(),
        );
        assert_eq!(o["out"], "video.mp4");
        assert_eq!(o["cookie"], "a=1; b=2");
        assert_eq!(o["header"], json!(["Referer: https://site.com/page"]));
    }

    #[test]
    fn adduri_options_strips_crlf_and_skips_empty() {
        let mut headers = HashMap::new();
        headers.insert("Evil\r\nX-Inject".to_string(), "y".to_string());
        headers.insert("Empty".to_string(), String::new());
        let o = adduri_options("/dl", None, None, &headers, &Config::default());
        // CRLF di-strip per karakter — fragmen menyambung (perilaku sama dengan
        // jalur CLI): yang penting tidak ada CR/LF tersisa, sehingga header
        // baru tidak bisa disisipkan lewat nama header palsu.
        assert_eq!(o["header"], json!(["EvilX-Inject: y"]));
        // Cookie kosong/whitespace saja tidak dikirim.
        let o2 = adduri_options(
            "/dl",
            None,
            Some("   "),
            &HashMap::new(),
            &Config::default(),
        );
        assert!(o2.get("cookie").is_none());
        // `out` kosong juga tidak dikirim.
        let o3 = adduri_options("/dl", Some(""), None, &HashMap::new(), &Config::default());
        assert!(o3.get("out").is_none());
    }

    #[test]
    fn adduri_options_follows_user_settings() {
        let mut cfg = Config::default();
        cfg.auto_file_renaming = false;
        cfg.timeout = 60;
        cfg.retry_count = 9;
        cfg.retry_wait = 7;
        let o = adduri_options("/dl", None, None, &HashMap::new(), &cfg);
        assert_eq!(o["allow-overwrite"], "true");
        assert_eq!(o["timeout"], "60");
        assert_eq!(o["max-tries"], "9");
        assert_eq!(o["retry-wait"], "7");
    }

    #[test]
    fn field_u64_accepts_strings_and_numbers() {
        let v = json!({"a": "42", "b": 7, "c": "x"});
        assert_eq!(field_u64(&v, "a"), 42);
        assert_eq!(field_u64(&v, "b"), 7);
        assert_eq!(field_u64(&v, "c"), 0);
        assert_eq!(field_u64(&v, "missing"), 0);
    }

    #[test]
    fn patch_from_status_maps_aria2_fields() {
        let st = json!({
            "status": "Active",
            "totalLength": "1000",
            "completedLength": "250",
            "downloadSpeed": "500",
            "numSeeders": "2",
            "numPeers": "7",
            "files": [{"path": "/home/u/Downloads/ubuntu.iso"}]
        });
        let p = patch_from_status(&st);
        assert_eq!(p.status, "active");
        assert_eq!((p.total, p.completed, p.speed), (1000, 250, 500));
        assert_eq!((p.seeders, p.peers), (2, 7));
        assert_eq!(p.first_file.as_deref(), Some("ubuntu.iso"));
        assert!(p.error.is_none());

        let err = json!({"status": "error", "errorCode": "5", "errorMessage": "Broken pipe"});

        let p = patch_from_status(&err);
        assert!(p.error.unwrap().contains("Broken pipe"));
    }
}
