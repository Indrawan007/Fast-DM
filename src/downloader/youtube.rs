use super::types::*;
use crate::config::Config;
use crate::static_regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

static RE_YT_WATCH: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("yt_watch", r"(?:https?://)?(?:www\.)?youtube\.com/watch\?v=[\w-]+"));
static RE_YT_SHORTS: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("yt_shorts", r"(?:https?://)?(?:www\.)?youtube\.com/shorts/[\w-]+"));
static RE_YT_BE: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("yt_be", r"(?:https?://)?youtu\.be/[\w-]+"));
static RE_YT_MUSIC: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("yt_music", r"(?:https?://)?music\.youtube\.com/watch\?v=[\w-]+"));
static RE_YTDLP_PROGRESS: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("ytdlp_progress", r"\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)\s+ETA\s+(\S+)"));
static RE_YTDLP_PROGRESS2: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("ytdlp_progress2", r"\[download\]\s+(\d+\.?\d*)%"));
static RE_YTDLP_DEST: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("ytdlp_destination", r"\[download\]\s+Destination:\s+(.+)"));
static RE_YTDLP_MERGE: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("ytdlp_merge", r"\[Merger\]|\[ffmpeg\]|Merging"));
static RE_SPEED_PARSE: LazyLock<regex::Regex> = LazyLock::new(|| static_regex!("ytdlp_speed_parse", r"([\d.]+)\s*(\S+)"));

pub fn is_youtube_url(url: &str) -> bool {
    RE_YT_WATCH.is_match(url)
        || RE_YT_SHORTS.is_match(url)
        || RE_YT_BE.is_match(url)
        || RE_YT_MUSIC.is_match(url)
}

/// Deteksi browser untuk cookies — gunakan nama support yt-dlp.
/// B17: browser DEFAULT sistem (xdg-settings) diprioritaskan — cookie login
/// user biasanya di browser default, bukan browser pertama yang kebetulan
/// ter-install.
/// Di-cache per-sesi: mendeteksi browser (spawn xdg-settings + stat folder)
/// tidak perlu diulang untuk setiap download.
static BROWSER: LazyLock<Option<&'static str>> =
    LazyLock::new(detect_browser_inner);

fn detect_browser() -> Option<&'static str> {
    *BROWSER
}

fn detect_browser_inner() -> Option<&'static str> {
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

    if let Ok(out) = std::process::Command::new("xdg-settings")     // B17
        .args(["get", "default-web-browser"])
        .output()
    {
        if out.status.success() {
            let desktop = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
            if let Some(name) = desktop_to_browser(&desktop) {
                if let Some(&(_, path)) = candidates.iter().find(|c| c.0 == name) {
                    if home.join(path).is_dir() {
                        return Some(name);
                    }
                }
            }
        }
    }


    for (name, path) in &candidates {
        if home.join(path).is_dir() {
            return Some(*name);
        }
    }
    None
}

/// Nama file .desktop browser default → nama browser yt-dlp
fn desktop_to_browser(desktop: &str) -> Option<&'static str> {
    if desktop.contains("google-chrome") { return Some("chrome"); }
    if desktop.contains("microsoft-edge") { return Some("edge"); }
    if desktop.contains("thorium") { return Some("chromium"); }
    if desktop.contains("chromium") { return Some("chromium"); }
    if desktop.contains("firefox") { return Some("firefox"); }
    if desktop.contains("brave") { return Some("brave"); }
    if desktop.contains("opera") { return Some("opera"); }
    if desktop.contains("vivaldi") { return Some("vivaldi"); }
    None
}

pub(crate) fn cookie_args(url: &str) -> Vec<String> {
    // B7: cookies per-domain dari extension (fresh < 2 jam) → pakai file itu.
    // Pencarian naik ke domain induk (sub.example.com → example.com) karena
    // extension menyimpan cookies memakai host halaman, sedangkan file video
    // kadang ada di subdomain CDN yang berbeda.
    if let Some(host) = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
    {
        if let Some(cookies_file) = Config::find_cookies_file(&host) {
            if is_fresh_cookie_file(&cookies_file) {
                return vec![
                    "--cookies".into(),
                    cookies_file.to_string_lossy().to_string(),
                ];
            }
        }
    }

    // Use browser cookies
    if let Some(browser) = detect_browser() {
        return vec!["--cookies-from-browser".into(), browser.into()];
    }

    vec![]
}

/// Cookie file ada (isi bukan hanya header), dan terakhir diubah < 2 jam lalu
fn is_fresh_cookie_file(path: &std::path::Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => {
            // Header Netscape = 29 byte; > 30 berarti minimal ada 1 cookie
            meta.len() > 30
                && meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|e| e.as_secs() < 7200)
        }
        Err(_) => false,
    }
}

pub(crate) fn output_template(save_dir: &str, filename: &str) -> String {
    let f = filename.trim();
    if f.is_empty() || f.starts_with("download_") {
        return format!("{}/%(title)s.%(ext)s", save_dir);
    }
    // B6: escape '%' → '%%' — nilai --output yt-dlp adalah template;
    // nama file hasil decode URL yang mengandung '%' akan salah parse.
    let f = f.replace('%', "%%");
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
    // Guard: user bisa pause/cancel di jeda sebelum child process lahir
    // (pid belum ada, jadi kill_child_pid tidak berdampak apa-apa). Tanpa guard,
    // status ditimpa Downloading dan download yang "dibatalkan" jalan terus.
    let (url, save_dir, headers, quality, filename) = {
        let mut i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
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

    cmd.extend(cookie_args(&url));

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
                // Batasi 16 KB (pertahankan yang terbaru) — situs bermasalah
                // bisa membanjiri stderr tanpa batas
                if buf.len() > 16 * 1024 {
                    let cut = buf.len() - 8 * 1024;
                    let drop = buf[..cut]
                        .find(|c: char| c == '\n')
                        .map(|i| i + 1)
                        .unwrap_or(cut);
                    buf.drain(..drop);
                }
            }
        })
        .ok();

    let reader = BufReader::new(stdout);
    let mut last_update = Instant::now();

    for line in reader.lines().flatten() {
        let rt = tokio::runtime::Handle::current();
        let status = rt.block_on(async { info.lock().await.status });

        if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            // B8: Paused → SIGTERM sudah dikirim pause_download(); tunggu proses
            // berhenti rapi (file .part tetap bisa di-resume). Cancelled → kill.
            if status == DownloadStatus::Cancelled {
                let _ = child.kill();
            }
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
       // Format lama/alternatif: "2.5M/s", "500K/s", "1.2G/s"
        if unit.starts_with('g') { return (val * 1073741824.0) as u64; }
        if unit.starts_with('m') { return (val * 1048576.0) as u64; }
        if unit.starts_with('k') { return (val * 1024.0) as u64; }
        return val as u64;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_youtube_url ──

    #[test]
    fn is_youtube_url_positive() {
        let urls = [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=abc123",
            "https://www.youtube.com/shorts/xyz",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=abc",
            "http://www.youtube.com/watch?v=abc", // http juga
        ];
        for url in urls {
            assert!(is_youtube_url(url), "{} harus dianggap YouTube", url);
        }
    }

    #[test]
    fn is_youtube_url_negative() {
        let urls = [
            "https://example.com/watch?v=abc",
            "https://vimeo.com/12345",
            "https://tiktok.com/@user/video/123",
            "https://example.com/file.mp4",
            "not a url",
            "",
        ];
        for url in urls {
            assert!(!is_youtube_url(url), "{} BUKAN YouTube", url);
        }
    }

    // ── parse_eta_hms ──

    #[test]
    fn parse_eta_hms_seconds() {
        // Single number (jarang, tapi handled)
        assert_eq!(parse_eta_hms("30"), 30);
    }

    #[test]
    fn parse_eta_hms_mm_ss() {
        assert_eq!(parse_eta_hms("4:51"), 4 * 60 + 51);
        assert_eq!(parse_eta_hms("10:00"), 600);
        assert_eq!(parse_eta_hms("00:30"), 30);
    }

    #[test]
    fn parse_eta_hms_hh_mm_ss() {
        assert_eq!(parse_eta_hms("01:23:45"), 3600 + 23 * 60 + 45);
        assert_eq!(parse_eta_hms("00:00:00"), 0);
    }

    #[test]
    fn parse_eta_hms_invalid() {
        assert_eq!(parse_eta_hms(""), 0);
        assert_eq!(parse_eta_hms("abc"), 0);
        assert_eq!(parse_eta_hms("1:2:3:4"), 0); // lebih dari 3 segmen
    }

    // ── parse_speed ──

    #[test]
    fn parse_speed_mib_s() {
        assert_eq!(parse_speed("2.5MiB/s"), (2.5 * 1_048_576.0) as u64);
        assert_eq!(parse_speed("1MiB/s"), 1_048_576);
    }

    #[test]
    fn parse_speed_short_units() {
        // yt-dlp versi lama pakai "M/s", "K/s"
        assert_eq!(parse_speed("500K/s"), 500 * 1024);
        assert_eq!(parse_speed("2M/s"), 2 * 1_048_576);
    }

    #[test]
    fn parse_speed_kib() {
        assert_eq!(parse_speed("300KiB/s"), 300 * 1024);
    }

    #[test]
    fn parse_speed_invalid() {
        assert_eq!(parse_speed(""), 0);
        assert_eq!(parse_speed("abc"), 0);
    }

    // ── output_template ──

    #[test]
    fn output_template_with_explicit_filename() {
        // File dengan ekstensi → pakai sebagai out, escape %
        assert_eq!(
            output_template("/tmp", "video.mp4"),
            "/tmp/video.mp4"
        );
    }

    #[test]
    fn output_template_escape_percent() {
        // B6: nama file dengan '%' di-escape jadi '%%' agar yt-dlp tidak
        // salah parse sebagai template
        assert_eq!(
            output_template("/tmp", "100%done.mp4"),
            "/tmp/100%%done.mp4"
        );
    }

    #[test]
    fn output_template_no_extension_adds_placeholder() {
        // Tanpa ekstensi → tambahkan %(ext)s
        assert_eq!(output_template("/tmp", "myvideo"), "/tmp/myvideo.%(ext)s");
    }

    #[test]
    fn output_template_generic_uses_title() {
        // download_xxx → pakai %(title)s.%(ext)s
        assert_eq!(
            output_template("/tmp", "download_12345"),
            "/tmp/%(title)s.%(ext)s"
        );
        assert_eq!(output_template("/tmp", ""), "/tmp/%(title)s.%(ext)s");
    }

    // ── quality_args ──

    #[test]
    fn quality_args_resolution_p() {
        let args = quality_args(Some("1080p"));
        assert!(args.contains(&"--format".to_string()));
        let fmt = &args[1];
        assert!(fmt.contains("height<=1080"));
        assert!(fmt.contains("bestaudio"));
    }

    #[test]
    fn quality_args_4k() {
        let args = quality_args(Some("2160p"));
        assert!(args[1].contains("height<=2160"));
    }

    #[test]
    fn quality_args_audio_best() {
        let args = quality_args(Some("audio_best"));
        assert!(args.contains(&"--extract-audio".to_string()));
        assert!(args.contains(&"m4a".to_string()));
    }

    #[test]
    fn quality_args_audio_mp3() {
        let args = quality_args(Some("audio_mp3"));
        assert!(args.contains(&"--extract-audio".to_string()));
        assert!(args.contains(&"mp3".to_string()));
        assert!(args.contains(&"--audio-quality".to_string()));
        assert!(args.contains(&"0".to_string())); // best quality
    }

    #[test]
    fn quality_args_default() {
        // None atau unknown → best MP4 default
        let args = quality_args(None);
        assert!(args.contains(&"--format".to_string()));
        assert!(args[1].contains("mp4"));

        let args = quality_args(Some("unknown"));
        assert!(args[1].contains("mp4"));
    }

    #[test]
    fn quality_args_non_numeric_p_ignored() {
        // "abc" tidak berakhir dengan "p" digit → default
        let args = quality_args(Some("high"));
        assert!(args[1].contains("mp4")); // default fallback
    }

    // ── desktop_to_browser ──

    #[test]
    fn desktop_to_browser_known_browsers() {
        assert_eq!(desktop_to_browser("google-chrome.desktop"), Some("chrome"));
        assert_eq!(desktop_to_browser("chromium.desktop"), Some("chromium"));
        assert_eq!(desktop_to_browser("firefox.desktop"), Some("firefox"));
        assert_eq!(desktop_to_browser("brave-browser.desktop"), Some("brave"));
        assert_eq!(desktop_to_browser("opera.desktop"), Some("opera"));
        assert_eq!(desktop_to_browser("vivaldi.desktop"), Some("vivaldi"));
        assert_eq!(
            desktop_to_browser("microsoft-edge.desktop"),
            Some("edge")
        );
    }

    #[test]
    fn desktop_to_browser_unknown() {
        assert_eq!(desktop_to_browser("libreoffice.desktop"), None);
        assert_eq!(desktop_to_browser(""), None);
    }
}

