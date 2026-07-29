use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

const YT_PATTERNS: &[&str] = &[
    r"(?:https?://)?(?:www\.)?youtube\.com/watch\?v=[\w-]+",
    r"(?:https?://)?(?:www\.)?youtube\.com/shorts/[\w-]+",
    r"(?:https?://)?youtu\.be/[\w-]+",
    r"(?:https?://)?music\.youtube\.com/watch\?v=[\w-]+",
];

pub fn is_youtube_url(url: &str) -> bool {
    YT_PATTERNS.iter().any(|p| {
        Regex::new(p).map(|re| re.is_match(url)).unwrap_or(false)
    })
}

/// Detect browser for cookies
fn detect_browser() -> Option<&'static str> {
    let home = dirs::home_dir()?;
    let candidates = [
        ("chromium", ".config/thorium"),
        ("chromium", ".config/chromium"),
        ("chrome",   ".config/google-chrome"),
        ("brave",    ".config/BraveSoftware/Brave-Browser"),
        ("edge",     ".config/microsoft-edge"),
        ("vivaldi",  ".config/vivaldi"),
        ("opera",    ".config/opera"),
        ("firefox",  ".mozilla/firefox"),
    ];

    for (name, path) in &candidates {
        if home.join(path).is_dir() {
            return Some(name);
        }
    }
    None
}

fn cookie_args() -> Vec<String> {
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

pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    _config: &Config,
) {
    let (url, save_dir) = {
        let mut i = info.lock().await;
        i.status = DownloadStatus::Downloading;
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
        (i.url.clone(), i.save_dir.clone())
    };

    let mut cmd = vec![
        "yt-dlp".to_string(),
        "--format".into(),
        "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best".into(),
        "--output".into(),
        format!("{}/%(title)s.%(ext)s", save_dir),
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
    ];

    cmd.extend(cookie_args());
    cmd.push(url);

    let info_clone = info.clone();
    let tx_clone = tx.clone();

    tokio::task::spawn_blocking(move || {
        run_ytdlp(cmd, info_clone, tx_clone);
    })
    .await
    .ok();
}

fn run_ytdlp(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) {
    let re_progress = Regex::new(
        r"\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)\s+ETA\s+(\S+)"
    ).unwrap();
    let re_progress2 = Regex::new(r"\[download\]\s+(\d+\.?\d*)%").unwrap();
    let re_dest = Regex::new(r"\[download\]\s+Destination:\s+(.+)").unwrap();
    let re_merge = Regex::new(r"\[Merger\]|\[ffmpeg\]|Merging").unwrap();

    let child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let mut i = info.lock().await;
                i.status = DownloadStatus::Error;
                i.error_msg = format!("yt-dlp: {}", e);
                let _ = tx.send(DownloadEvent::Error(i.clone()));
            });
            return;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut last_update = Instant::now();

    for line in reader.lines().flatten() {
        let rt = tokio::runtime::Handle::current();
        let status = rt.block_on(async { info.lock().await.status });

        if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            let _ = child.kill();
            return;
        }

        // Destination filename
        if let Some(m) = re_dest.captures(&line) {
            let filename = std::path::Path::new(m.get(1).unwrap().as_str())
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            rt.block_on(async {
                info.lock().await.filename = filename;
            });
        }

        // Merging
        if re_merge.is_match(&line) {
            rt.block_on(async {
                let mut i = info.lock().await;
                i.progress = 99.0;
                i.error_msg = "Merging video + audio...".into();
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
            });
            continue;
        }

        // Progress
        let progress_match = re_progress.captures(&line)
            .or_else(|| re_progress2.captures(&line));

        if let Some(m) = progress_match {
            if last_update.elapsed().as_millis() >= 250 {
                let pct: f64 = m[1].parse().unwrap_or(0.0);

                let speed_str = m.get(3).map(|s| s.as_str()).unwrap_or("");
                let speed = parse_speed(speed_str);

                rt.block_on(async {
                    let mut i = info.lock().await;
                    i.progress = pct;
                    i.speed = speed;
                    i.error_msg.clear();
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                });
                last_update = Instant::now();
            }
        }
    }

    let exit_code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    let rt = tokio::runtime::Handle::current();
    rt.block_on(async {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }

        if exit_code == 0 {
            i.status = DownloadStatus::Completed;
            i.progress = 100.0;
            i.speed = 0;
            i.error_msg.clear();
            let _ = tx.send(DownloadEvent::Completed(i.clone()));
        } else {
            i.status = DownloadStatus::Error;
            i.error_msg = format!("yt-dlp exit code: {}", exit_code);
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
        }
    });
}

fn parse_speed(s: &str) -> u64 {
    let re = Regex::new(r"([\d.]+)\s*(\S+)").unwrap();
    if let Some(m) = re.captures(s) {
        let val: f64 = m[1].parse().unwrap_or(0.0);
        let unit = m[2].to_lowercase();
        if unit.contains("gib") { return (val * 1073741824.0) as u64; }
        if unit.contains("mib") { return (val * 1048576.0) as u64; }
        if unit.contains("kib") { return (val * 1024.0) as u64; }
        return val as u64;
    }
    0
}
