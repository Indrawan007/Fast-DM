use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

const CHROME_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &Config,
) {
    // Resolve filename
    {
        let mut i = info.lock().await;
        i.status = DownloadStatus::Resolving;
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
    }

    resolve_filename(&info).await;

    // Check if cancelled during resolve
    {
        let i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }
    }

    // Build aria2c command
    let (cmd, input_file) = {
        let i = info.lock().await;
        build_aria2_cmd(&i, config)
    };

    // Update status
    {
        let mut i = info.lock().await;
        i.status = DownloadStatus::Downloading;
        let _ = tx.send(DownloadEvent::Progress(i.clone()));
    }

    tracing::info!("Downloading: {}", info.lock().await.filename);

    // Spawn aria2c
    let _result = tokio::task::spawn_blocking(move || {
        run_aria2c(cmd, info.clone(), tx.clone())
    })
    .await;

    // Cleanup input file
    if let Some(path) = input_file {
        let _ = std::fs::remove_file(path);
    }
}

fn build_aria2_cmd(info: &DownloadInfo, config: &Config) -> (Vec<String>, Option<String>) {
    // Write URL to input file (handles long URLs)
    let tmp_dir = std::env::temp_dir().join("fast-dm");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let input_path = tmp_dir.join(format!("{}.txt", info.id));

    let mut input_content = format!("{}\n", info.url);
    input_content += &format!("  dir={}\n", info.save_dir);
    input_content += &format!("  out={}\n", info.filename);
    input_content += "  continue=true\n";
    input_content += "  allow-overwrite=true\n";
    input_content += "  auto-file-renaming=false\n";

    let _ = std::fs::write(&input_path, &input_content);

    let cmd = vec![
        "aria2c".into(),
        format!("--input-file={}", input_path.display()),
        format!("--max-connection-per-server={}", config.max_connections),
        format!("--split={}", config.max_connections),
        "--min-split-size=1M".into(),
        "--piece-length=1M".into(),
        format!("--timeout={}", config.timeout),
        "--connect-timeout=15".into(),
        "--lowest-speed-limit=1K".into(),
        format!("--max-tries={}", config.retry_count),
        format!("--retry-wait={}", config.retry_wait),
        "--max-resume-failure-tries=5".into(),
        format!("--disk-cache={}", config.disk_cache_size),
        format!("--file-allocation={}", config.file_allocation),
        format!("--user-agent={}", CHROME_UA),
        "--console-log-level=notice".into(),
        "--summary-interval=1".into(),
        "--human-readable=false".into(),
        "--show-console-readout=true".into(),
        "--download-result=full".into(),
        format!("--max-overall-download-limit={}", config.max_overall_speed),
        "--check-integrity=false".into(),
        "--check-certificate=false".into(),
    ];

    (cmd, Some(input_path.to_string_lossy().to_string()))
}

fn run_aria2c(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) {
    let re_progress = Regex::new(r"(\d+)B/(\d+)B\((\d+)%\)").unwrap();
    let re_speed = Regex::new(r"DL:(\d+)").unwrap();
    let re_cn = Regex::new(r"CN:(\d+)").unwrap();
    let re_eta = Regex::new(r"ETA:(\S+)").unwrap();

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
                i.error_msg = format!("aria2c: {}", e);
                let _ = tx.send(DownloadEvent::Error(i.clone()));
            });
            return;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut last_update = Instant::now();

    for line in reader.lines().flatten() {
        // Check cancel/pause
        let rt = tokio::runtime::Handle::current();
        let status = rt.block_on(async { info.lock().await.status });

        if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            let _ = child.kill();
            return;
        }

        // Parse progress
        if let Some(m) = re_progress.captures(&line) {
            let downloaded: u64 = m[1].parse().unwrap_or(0);
            let total: u64 = m[2].parse().unwrap_or(0);
            let progress: f64 = m[3].parse().unwrap_or(0.0);

            let speed = re_speed
                .captures(&line)
                .and_then(|m| m[1].parse().ok())
                .unwrap_or(0u64);

            let connections = re_cn
                .captures(&line)
                .and_then(|m| m[1].parse().ok())
                .unwrap_or(0u8);

            let eta = re_eta
                .captures(&line)
                .map(|m| parse_eta(&m[1]))
                .unwrap_or(0);

            // Throttle updates to 5fps
            if last_update.elapsed().as_millis() >= 200 {
                rt.block_on(async {
                    let mut i = info.lock().await;
                    i.downloaded = downloaded;
                    i.total_size = total;
                    i.progress = progress;
                    i.speed = speed;
                    i.connections = connections;
                    i.eta = eta;
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
            let _ = tx.send(DownloadEvent::Completed(i.clone()));
        } else {
            i.status = DownloadStatus::Error;
            i.error_msg = format!("aria2c exit code: {}", exit_code);
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
        }
    });
}

fn parse_eta(s: &str) -> u64 {
    let mut total = 0u64;
    let re_h = Regex::new(r"(\d+)h").unwrap();
    let re_m = Regex::new(r"(\d+)m").unwrap();
    let re_s = Regex::new(r"(\d+)s").unwrap();

    if let Some(m) = re_h.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0) * 3600;
    }
    if let Some(m) = re_m.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0) * 60;
    }
    if let Some(m) = re_s.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0);
    }
    total
}

async fn resolve_filename(info: &Arc<Mutex<DownloadInfo>>) {
    let url = { info.lock().await.url.clone() };

    let client = reqwest::Client::builder()
        .user_agent(CHROME_UA)
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // Try HEAD request with range
    if let Ok(resp) = client
        .head(&url)
        .header("Range", "bytes=0-0")
        .send()
        .await
    {
        let mut i = info.lock().await;

        // Content-Disposition
        if let Some(cd) = resp.headers().get("content-disposition") {
            if let Ok(cd_str) = cd.to_str() {
                if let Some(name) = parse_content_disposition(cd_str) {
                    i.filename = super::sanitize_filename(&name);
                }
            }
        }

        // Content-Length / Content-Range
        if let Some(cr) = resp.headers().get("content-range") {
            if let Ok(cr_str) = cr.to_str() {
                let re = Regex::new(r"/(\d+)").unwrap();
                if let Some(m) = re.captures(cr_str) {
                    i.total_size = m[1].parse().unwrap_or(0);
                }
            }
        } else if let Some(cl) = resp.headers().get("content-length") {
            if let Ok(cl_str) = cl.to_str() {
                i.total_size = cl_str.parse().unwrap_or(0);
            }
        }
    }
}

fn parse_content_disposition(cd: &str) -> Option<String> {
    // filename*=UTF-8''name
    let re1 = Regex::new(r"filename\*\s*=\s*(?:UTF-8|utf-8)?'[^']*'(.+?)(?:\s*;|$)").unwrap();
    if let Some(m) = re1.captures(cd) {
        let decoded = urlencoding::decode(&m[1]).unwrap_or_default();
        return Some(decoded.to_string());
    }

    // filename="name"
    let re2 = Regex::new(r#"filename\s*=\s*"([^"]+)""#).unwrap();
    if let Some(m) = re2.captures(cd) {
        return Some(m[1].to_string());
    }

    // filename=name
    let re3 = Regex::new(r"filename\s*=\s*([^\s;]+)").unwrap();
    if let Some(m) = re3.captures(cd) {
        return Some(m[1].trim_matches('"').to_string());
    }

    None
}
