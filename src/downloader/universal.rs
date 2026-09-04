use super::types::*;
use crate::config::Config;
use crate::downloader::youtube::{cookie_args, output_template, quality_args, run_ytdlp};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Hasil percobaan yt-dlp universal (semua situs non-YouTube).
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// yt-dlp berhasil — status Completed sudah dikirim.
    Completed,
    /// yt-dlp gagal (mis. situs tidak didukung / butuh fallback) —
    /// status di-reset ke Downloading, mod.rs boleh mencoba aria2.
    Failed,
    /// yt-dlp tidak terinstall — status Error sudah di-set, JANGAN fallback
    /// (pesan "install yt-dlp" lebih jelas daripada error aria2).
    MissingTool,
}

/// Unduh URL non-YouTube via yt-dlp sebagai "resolver universal" (gaya IDM):
/// yt-dlp mengenali 1800+ situs (TikTok, Instagram, Facebook, Twitter/X, Vimeo,
/// Twitch, situs berita, HLS/m3u8, dll.) dan menangani login + kualitas.
pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &Config,
) -> Outcome {
    // Guard: user bisa cancel/pause di jeda sebelum child proses lahir
    // (pid belum ada → kill_child_pid tidak berdampak). Tanpa guard, status
    // ditimpa Downloading dan download yang "dibatalkan" jalan terus.
    let (url, save_dir, headers, quality, filename) = {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return Outcome::Failed;
        }
        i.status = DownloadStatus::Downloading;
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
        (
            i.url.clone(),
            i.save_dir.clone(),
            i.headers.clone(),
            i.quality.clone(),
            i.filename.clone(),
        )
    };

    // B10: spawn_blocking — jangan blokir thread executor tokio menunggu proses.
    // Hasil di-cache per-sesi: spawn "yt-dlp --version" (~50–100 ms) tidak
    // perlu diulang untuk setiap download.
    static YTDLP_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let available = if let Some(cached) = YTDLP_AVAILABLE.get() {
        *cached
    } else {
        let avail = tokio::task::spawn_blocking(|| {
            Command::new("yt-dlp")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        let _ = YTDLP_AVAILABLE.set(avail);
        avail
    };

    if !available {
        // v2.3.1 (M1): dulu `Handle::current().block_on` dari konteks async —
        // panic di runtime Tokio; kini await langsung.
        let mut i = info.lock().await;
        i.status = DownloadStatus::Error;
        i.error_msg = "yt-dlp tidak terinstall — jalankan: sudo apt install yt-dlp".to_string();
        i.speed = 0;
        let _ = tx.send(DownloadEvent::Error(i.clone()));
        return Outcome::MissingTool;
    }

    let mut cmd = vec!["yt-dlp".to_string()];
    cmd.extend(quality_args(quality.as_deref()));
    cmd.extend([
        "--output".into(),
        output_template(&save_dir, &filename),
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--newline".into(),
        "--no-colors".into(),
        "--no-overwrites".into(),
        "--continue".into(),
        "--socket-timeout".into(),
        "15".into(),
        "--retries".into(),
        "5".into(),
        "--merge-output-format".into(),
        "mp4".into(),
    ]);

    // Cookies (dari cookies.txt / browser) + Referer & header kustom extension
    cmd.extend(cookie_args(&url));

    // v2.9.1: batas kecepatan total per unduhan hidup (pembagian M3 dari
    // engine) — sama seperti jalur YouTube; sebelumnya limit Pengaturan
    // tidak berlaku untuk yt-dlp.
    if !config.max_overall_speed.is_empty() && config.max_overall_speed != "0" {
        cmd.extend(["--limit-rate".into(), config.max_overall_speed.clone()]);
    }

    // v2.4.0 (D3): proxy juga untuk jalur resolver universal
    if !config.proxy_url.trim().is_empty() {
        cmd.extend(["--proxy".into(), config.proxy_url.trim().to_string()]);
    }
    for (k, v) in &headers {
        let k = k.replace(['\r', '\n'], "");
        let v = v.replace(['\r', '\n'], "");
        if !k.is_empty() && !v.is_empty() {
            cmd.push("--add-header".into());
            cmd.push(format!("{}:{}", k, v));
        }
    }
    cmd.push(url);

    let ok = run_ytdlp(cmd, info.clone(), tx.clone()).await;

    if ok {
        return Outcome::Completed;
    }

    // yt-dlp gagal → reset status supaya aria2 boleh mencoba sebagai fallback.
    // (run_ytdlp sudah mengirim event Error, tapi aria2 akan mengirim
    //  Progress/Downloading berikutnya sehingga UI tidak terjebak di Error.)
    // v2.3.1 (M1): dulu blok ini memakai `Handle::current().block_on` dari
    // dalam konteks async — panic di runtime; kini await langsung.
    {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return Outcome::Failed;
        }
        i.status = DownloadStatus::Downloading;
        i.speed = 0;
        i.error_msg.clear();
        i.status_detail.clear();
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
    }

    Outcome::Failed
}
