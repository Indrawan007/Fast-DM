use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

static RE_YT_WATCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/watch\?v=[\w-]+").unwrap());
static RE_YT_SHORTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/shorts/[\w-]+").unwrap());
static RE_YT_BE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:https?://)?youtu\.be/[\w-]+").unwrap());
static RE_YT_MUSIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:https?://)?music\.youtube\.com/watch\?v=[\w-]+").unwrap());
static RE_YTDLP_PROGRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)\s+ETA\s+(\S+)").unwrap());
static RE_YTDLP_PROGRESS2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[download\]\s+(\d+\.?\d*)%").unwrap());
static RE_YTDLP_DEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[download\]\s+Destination:\s+(.+)").unwrap());
static RE_YTDLP_MERGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[Merger\]|\[ffmpeg\]|Merging").unwrap());
static RE_SPEED_PARSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\d.]+)\s*(\S+)").unwrap());

pub fn is_youtube_url(url: &str) -> bool {
    RE_YT_WATCH.is_match(url)
        || RE_YT_SHORTS.is_match(url)
        || RE_YT_BE.is_match(url)
        || RE_YT_MUSIC.is_match(url)
}

/// Detect browser for cookies — gunakan nama support yt-dlp
fn detect_browser() -> Option<&'static str> {
    let home = dirs::home_dir()?;
    let candidates = [
        ("chrome",   ".config/google-chrome"),
        ("chromium", ".config/chromium"),
        ("edge",     ".config/microsoft-edge"),
        ("firefox",  ".mozilla/firefox"),
        ("brave",    ".config/BraveSoftware/Brave-Browser"),
        ("opera",    ".config/opera"),
        ("vivaldi",  ".config/vivaldi"),
        // Thorium menggunakan format Chromium
        ("chromium", ".config/thorium"),
    ];

    for (name, path) in &candidates {
        if home.join(path).is_dir() {
            return Some(name);
        }
    }
    None
}

pub(crate) fn cookie_args() -> Vec<String> {
    let cookies_file = Config::config_dir().join("cookies.txt");

    // Use cookies.txt if fresh
    if cookies_file.exists() {
        if let Ok(meta) = std::fs::metadata(&cookies_file) {
            if let Ok(modified) = meta.modified() {
                if modified.elapsed().unwrap_or_default().as_secs() < 7200 {
                    if meta.len() > 100 {
                        return vec![
                            "--cookies".into(),
                            cookies_file.to_string_lossy().to_string(),
                        ];
                    }
                }
            }
        }
    }

    // Use browser cookies
    if let Some(browser) = detect_browser() {
        return vec!["--cookies-from-browser".into(), browser.into()];
    }

    vec![]
}

pub(crate) fn output_template(save_dir: &str, filename: &str) -> String {
    let f = filename.trim();
    if f.is_empty() || f.starts_with("download_") {
        return format!("{}/%(title)s.%(ext)s", save_dir);
    }
    if f.contains('.') {
        return format!("{}/{}", save_dir, f);
    }
    format!("{}/{}.%(ext)s", save_dir, f)
}

/// Mapping pilihan kualitas dari extension → argumen yt-dlp
pub(crate) fn quality_args(quality: Option<&str>) -> Vec<String> {
    match quality {
        Some(q) if q.ends_with('p') && q[..q.len() - 1].chars().all(|c| c.is_ascii_digit()) => {
            let h = &q[..q.len() - 1];
            vec![
                "--format".into(),
                format!("bestvideo[height<={}]+bestaudio/best[height<={}]/best", h, h),
            ]
        }
        Some("audio_best") => vec![
            "--format".into(), "bestaudio/best".into(),
            "--extract-audio".into(),
            "--audio-format".into(), "m4a".into(),
        ],
        Some("audio_mp3") => vec![
            "--format".into(), "bestaudio/best".into(),
            "--extract-audio".into(),
            "--audio-format".into(), "mp3".into(),
            "--audio-quality".into(), "0".into(),
        ],
        _ => vec![
            "--format".into(),
            "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best".into(),
        ],
    }
}

pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    _config: &Config,
) {
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
        "--embed-thumbnail".into(),
        "--embed-metadata".into(),
    ]);

    cmd.extend(cookie_args());

    // Header kustom dari browser extension (mis. Referer)
    for (k, v) in &headers {
        let k = k.replace(['\r', '\n'], "");
        let v = v.replace(['\r', '\n'], "");
        if !k.is_empty() && !v.is_empty() {
            cmd.push("--add-header".into());
            cmd.push(format!("{}:{}", k, v));
        }
    }

    cmd.push(url);

    let info_clone = info.clone();
    let tx_clone = tx.clone();

    tokio::task::spawn_blocking(move || {
        run_ytdlp(cmd, info_clone, tx_clone);
    })
    .await
    .ok();
}

pub(crate) fn run_ytdlp(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) -> bool {
    let mut child = match Command::new(&cmd[0]).args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "yt-dlp tidak terinstall — jalankan: sudo apt install yt-dlp".to_string()
            } else {
                format!("yt-dlp: {}", e)
            };
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let mut i = info.lock().await;
                i.status = DownloadStatus::Error;
                i.error_msg = msg;
                let _ = tx.send(DownloadEvent::Error(i.clone()));
            });
            return false;
        }
    };

    // Simpan PID supaya bisa di-kill saat app ditutup (anti orphan)
    {
        let rt = tokio::runtime::Handle::current();
        let pid = child.id();
        rt.block_on(async { info.lock().await.pid = Some(pid); });
    }

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Baca stderr di thread terpisah agar tidak deadlock
    let stderr_buf = Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_thread = std::thread::Builder::new()
        .name("ytdlp-stderr".into())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                let mut buf = stderr_buf_clone.lock().unwrap();
                buf.push_str(&line);
                buf.push('\n');
            }
        })
        .ok();

    let reader = BufReader::new(stdout);
    let mut last_update = Instant::now();

    for line in reader.lines().flatten() {
        let rt = tokio::runtime::Handle::current();
        let status = rt.block_on(async { info.lock().await.status });

        if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            let _ = child.kill();
            // Reap the child so it does not linger as a zombie process
            let _ = child.wait();
            rt.block_on(async { info.lock().await.pid = None; });
            return false;
        }

        if let Some(m) = RE_YTDLP_DEST.captures(&line) {
            let filename = std::path::Path::new(m.get(1).unwrap().as_str())
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            rt.block_on(async {
                info.lock().await.filename = filename;
            });
        }

        if RE_YTDLP_MERGE.is_match(&line) {
            rt.block_on(async {
                let mut i = info.lock().await;
                i.progress = 99.0;
                i.error_msg = "Merging video + audio...".into();
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
            });
            continue;
        }

        let progress_match = RE_YTDLP_PROGRESS.captures(&line)
            .or_else(|| RE_YTDLP_PROGRESS2.captures(&line));

        if let Some(m) = progress_match {
            if last_update.elapsed().as_millis() >= 250 {
                let pct: f64 = m[1].parse().unwrap_or(0.0);
                let speed_str = m.get(3).map(|s| s.as_str()).unwrap_or("");
                let speed = parse_speed(speed_str);
                // Size (grup 2) & ETA (grup 4) hanya ada di format progress lengkap
                let total = m.get(2).map(|s| parse_speed(s.as_str())).unwrap_or(0);
                let eta = m.get(4).map(|s| parse_eta_hms(s.as_str())).unwrap_or(0);

                rt.block_on(async {
                    let mut i = info.lock().await;
                    i.progress = pct;
                    i.speed = speed;
                    if total > 0 {
                        i.total_size = total;
                        i.downloaded = (pct / 100.0 * total as f64) as u64;
                    }
                    i.eta = eta;
                    i.error_msg.clear();
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                });
                last_update = Instant::now();
            }
        }
    }

    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }

    let exit_code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
    let err_detail = stderr_buf.lock().unwrap().clone();

    let rt = tokio::runtime::Handle::current();
    let ok = rt.block_on(async {
        let mut i = info.lock().await;
        i.pid = None;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return false;
        }

        if exit_code == 0 {
            i.status = DownloadStatus::Completed;
            i.progress = 100.0;
            i.speed = 0;
            if i.total_size > 0 {
                i.downloaded = i.total_size;
            }
            i.error_msg.clear();
            let _ = tx.send(DownloadEvent::Completed(i.clone()));
            true
        } else {
            i.status = DownloadStatus::Error;
            let detail = if err_detail.is_empty() { String::new() } else { format!("\n{}", err_detail.trim()) };
            i.error_msg = format!("yt-dlp exit code: {}{}", exit_code, detail);
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
            false
        }
    });
    ok
}

/// Parse ETA yt-dlp ("MM:SS" atau "HH:MM:SS") → detik
fn parse_eta_hms(s: &str) -> u64 {
    let nums: Vec<u64> = s
        .split(':')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    match nums.len() {
        3 => nums[0] * 3600 + nums[1] * 60 + nums[2],
        2 => nums[0] * 60 + nums[1],
        1 => nums[0],
        _ => 0,
    }
}

fn parse_speed(s: &str) -> u64 {
    if let Some(m) = RE_SPEED_PARSE.captures(s) {
        let val: f64 = m[1].parse().unwrap_or(0.0);
        let unit = m[2].to_lowercase();
        if unit.contains("gib") { return (val * 1073741824.0) as u64; }
        if unit.contains("mib") { return (val * 1048576.0) as u64; }
        if unit.contains("kib") { return (val * 1024.0) as u64; }
        return val as u64;
    }
    0
}
