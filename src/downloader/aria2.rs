use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

// Readout aria2c:
//   --human-readable=true  -> [#2089b0 400.0KiB/33.2MiB(1%) CN:1 DL:115.7KiB ETA:4m51s]
//   --human-readable=false -> [#2089b0 1048576/34896138(3%) CN:1 DL:524288 ETA:0s]
// Regex harus menerima KEDUA format; sebelumnya hanya "123B/456B" yang tidak pernah
// muncul sama sekali, sehingga progress/speed/ETA tidak pernah ter-update.
static RE_PROGRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([\d.]+[KMGT]?i?B?)/([\d.]+[KMGT]?i?B?)\((\d+)%\)"#).unwrap());
static RE_SPEED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"DL:([\d.]+[KMGT]?i?B?/?s?)"#).unwrap());
static RE_CN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"CN:(\d+)").unwrap());
static RE_ETA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ETA:(\S+)").unwrap());
static RE_H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)h").unwrap());
static RE_M: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)m").unwrap());
static RE_S: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)s").unwrap());
static RE_CONTENT_RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(\d+)").unwrap());
static RE_CD_RFC5987: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"filename\*\s*=\s*(?:[Uu][Tt][Ff]-8)?'[^']*'(.+?)(?:\s*;|$)").unwrap());
static RE_CD_QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"filename\s*=\s*"([^"]+)""#).unwrap());
static RE_CD_UNQUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"filename\s*=\s*([^\s;]+)").unwrap());

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

    // Resolve filename + tolak halaman HTML (penyebab "file .php" terdownload)
    if let Err(msg) = resolve_filename(&info, config.verify_tls).await {
        let mut i = info.lock().await;
        i.status = DownloadStatus::Error;
        i.error_msg = msg;
        i.speed = 0;
        let _ = tx.send(DownloadEvent::Error(i.clone()));
        return;
    }

    // Check if cancelled during resolve
    {
        let i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }
    }

    // Pre-check ruang disk (total_size sudah diketahui dari resolve) —
    // gagal cepat dengan pesan jelas, bukan file korup di tengah jalan
    let (size, dir) = {
        let i = info.lock().await;
        (i.total_size, i.save_dir.clone())
    };
    if size > 0 && !has_space(&dir, size) {
        let mut i = info.lock().await;
        i.status = DownloadStatus::Error;
        i.error_msg = format!("Ruang disk tidak cukup — butuh {}", format_size(size));
        let _ = tx.send(DownloadEvent::Error(i.clone()));
        return;
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

    // Hanya URL di input-file (untuk menangani URL panjang). Semua opsi lain
    // dikirim sebagai argumen CLI: nilai dengan spasi (path folder, nama file,
    // header) tidak salah di-parse oleh format input-file aria2.
    let _ = std::fs::write(&input_path, format!("{}\n", info.url));

    let mut cmd = vec![
        "aria2c".into(),
        format!("--input-file={}", input_path.display()),
        format!("--dir={}", info.save_dir),
        format!("--out={}", info.filename),
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
        "--continue=true".into(),
        "--allow-overwrite=true".into(),
        "--auto-file-renaming=false".into(),
    ];

    // Header kustom dari browser extension (mis. Referer) — strip \r\n anti injection.
    // Dikirim per argumen agar nilai dengan spasi aman.
    for (k, v) in &info.headers {
        let k = k.replace(['\r', '\n'], "");
        let v = v.replace(['\r', '\n'], "");
        if !k.is_empty() && !v.is_empty() {
            cmd.push(format!("--header={}: {}", k, v));
        }
    }

    // Verifikasi sertifikat TLS hanya dimatikan kalau user eksplisit memilih begitu
    if !config.verify_tls {
        cmd.push("--check-certificate=false".into());
    }

    // Cookies dari browser extension (Netscape format, file per-domain) —
    // aria2 otomatis hanya memakai cookie yang cocok dengan domain target
    if let Ok(u) = url::Url::parse(&info.url) {
        if let Some(host) = u.host_str() {
            let cookies = Config::cookies_file_for(host);
            if cookies.exists() {
                cmd.push(format!("--load-cookies={}", cookies.display()));
            }
        }
    }

    (cmd, Some(input_path.to_string_lossy().to_string()))
}

fn run_aria2c(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) {

    let child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "aria2c tidak terinstall — jalankan: sudo apt install aria2".to_string()
            } else {
                format!("aria2c: {}", e)
            };
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let mut i = info.lock().await;
                i.status = DownloadStatus::Error;
                i.error_msg = msg;
                let _ = tx.send(DownloadEvent::Error(i.clone()));
            });
            return;
        }
    };

    // Simpan PID supaya bisa di-kill saat app ditutup (anti orphan)
    {
        let rt = tokio::runtime::Handle::current();
        let pid = child.id();
        rt.block_on(async { info.lock().await.pid = Some(pid); });
    }

    let stdout = child.stdout.take().unwrap();

    // Baca stderr di thread terpisah — kalau tidak, buffer pipe (64KB) bisa
    // penuh oleh log error/warning dan aria2c berhenti menulis → deadlock.
    let stderr = child.stderr.take().unwrap();
    let stderr_buf = Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_thread = std::thread::Builder::new()
        .name("aria2-stderr".into())
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
        // Check cancel/pause
        let rt = tokio::runtime::Handle::current();
        let status = rt.block_on(async { info.lock().await.status });

        if matches!(status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            // B8: Paused → SIGTERM sudah dikirim pause_download(); TUNGGU aria2c
            // menulis control file .aria2 (supaya bisa di-resume), jangan SIGKILL.
            // Cancelled → paksa kill.
            if status == DownloadStatus::Cancelled {
                let _ = child.kill();
            }
            // Reap the child so it does not linger as a zombie process
            let _ = child.wait();
            rt.block_on(async { info.lock().await.pid = None; });
            return;
        }

        // Parse progress (dukung format raw & human-readable)
        if let Some(m) = RE_PROGRESS.captures(&line) {
            let downloaded = parse_aria2_size(&m[1]);
            let total = parse_aria2_size(&m[2]);
            let progress: f64 = m[3].parse().unwrap_or(0.0);

            let speed = RE_SPEED
                .captures(&line)
                .map(|m| parse_aria2_size(&m[1]))
                .unwrap_or(0u64);

            let connections = RE_CN
                .captures(&line)
                .and_then(|m| m[1].parse().ok())
                .unwrap_or(0u8);

            let eta = RE_ETA
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
                    // Bersihkan error lama saat download berjalan lagi (mis. setelah Retry)
                    i.error_msg.clear();
                    let _ = tx.send(DownloadEvent::Progress(i.clone()));
                });
                last_update = Instant::now();
            }
        }
    }

    // Tunggu pembaca stderr selesai (berarti proses sudah menutup stderr)
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }

    let exit_code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    let rt = tokio::runtime::Handle::current();
    rt.block_on(async {
        let mut i = info.lock().await;
        i.pid = None;

        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }

        if exit_code == 0 {
            i.status = DownloadStatus::Completed;
            i.progress = 100.0;
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Completed(i.clone()));
        } else {
            let err_detail = stderr_buf.lock().unwrap().clone();
            let detail = if err_detail.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", err_detail.trim())
            };
            i.status = DownloadStatus::Error;
            i.error_msg = format!("aria2c exit code: {}{}", exit_code, detail);
            i.speed = 0;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
        }
    });
}

/// Cek ruang disk tersedia untuk direktori tujuan. Gagal cek → izinkan (jangan blokir).
fn has_space(dir: &str, needed: u64) -> bool {
    match nix::sys::statvfs::statvfs(std::path::Path::new(dir)) {
        Ok(stat) => {
            let avail = stat.blocks_available() * stat.block_size();
            needed <= avail
        }
        Err(_) => true,
    }
}

fn parse_eta(s: &str) -> u64 {
    let mut total = 0u64;

    if let Some(m) = RE_H.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0) * 3600;
    }
    if let Some(m) = RE_M.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0) * 60;
    }
    if let Some(m) = RE_S.captures(s) {
        total += m[1].parse::<u64>().unwrap_or(0);
    }
    total
}

/// Parse ukuran output aria2c: angka mentah ("1048576") maupun human-readable
/// ("400.0KiB", "1.4MiB", "300KiB/s", "1.2Gi").
fn parse_aria2_size(s: &str) -> u64 {
    let s = s.trim();
    let end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let val: f64 = s[..end].parse().unwrap_or(0.0);
    let unit = s[end..].to_ascii_lowercase();

    let mult: f64 = if unit.contains("tib") {
        1024.0 * 1024.0 * 1024.0 * 1024.0
    } else if unit.contains("gib") || unit.starts_with('g') {
        1024.0 * 1024.0 * 1024.0
    } else if unit.contains("mib") || unit.starts_with('m') {
        1024.0 * 1024.0
    } else if unit.contains("kib") || unit.starts_with('k') {
        1024.0
    } else {
        1.0
    };

    (val * mult) as u64
}

async fn resolve_filename(info: &Arc<Mutex<DownloadInfo>>, verify_tls: bool) -> Result<(), String> {
    let (url, headers) = {
        let i = info.lock().await;
        (i.url.clone(), i.headers.clone())
    };

    let client = match reqwest::Client::builder()
        .user_agent(CHROME_UA)
        .danger_accept_invalid_certs(!verify_tls)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    // Header kustom (mis. Referer) juga dipakai saat resolve agar server yang
    // butuh auth tidak menolak → nama & ukuran tetap terdeteksi.
    let cookie = cookie_header_for(&url);
    let build_get = || {
        let mut req = client.get(&url).header("Range", "bytes=0-0");
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(c) = &cookie {
            req = req.header("Cookie", c.as_str());
        }
        req
    };
    let build_head = || {
        let mut req = client.head(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(c) = &cookie {
            req = req.header("Cookie", c.as_str());
        }
        req
    };

    // Try GET with Range 0-0 (more reliable than HEAD for Content-Disposition)
    let resp = build_get().send().await;

    let resp = match resp {
        Ok(r) => r,
        Err(_) => {
            // Fallback: try HEAD
            match build_head().send().await {
                Ok(r) => r,
                Err(_) => return Ok(()),
            }
        }
    };

    // 0. Tolak halaman HTML / HTTP error — inilah penyebab "file .php" yang
    //    sebenarnya isi halaman web. Jangan pernah menyimpannya sebagai download.
    if !resp.status().is_success() {
        return Err(format!(
            "Server menjawab HTTP {} — bukan file video (halaman error/protected).",
            resp.status().as_u16()
        ));
    }
    let ct_raw = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ct = ct_raw.split(';').next().unwrap_or("").trim().to_lowercase();
    let is_html = ct == "text/html"
        || ct == "application/xhtml+xml"
        || ct.contains("text/html");
    if is_html {
        return Err(
            "URL ini mengembalikan halaman web (HTML), bukan file video — posting/halaman situs \
             (mis. *.php/*.html). Buka halaman video lalu klik tombol ⚡ Unduh di player."
                .to_string(),
        );
    }

    let mut i = info.lock().await;

    // 1. Content-Disposition — paling akurat
    if let Some(cd) = resp.headers().get("content-disposition") {
        if let Ok(cd_str) = cd.to_str() {
            if let Some(name) = parse_content_disposition(cd_str) {
                let cleaned = super::sanitize_filename(&name);
                if !cleaned.is_empty() && cleaned.contains('.') {
                    tracing::info!("Filename from Content-Disposition: {}", cleaned);
                    i.filename = cleaned;
                }
            }
        }
    }

    // 2. Jika filename masih generic, coba dari URL final (setelah redirect)
    if is_generic_filename(&i.filename) {
        let final_url = resp.url().to_string();
        if final_url != url {
            let name = super::extract_filename_from_url(&final_url);
            if !is_generic_filename(&name) {
                tracing::info!("Filename from final URL: {}", name);
                i.filename = name;
            }
        }
    }

    // 3. Content-Range untuk total size
    if let Some(cr) = resp.headers().get("content-range") {
        if let Ok(cr_str) = cr.to_str() {
            if let Some(m) = RE_CONTENT_RANGE.captures(cr_str) {
                if let Ok(total) = m[1].parse::<u64>() {
                    if total > i.total_size {
                        i.total_size = total;
                    }
                }
            }
        }
    } else if let Some(cl) = resp.headers().get("content-length") {
        if let Ok(cl_str) = cl.to_str() {
            if let Ok(size) = cl_str.parse::<u64>() {
                if size > i.total_size {
                    i.total_size = size;
                }
            }
        }
    }

    // 4. Ekstensi dari content-type:
    //    a. nama tanpa ekstensi → tambahkan ekstensi media
    //    b. nama *.php / *.asp / *.jsp / *.do / *.html yang ternyata
    //       mengembalikan video → GANTI ekstensi ke ekstensi media asli
    if let Some(ext) = content_type_to_ext(&ct) {
        let lower = i.filename.to_lowercase();
        let fake_ext = [".php", ".asp", ".aspx", ".jsp", ".do", ".action", ".html", ".htm"]
            .iter()
            .any(|e| lower.ends_with(e));
        if fake_ext && !i.filename.is_empty() {
            let stem = i.filename
                .rsplit_once('.')
                .map(|(s, _)| s.to_string())
                .unwrap_or_else(|| i.filename.clone());
            tracing::info!("Ganti ekstensi {} → {} (content-type: {})", i.filename, ext, ct);
            i.filename = format!("{}{}", stem, ext);
        } else if !i.filename.contains('.') {
            i.filename = format!("{}{}", i.filename, ext);
        }
    }

    tracing::info!("Final filename: {} (size: {})", i.filename, i.total_size);
    Ok(())
}

/// Baca cookies.txt (Netscape) untuk domain URL → header "Cookie: ...".
/// Supaya resolve & aria2 memakai sesi login yang sama dengan browser.
fn cookie_header_for(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.trim_start_matches("www.").to_ascii_lowercase();
    // File per-domain dulu; fallback ke cookies.txt lama (versi sebelumnya)
    let path = Config::cookies_file_for(&host);
    let text = std::fs::read_to_string(&path)
        .or_else(|_| std::fs::read_to_string(Config::config_dir().join("cookies.txt")))
        .ok()?;
    let mut pairs: Vec<String> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let domain = f[0].trim_start_matches('.').to_ascii_lowercase();
        // Cookie berlaku bila domain sama / subdomain dari domain cookie
        if host == domain || host.ends_with(&format!(".{}", domain)) {
            let name = f[5].trim();
            let value = f[6].trim();
            if !name.is_empty() {
                pairs.push(format!("{}={}", name, value));
            }
        }
    }

    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

fn is_generic_filename(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    let generic = [
        "download", "index", "file", "get", "fetch",
        "stream", "media", "content", "data", "output",
        "video", "audio", "default", "main",
    ];
    if generic.contains(&stem) {
        return true;
    }
    if stem.starts_with("download_") {
        return true;
    }
    if stem.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

fn content_type_to_ext(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next().unwrap_or("").trim().to_lowercase();
    match ct.as_str() {
        "video/mp4"        => Some(".mp4"),
        "video/webm"       => Some(".webm"),
        "video/x-matroska" => Some(".mkv"),
        "video/quicktime"  => Some(".mov"),
        "video/x-msvideo"  => Some(".avi"),
        "video/x-flv"      => Some(".flv"),
        "video/3gpp"       => Some(".3gp"),
        "video/mp2t"       => Some(".ts"),
        "audio/mpeg"       => Some(".mp3"),
        "audio/mp4"        => Some(".m4a"),
        "audio/ogg"        => Some(".ogg"),
        "audio/wav"        => Some(".wav"),
        "audio/flac"       => Some(".flac"),
        "application/pdf"  => Some(".pdf"),
        "application/zip"  => Some(".zip"),
        "application/gzip" => Some(".gz"),
        "application/x-rar-compressed" => Some(".rar"),
        "application/x-7z-compressed"  => Some(".7z"),
        "application/x-tar"            => Some(".tar"),
        "application/x-iso9660-image"  => Some(".iso"),
        "image/jpeg"       => Some(".jpg"),
        "image/png"        => Some(".png"),
        "image/gif"        => Some(".gif"),
        "image/webp"       => Some(".webp"),
        _ => None,
    }
}

fn parse_content_disposition(cd: &str) -> Option<String> {
    // filename*=UTF-8''encoded_name (RFC 5987)
    if let Some(m) = RE_CD_RFC5987.captures(cd) {
        let decoded = urlencoding::decode(&m[1]).unwrap_or_default();
        let name = decoded.trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    // filename="quoted name"
    if let Some(m) = RE_CD_QUOTED.captures(cd) {
        let name = m[1].trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    // filename=unquoted
    if let Some(m) = RE_CD_UNQUOTED.captures(cd) {
        let name = m[1].trim().trim_matches('"').to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    None
}
