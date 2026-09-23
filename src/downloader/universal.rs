use super::types::*;
use crate::config::Config;
use crate::downloader::aria2::conn_per_server;
use crate::downloader::youtube::{
    cookie_args, merge_output_format, network_args, output_template, quality_args,
    run_ytdlp_with_stdin, ytdlp_proxy_args, PrivateFileGuard, YTDLP_BATCH_FILE_STDIN,
};
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
    /// Config privat yt-dlp gagal dibuat — status Error sudah di-set, JANGAN
    /// fallback dengan menurunkan proxy/kredensial ke jalur yang berbeda.
    ConfigurationError,
}

/// v3.2.2: tandai unduhan gagal karena tool downloader tidak terpasang.
///
/// Dipisah dari `download()` supaya bisa di-unit test tanpa men-spawn
/// `yt-dlp --version`. Cabang `MissingTool` dulu menyetel `status = Error`
/// tetapi membiarkan `error_msg` KOSONG — komentarnya menyebut `crate::pkg`,
/// panggilannya tidak ada — sehingga GUI menampilkan kartu "GAGAL" tanpa teks
/// apa pun dan retry supervisor mengulang kegagalan yang sama dalam diam.
/// Pesan install mengikuti distro user (`pacman`/`apt`/`dnf`/…), sama seperti
/// `youtube.rs` dan `aria2.rs` — satu sumber di `crate::pkg`.
fn mark_missing_tool(info: &mut DownloadInfo, binary: &str, pkg: &str) {
    info.status = DownloadStatus::Error;
    info.error_msg = crate::pkg::missing_tool_msg(binary, pkg);
    info.speed = 0;
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
    // v3.2.8-fix: hasil NEGATIF tidak lagi di-cache permanen. Dulu `OnceLock`
    // menyimpan `false` selamanya — user yang memasang yt-dlp setelah unduhan
    // pertama gagal tetap melihat "yt-dlp tidak terinstall" sampai aplikasi
    // di-restart, terasa seperti "kadang berhasil kadang gagal" tergantung
    // urutan install vs. start app. Sekarang hanya keberhasilan yang di-cache;
    // kegagalan di-probe ulang tiap percobaan (biaya ~50–100 ms, dapat
    // diabaikan) sehingga instalasi baru langsung terdeteksi tanpa restart.
    static YTDLP_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let available = if let Some(true) = YTDLP_AVAILABLE.get().copied() {
        true
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
        if avail {
            let _ = YTDLP_AVAILABLE.set(true);
        }
        avail
    };

    if !available {
        // v2.3.1 (M1): dulu `Handle::current().block_on` dari konteks async —
        // panic di runtime Tokio; kini await langsung.
        let mut i = info.lock().await;
        if i.stop_requested() {
            return Outcome::Failed;
        }
        // Pesan install mengikuti distro user — lihat `mark_missing_tool`.
        mark_missing_tool(&mut i, "yt-dlp", "yt-dlp");

        let _ = tx.send(DownloadEvent::Error(i.clone()));
        return Outcome::MissingTool;
    }

    let (proxy_args, proxy_file) = match ytdlp_proxy_args(config) {
        Ok(value) => value,
        Err(msg) => {
            let mut i = info.lock().await;
            if !i.stop_requested() {
                i.status = DownloadStatus::Error;
                i.error_msg = msg;
                i.speed = 0;
                let _ = tx.send(DownloadEvent::Error(i.clone()));
            }
            return Outcome::ConfigurationError;
        }
    };
    let _proxy_file = PrivateFileGuard::new(proxy_file);

    let mut cmd = vec!["yt-dlp".to_string()];
    cmd.extend(quality_args(quality.as_deref()));
    // v2.10.5 (perf): fragmen HLS/DASH paralel (lihat youtube.rs).
    cmd.extend([
        "--concurrent-fragments".into(),
        conn_per_server(config.max_connections).to_string(),
    ]);
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
        config.retry_count.to_string(),
        "--fragment-retries".into(),
        "10".into(),
        "--retry-sleep".into(),
        "fragment:exp=1:1:5".into(),
        "--file-access-retries".into(),
        "3".into(),
        "--extractor-retries".into(),
        "3".into(),
        "--throttled-rate".into(),
        "100K".into(),
        "--merge-output-format".into(),
        merge_output_format(quality.as_deref()).into(),
        "--http-chunk-size".into(),
        "10M".into(),
        "--buffer-size".into(),
        "64K".into(),
    ]);

    // Cookies (dari cookies.txt / browser) + Referer & header kustom extension
    cmd.extend(cookie_args(&url));

    // v2.9.1: batas kecepatan total per unduhan hidup (pembagian M3 dari
    // engine) — sama seperti jalur YouTube; sebelumnya limit Pengaturan
    // tidak berlaku untuk yt-dlp.
    if !config.max_overall_speed.is_empty() && config.max_overall_speed != "0" {
        cmd.extend(["--limit-rate".into(), config.max_overall_speed.clone()]);
    }

    // v2.4.0 (D3): proxy juga untuk jalur resolver universal. URL proxy
    // berada di config privat; argv hanya membawa path config tersebut.
    cmd.extend(proxy_args);
    cmd.extend(network_args(config));
    for (k, v) in &headers {
        let k = k.replace(['\r', '\n'], "");
        let v = v.replace(['\r', '\n'], "");
        if !k.is_empty() && !v.is_empty() {
            cmd.push("--add-header".into());
            cmd.push(format!("{}:{}", k, v));
        }
    }
    // Jangan taruh signed URL di argv (`/proc/<pid>/cmdline`). yt-dlp membaca
    // satu URL dari stdin lewat batch-file=-.
    cmd.push(YTDLP_BATCH_FILE_STDIN.into());

    let ok = run_ytdlp_with_stdin(cmd, info.clone(), tx.clone(), Some(url)).await;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// v3.2.2: cabang "yt-dlp tidak terinstall" WAJIB meninggalkan pesan yang
    /// bisa dibaca user. Dulu hanya `status = Error` dengan `error_msg` kosong
    /// → GUI menampilkan kartu "GAGAL" tanpa teks apa pun, lalu retry
    /// supervisor mengulang kegagalan yang sama dalam diam.
    #[test]
    fn missing_tool_leaves_a_readable_error_message() {
        let mut info = DownloadInfo::new(
            "dl_x".into(),
            "https://example.com/page".into(),
            "page".into(),
            "/tmp".into(),
            Default::default(),
            None,
        );
        info.speed = 4096;
        info.status = DownloadStatus::Downloading;

        mark_missing_tool(&mut info, "yt-dlp", "yt-dlp");

        assert_eq!(info.status, DownloadStatus::Error);
        assert_eq!(info.speed, 0);
        assert_eq!(
            info.error_msg,
            crate::pkg::missing_tool_msg("yt-dlp", "yt-dlp"),
            "pesan harus datang dari crate::pkg — satu sumber dengan youtube.rs/aria2.rs"
        );
        assert!(info.error_msg.starts_with("yt-dlp tidak terinstall"));
    }
}
