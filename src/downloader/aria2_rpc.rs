//! v2.7.0 (B2.1): klien JSON-RPC aria2 — daemon bersama + magnet/torrent.
//!
//! Mengapa daemon: jalur proses-per-unduh (`aria2.rs`) tidak bisa mengubah
//! limit setelah proses lahir, dan tidak dapat magnet sama sekali. Melalui
//! `aria2.addUri`/`changeGlobalOption`:
//! - limit total di-tegakkan LIVE oleh daemon (satu budget global untuk semua
//!   unduhan aktif — bukan pembagian statis per-proses ala M3);
//! - `magnet:?xt=urn:btih:…` akhirnya bisa diunduh;
//! - pause/resume = forcePause/unpause (state & file parsial utuh di daemon).
//!
//! Keamanan RPC: bind loopback (`--rpc-listen-all=false`) + secret acak
//! per-installasi di `~/.config/fast-dm/rpc.secret` (mode 600) — daemon yatim
//! dari sesi app sebelumnya tetap bisa dipakai (secret sama → probe cocok)
//! dan tidak bisa dikendalikan proses lain.
//!
//! GATEWAY BATCH INI: hanya magnet. http/https langsung tetap lewat
//! `aria2.rs` (perilaku lama, nol regresi); migrasi penuh = B2.2.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use super::aria2;
use super::types::{DownloadEvent, DownloadInfo, DownloadStatus};
use crate::config::Config;

static RPC_ID: AtomicU64 = AtomicU64::new(1);

/// Deteksi awal magnet (trim + case-insensitive). Hanya awalan `magnet:`.
pub fn is_magnet(url: &str) -> bool {
    url.trim_start().to_ascii_lowercase().starts_with("magnet:")
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
        let v: Value =
            serde_json::from_str(&txt).map_err(|e| format!("aria2 RPC JSON: {e}"))?;
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

async fn forget(rpc: &Rpc, gid: &str) {
    let _ = rpc.call("forceRemove", vec![json!(gid)]).await;
    let _ = rpc.call("removeDownloadResult", vec![json!(gid)]).await;
}

/// Jalur unduh magnet/torrent via daemon RPC.
pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    cfg: &Config,
) {
    // Guard ala aria2.rs: user bisa cancel/pause sebelum daemon lahir
    // (pid tak ada — kill_child_pid adalah no-op).
    {
        let i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }
    }

    let rpc = match ensure_daemon(cfg).await {
        Ok(r) => r,
        Err(e) => {
            fail(&info, &tx, format!("Aria2 RPC: {e}")).await;
            return;
        }
    };

    let (url, save_dir) = {
        let i = info.lock().await;
        (i.url.clone(), i.save_dir.clone())
    };

    // B2 inti: limit total diterapkan GLOBAL ke daemon dan berubah live
    // setiap unduhan baru start (daemon yang membagi ke semua yang aktif —
    // tidak perlu pembagian statis per-proses lagi untuk jalur ini).
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

    // pause-true dulu: hindari balapan "sudah jalan" sebelum tick pertama.
    let gid = match rpc
        .call(
            "addUri",
            vec![
                json!([url]),
                json!({"dir": save_dir, "pause": "true", "continue": "true"}),
            ],
        )
        .await
    {
        Ok(Value::String(g)) => g,
        Ok(other) => other
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        Err(e) => {
            fail(&info, &tx, format!("addUri: {e}")).await;
            return;
        }
    };
    if gid.is_empty() {
        fail(&info, &tx, "addUri: gid kosong dari aria2".into()).await;
        return;
    }
    // GID baru tidak pernah pause → unpause mungkin error; abaikan (loop
    // tetap mem-poll dan file terus jalan walau error ini muncul).
    let _ = rpc.call("unpause", vec![json!(gid.clone())]).await;

    {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            forget(&rpc, &gid).await;
            return;
        }
        i.status = DownloadStatus::Downloading;
        i.error_msg.clear();
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
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
                let _ = rpc.call("forcePause", vec![json!(gid.as_str())]).await;
                return; // resume → engine start_download → addUri gid lama
            }
            DownloadStatus::Cancelled => {
                forget(&rpc, &gid).await; // parsial dibiarkan (sama dgn proses path)
                return;
            }
            _ => {}
        }

        let st = match rpc
            .call(
                "tellStatus",
                vec![json!(gid.as_str()), json!(STATUS_KEYS)],
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                // GID hilang (daemon di-restart/manual) → tak ada yang
                // bisa di-resume; laporkan apa adanya.
                fail(&info, &tx, format!("tellStatus: {e}")).await;
                return;
            }
        };
        let p = patch_from_status(&st);
        match p.status.as_str() {
            "complete" => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused)
                {
                    continue; // aksi user diproses tick berikutnya
                }
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
                return;
            }
            "error" | "removed" => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused)
                {
                    continue;
                }
                i.status = DownloadStatus::Error;
                i.error_msg =
                    p.error.unwrap_or_else(|| "aria2: download berhenti (error)".into());
                i.speed = 0;
                i.status_detail.clear();
                let _ = tx.send(DownloadEvent::Error(i.clone()));
                drop(i);
                let _ = rpc
                    .call("removeDownloadResult", vec![json!(gid.as_str())])
                    .await;
                return;
            }
            _ => {
                let mut i = info.lock().await;
                if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused)
                {
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
                i.status_detail =
                    format!("seeders: {} · peers: {}", p.seeders, p.peers);
                i.connections = p.peers.min(255) as u8;
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
        let e =
            parse_response(json!({"error": {"code": 1, "message": "Unauthorized"}}));
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

        let err =
            json!({"status": "error", "errorCode": "5", "errorMessage": "Broken pipe"});
        let p = patch_from_status(&err);
        assert!(p.error.unwrap().contains("Broken pipe"));
    }
}
