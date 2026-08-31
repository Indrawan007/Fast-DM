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
    _config: &Config,
) -> Outcome {
    let (url, save_dir, headers, quality, filename) = {
        let mut i = info.lock().await;
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

    // Cek ketersediaan yt-dlp SEKALI: kalau tidak ada, jangan fallback ke aria2
    // (larinya hanya menghasilkan error membingungkan).
    // B10: spawn_blocking — jangan blokir thread executor tokio menunggu proses.
    let available = tokio::task::spawn_blocking(|| {
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

    if !available {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async {
            let mut i = info.lock().await;
            i.status = DownloadStatus::Error;
            i.error_msg =
                "yt-dlp tidak terinstall — jalankan: sudo apt install yt-dlp".to_string();
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
        });
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
        "--socket-timeout".into(), "15".into(),
        "--retries".into(), "5".into(),
        "--merge-output-format".into(), "mp4".into(),
    ]);

    // Cookies (dari cookies.txt / browser) + Referer & header kustom extension
    cmd.extend(cookie_args(&url));
    for (k, v) in &headers {
        let k = k.replace(['\r', '\n'], "");
        let v = v.replace(['\r', '\n'], "");
        if !k.is_empty() && !v.is_empty() {
            cmd.push("--add-header".into());
            cmd.push(format!("{}:{}", k, v));
        }
    }
    cmd.push(url);

    let info_run = info.clone();
    let tx_run = tx.clone();
    let ok = tokio::task::spawn_blocking(move || {
        run_ytdlp(cmd, info_run, tx_run)
    })
    .await
    .unwrap_or(false);

    if ok {
        return Outcome::Completed;
    }

    // yt-dlp gagal → reset status supaya aria2 boleh mencoba sebagai fallback.
    // (run_ytdlp sudah mengirim event Error, tapi aria2 akan mengirim
    //  Progress/Downloading berikutnya sehingga UI tidak terjebak di Error.)
    let rt = tokio::runtime::Handle::current();
    rt.block_on(async {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }
        i.status = DownloadStatus::Downloading;
        i.speed = 0;
        i.error_msg.clear();
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
    });

    Outcome::Failed
}
