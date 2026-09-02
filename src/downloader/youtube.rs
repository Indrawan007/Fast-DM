use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

static RE_YTDLP_PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)\s+ETA\s+(\S+)").unwrap()
});
static RE_YTDLP_PROGRESS2: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[download\]\s+(\d+\.?\d*)%").unwrap());
static RE_YTDLP_DEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[download\]\s+Destination:\s+(.+)").unwrap());
static RE_YTDLP_MERGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[Merger\]|\[ffmpeg\]|Merging").unwrap());
static RE_SPEED_PARSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\d.]+)\s*(\S+)").unwrap());

/// v2.3.0 (M6): deteksi host-based menggantikan regex lama yang menuntut
/// `?v=` menjadi parameter pertama (`watch?app=desktop&v=…` dulu terlewat)
/// dan tidak mengenal /live/, /embed/, /v/.
pub fn is_youtube_url(url: &str) -> bool {
    let s = url.trim();
    let candidate = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let Ok(u) = url::Url::parse(&candidate) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    let host = u
        .host_str()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    // host dievaluasi EKSPLISIT — "notyoutube.com" / "youtube.com.evil.test"
    // tidak boleh ikut.
    let path = u.path().to_string();
    let has_v = u.query_pairs().any(|(k, v)| k == "v" && !v.is_empty());
    let seg = |p: &str, i: usize| p.split('/').nth(i).map_or(false, |x| !x.is_empty());

    match host.as_str() {
        "youtu.be" => seg(&path, 1),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            ((path == "/watch" || path.starts_with("/watch/")) && has_v)
                || (path.starts_with("/shorts/") && seg(&path, 2))
                || (path.starts_with("/live/") && seg(&path, 2))
                || (path.starts_with("/embed/") && seg(&path, 2))
                || (path.starts_with("/v/") && seg(&path, 2))
        }
        "music.youtube.com" => (path == "/watch" || path.starts_with("/watch/")) && has_v,
        _ => false,
    }
}

/// Deteksi browser untuk cookies — gunakan nama support yt-dlp.
/// B17: browser DEFAULT sistem (xdg-settings) diprioritaskan — cookie login
/// user biasanya di browser default, bukan browser pertama yang kebetulan
/// ter-install.
/// Di-cache per-sesi: mendeteksi browser (spawn xdg-settings + stat folder)
/// tidak perlu diulang untuk setiap download.
static BROWSER: LazyLock<Option<&'static str>> = LazyLock::new(detect_browser_inner);

fn detect_browser() -> Option<&'static str> {
    *BROWSER
}

fn detect_browser_inner() -> Option<&'static str> {
    let home = dirs::home_dir()?;
    let candidates = [
        ("chrome", ".config/google-chrome"),
        ("chromium", ".config/chromium"),
        ("edge", ".config/microsoft-edge"),
        ("firefox", ".mozilla/firefox"),
        ("brave", ".config/BraveSoftware/Brave-Browser"),
        ("opera", ".config/opera"),
        ("vivaldi", ".config/vivaldi"),
        // Thorium menggunakan format Chromium
        ("chromium", ".config/thorium"),
    ];

    if let Ok(out) = std::process::Command::new("xdg-settings") // B17
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
    if desktop.contains("google-chrome") {
        return Some("chrome");
    }
    if desktop.contains("microsoft-edge") {
        return Some("edge");
    }
    if desktop.contains("thorium") {
        return Some("chromium");
    }
    if desktop.contains("chromium") {
        return Some("chromium");
    }
    if desktop.contains("firefox") {
        return Some("firefox");
    }
    if desktop.contains("brave") {
        return Some("brave");
    }
    if desktop.contains("opera") {
        return Some("opera");
    }
    if desktop.contains("vivaldi") {
        return Some("vivaldi");
    }
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
                format!(
                    "bestvideo[height<={}]+bestaudio/best[height<={}]/best",
                    h, h
                ),
            ]
        }
        Some("audio_best") => vec![
            "--format".into(),
            "bestaudio/best".into(),
            "--extract-audio".into(),
            "--audio-format".into(),
            "m4a".into(),
        ],
        Some("audio_mp3") => vec![
            "--format".into(),
            "bestaudio/best".into(),
            "--extract-audio".into(),
            "--audio-format".into(),
            "mp3".into(),
            "--audio-quality".into(),
            "0".into(),
        ],
        // v2.6.0 (D6): id format NYATA dari yt-dlp ("137", "137+140") —
        // diteruskan sebagai selector, dengan fallback /best agar tetap jalan
        // bila id tidak tersedia saat eksekusi (mis. dialog basi).
        Some(q) if looks_like_format_id(q) => {
            vec!["--format".into(), format!("{q}/best")]
        }
        _ => vec![
            "--format".into(),
            "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best".into(),
        ],
    }
}

/// v2.6.0 (D6): token terlihat seperti id/selector format yt-dlp — tanpa
/// whitespace, panjang terbatas, dan karakter ter-batasi whitelist.
/// (Command API tidak melewati shell, tapi pembatasan ini defense-in-depth
/// karena string bisa berasal dari data halaman.)
fn looks_like_format_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        // id format yt-dlp selalu mengandung digit ("137", "137+140",
        // "251"); kata bebas ("unknown", "high") bukan id → default teraman,
        // bukan "--format unknown/best".
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '+' | '-' | '.' | '_' | '*' | '[' | ']' | '(' | ')' | '>' | '<' | '^'
                        | '&' | '|' | '/' | ',' | '=' | '!'
                )
        })
}

/// v2.6.0 (D6): satu opsi pada dialog kualitas. `id` = preset ("1080p",
/// "audio_mp3", "best_mp4") ATAU id format nyata dari yt-dlp ("137",
/// "137+140") — `quality_args()` memetakan keduanya.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatOption {
    pub id: String,
    pub label: String,
    pub desc: String,
}

#[derive(serde::Deserialize)]
struct YtFormatJson {
    #[serde(default)]
    format_id: String,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    filesize: Option<u64>,
    #[serde(default)]
    filesize_approx: Option<u64>,
    #[serde(default)]
    format_note: Option<String>,
}

/// Ambil daftar format NYATA untuk sebuah URL via `yt-dlp -J` (simulated
/// extraction, tanpa download). Gagal dalam bentuk apa pun (tool tidak ada,
/// timeout 20 dtk, parse error) → Vec KOSONG; pemanggil fallback ke daftar
/// statis seperti perilaku ≤2.5.x.
pub async fn fetch_formats(url: &str, config: &Config) -> Vec<FormatOption> {
    let mut cmd: Vec<String> = vec![
        "yt-dlp".into(),
        "-J".into(),
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--retries".into(),
        "2".into(),
        "--socket-timeout".into(),
        "10".into(),
    ];
    cmd.extend(cookie_args(url));
    if !config.proxy_url.trim().is_empty() {
        cmd.extend(["--proxy".into(), config.proxy_url.trim().to_string()]);
    }
    if !config.verify_tls {
        cmd.push("--no-check-certificates".into());
    }
    cmd.push(url.to_string());

    let Ok(child) = tokio::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return Vec::new();
    };
    // Cap 20 dtk: ekstraksi situs tertentu bisa sangat lambat — lebih baik
    // pakai daftar statis daripada user menunggu tanpa kepastian.
    let Ok(out) = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output()).await else {
        return Vec::new(); // future di-drop → kill_on_drop meracun yt-dlp
    };
    let Ok(status) = out else { return Vec::new() };
    if !status.status.success() {
        return Vec::new();
    }
    parse_formats_json(&String::from_utf8_lossy(&status.stdout))
}

/// Parse output `yt-dlp -J` (pub(crate) supaya bisa di-unit test).
pub(crate) fn parse_formats_json(s: &str) -> Vec<FormatOption> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
        return Vec::new();
    };
    let Some(arr) = v.get("formats").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<FormatOption> = Vec::new();
    for f in arr {
        let Ok(fmt) = serde_json::from_value::<YtFormatJson>(f.clone()) else {
            continue;
        };
        if fmt.format_id.is_empty() {
            continue;
        }
        let ext = fmt.ext.unwrap_or_default();
        if ext == "mhtml" {
            continue; // bukan media — sampah dari "-J"
        }
        let has_v = !fmt.vcodec.as_deref().map_or(false, |c| c == "none" || c.is_empty());
        let has_a = !fmt.acodec.as_deref().map_or(false, |c| c == "none" || c.is_empty());
        let kind = match (has_v, has_a) {
            (true, true) => "video+audio",
            (true, false) => "video",
            (false, true) => "audio",
            _ => continue, // container tanpa stream? lewati
        };
        let label = match fmt.height {
            Some(h) if h > 0 => format!("{} {}p", ext, h),
            _ => format!("{} {}", ext, kind),
        };
        let mut desc = kind.to_string();
        if let Some(sz) = fmt.filesize_approx.or(fmt.filesize).filter(|b| *b > 0) {
            desc = format!("{} · {}", desc, super::types::format_size(sz));
        }
        if let Some(note) = fmt.format_note {
            if !note.is_empty() {
                desc = format!("{} · {}", desc, note);
            }
        }
        if !seen.insert(fmt.format_id.clone()) {
            continue;
        }
        out.push(FormatOption {
            id: fmt.format_id,
            label,
            desc,
        });
        if out.len() >= 24 {
            break; // dialog sudah panjang; 24 entri lebih dari cukup
        }
    }
    out
}

pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &Config,
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
        "--socket-timeout".into(),
        "15".into(),
        "--retries".into(),
        "5".into(),
        "--merge-output-format".into(),
        "mp4".into(),
        "--embed-thumbnail".into(),
        "--embed-metadata".into(),
    ]);

    cmd.extend(cookie_args(&url));

    // v2.4.0 (D3): proxy dari Pengaturan — yt-dlp & ffmpeg turunannya ikut.
    if !config.proxy_url.trim().is_empty() {
        cmd.extend(["--proxy".into(), config.proxy_url.trim().to_string()]);
    }

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

    // v2.3.1 (M1): async penuh — run_ytdlp kini memakai tokio::process
    run_ytdlp(cmd, info.clone(), tx.clone()).await;
}

/// v2.3.1 (M1): async penuh via tokio::process — lihat komentar
/// `aria2::run_aria2c` untuk alasan lengkap (ticker cek status, wait paus
/// terbatas + eskalasi SIGKILL, kill_on_drop, ChildLines anti-kehilangan-byte).
/// `false` = tidak selesai normal (cancel/pause/error) — dipakai universal.rs
/// untuk memutuskan fallback aria2.
pub(crate) async fn run_ytdlp(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) -> bool {
    // process_group(0) → killpg menjangkau ffmpeg anak-anaknya saat pause/cancel (K4).
    let mut child = match tokio::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "yt-dlp tidak terinstall — jalankan: sudo apt install yt-dlp".to_string()
            } else {
                format!("yt-dlp: {}", e)
            };
            let mut i = info.lock().await;
            i.status = DownloadStatus::Error;
            i.error_msg = msg;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
            return false;
        }
    };

    // Simpan PID supaya bisa di-kill saat app ditutup (anti orphan).
    // id() -> Option; kill di bawah dijaga — JANGAN pernah killpg(0).
    let pid = child.id();
    info.lock().await.pid = pid;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Baca stderr di task terpisah agar pipe 64KB tidak membuat yt-dlp macet.
    let stderr_task = tokio::spawn(async move {
        let mut lines = super::ChildLines::new(stderr);
        let mut buf = String::new();
        while let Some(line) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
            // Batasi 16 KB (pertahankan yang terbaru) — situs bermasalah
            // bisa membanjiri stderr tanpa batas
            if buf.len() > 16 * 1024 {
                let cut = buf.len() - 8 * 1024;
                let drop = buf[..cut].find('\n').map(|i| i + 1).unwrap_or(cut);
                buf.drain(..drop);
            }
        }
        buf
    });

    let mut lines = super::ChildLines::new(stdout);
    let mut last_update = Instant::now();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut aborted = None;

    loop {
        let line = tokio::select! {
            biased;
            l = lines.next_line() => match l {
                Some(s) => s,
                None => break,
            },
            _ = ticker.tick() => {
                let status = info.lock().await.status;
                if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
                    aborted = Some(status);
                    break;
                }
                continue;
            }
        };

        if let Some(m) = RE_YTDLP_DEST.captures(&line) {
            let filename = std::path::Path::new(m.get(1).unwrap().as_str())
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            info.lock().await.filename = filename;
        }

        if RE_YTDLP_MERGE.is_match(&line) {
            let mut i = info.lock().await;
            i.progress = 99.0;
            // M10: info merge → status_detail (bukan error_msg merah)
            i.status_detail = "Menggabungkan video + audio…".into();
            let _ = tx.send(DownloadEvent::Progress(i.clone()));
            continue;
        }

        let progress_match = RE_YTDLP_PROGRESS
            .captures(&line)
            .or_else(|| RE_YTDLP_PROGRESS2.captures(&line));

        if let Some(m) = progress_match {
            if last_update.elapsed().as_millis() >= 250 {
                let pct: f64 = m[1].parse().unwrap_or(0.0);
                let speed_str = m.get(3).map(|s| s.as_str()).unwrap_or("");
                let speed = parse_speed(speed_str);
                // Size (grup 2) & ETA (grup 4) hanya ada di format progress lengkap
                let total = m.get(2).map(|s| parse_speed(s.as_str())).unwrap_or(0);
                let eta = m.get(4).map(|s| parse_eta_hms(s.as_str())).unwrap_or(0);

                let mut i = info.lock().await;
                i.progress = pct;
                i.speed = speed;
                if total > 0 {
                    i.total_size = total;
                    i.downloaded = (pct / 100.0 * total as f64) as u64;
                }
                i.eta = eta;
                i.error_msg.clear();
                i.status_detail.clear();
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
                drop(i);
                last_update = Instant::now();
            }
        }
    }

    if let Some(status) = aborted {
        // B8: Paused → SIGTERM sudah dikirim pause_download(); tunggu proses
        // berhenti rapi (file .part tetap bisa di-resume) TERBATAS 30 dtk (M1),
        // lalu eskalasi SIGKILL ke group bila yt-dlp/ffmpeg macet. Cancelled →
        // kill seluruh group (ffmpeg ikut mati, K4).
        if status == DownloadStatus::Paused {
            if tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
                .await
                .is_err()
            {
                if let Some(pid) = pid {
                    super::kill_child_group_hard(pid);
                }
            }
        } else if let Some(pid) = pid {
            super::kill_child_group_hard(pid);
        }
        let _ = child.kill().await; // SIGKILL langsung + reap
        info.lock().await.pid = None;
        stderr_task.abort();
        return false;
    }

    let err_detail = stderr_task.await.unwrap_or_default();
    let exit_code = child
        .wait()
        .await
        .map(|s| s.code().unwrap_or(-1))
        .unwrap_or(-1);

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
        i.status_detail.clear();
        let _ = tx.send(DownloadEvent::Completed(i.clone()));
        true
    } else {
        i.status = DownloadStatus::Error;
        let detail = if err_detail.is_empty() {
            String::new()
        } else {
            format!("\n{}", err_detail.trim())
        };
        i.error_msg = format!("yt-dlp exit code: {}{}", exit_code, detail);
        i.status_detail.clear();
        i.speed = 0;
        let _ = tx.send(DownloadEvent::Error(i.clone()));
        false
    }
}

/// Parse ETA yt-dlp ("MM:SS" atau "HH:MM:SS") → detik
fn parse_eta_hms(s: &str) -> u64 {
    let nums: Vec<u64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
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
        if unit.contains("gib") {
            return (val * 1073741824.0) as u64;
        }
        if unit.contains("mib") {
            return (val * 1048576.0) as u64;
        }
        if unit.contains("kib") {
            return (val * 1024.0) as u64;
        }
        // Format lama/alternatif: "2.5M/s", "500K/s", "1.2G/s"
        if unit.starts_with('g') {
            return (val * 1073741824.0) as u64;
        }
        if unit.starts_with('m') {
            return (val * 1048576.0) as u64;
        }
        if unit.starts_with('k') {
            return (val * 1024.0) as u64;
        }
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
            // M6: kasus yang regex lama lewatkan
            "https://www.youtube.com/watch?app=desktop&v=dQw4w9WgXcQ", // v= bukan param pertama
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/v/dQw4w9WgXcQ",
            "youtube.com/watch?v=abc", // tanpa scheme (GUI menambahkan https://, tapi fungsi harus toleran)
            "https://m.youtube.com/watch?v=abc",
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
            // host mirip tapi bukan YouTube
            "https://notyoutube.com/watch?v=abc",
            "https://youtube.com.evil.test/watch?v=abc",
            // halaman non-video di youtube.com → biar universal resolver yang urus
            "https://www.youtube.com/",
            "https://www.youtube.com/playlist?list=PL123",
            // watch tanpa id
            "https://www.youtube.com/watch",
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
        assert_eq!(output_template("/tmp", "video.mp4"), "/tmp/video.mp4");
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
    fn parse_formats_json_basic() {
        let j = r#"{"formats":[
            {"format_id":"251","ext":"webm","acodec":"opus","vcodec":"none","height":null,"filesize_approx":2000000},
            {"format_id":"137","ext":"mp4","acodec":"none","vcodec":"av01.0.08M.08","height":1080},
            {"format_id":"137","ext":"mp4","acodec":"none","vcodec":"av01.0.08M.08","height":1080},
            {"format_id":"18","ext":"mp4","acodec":"mp4a.40.2","vcodec":"avc1.42001E","height":360,"format_note":"low res"},
            {"format_id":"0","ext":"mhtml","vcodec":"none","acodec":"none"}
        ]}"#;
        let v = parse_formats_json(j);
        assert_eq!(v.len(), 3, "duplikat id & mhtml harus tersaring");
        assert_eq!(v[0].id, "251");
        assert_eq!(v[0].label, "webm audio");
        assert!(v[0].desc.contains("audio"));
        assert_eq!(v[1].label, "mp4 1080p");
        assert_eq!(v[2].desc, "video+audio · low res");
    }

    #[test]
    fn parse_formats_json_garbage_is_empty() {
        assert!(parse_formats_json("").is_empty());
        assert!(parse_formats_json("bukan json").is_empty());
        assert!(parse_formats_json("{}").is_empty());
        assert!(parse_formats_json(r#"{"formats": null}"#).is_empty());
    }

    #[test]
    fn quality_args_passthrough_real_format_id() {
        assert_eq!(
            quality_args(Some("137+140")),
            vec!["--format".to_string(), "137+140/best".to_string()]
        );
        // preset tidak boleh ter-bajak passthrough
        assert!(quality_args(Some("audio_mp3")).contains(&"--audio-format".to_string()));
        assert!(quality_args(Some("720p")).iter().any(|a| a.contains("height<=720")));
        // bukan format id (whitespace dsb.) → tetap default teraman
        assert_eq!(
            quality_args(Some("rm -rf /")),
            vec![
                "--format".to_string(),
                "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best".to_string()
            ]
        );
    }

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
        // None atau string non-sens → default "best MP4" dengan fallback
        // berjenjang (/best di akhir → video AV1/VP9-only tetap terunduh).
        let args = quality_args(None);
        assert!(args.contains(&"--format".to_string()));
        assert!(args[1].contains("mp4"));

        // v2.6.1: kata bebas BUKAN id format (tak mengandung digit) → default,
        // bukan "--format unknown/best" (bug guard D6 yang test ini tangkap).
        let args = quality_args(Some("unknown"));
        assert!(args[1].contains("mp4"));
    }

    #[test]
    fn quality_args_non_numeric_p_ignored() {
        // "high" tidak berakhir digit+"p" dan tanpa digit → default
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
        assert_eq!(desktop_to_browser("microsoft-edge.desktop"), Some("edge"));
    }

    #[test]
    fn desktop_to_browser_unknown() {
        assert_eq!(desktop_to_browser("libreoffice.desktop"), None);
        assert_eq!(desktop_to_browser(""), None);
    }
}
