use super::types::*;
use crate::config::Config;
use regex::Regex;
use std::process::Stdio;
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
static RE_CD_RFC5987: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"filename\*\s*=\s*(?:[Uu][Tt][Ff]-8)?'[^']*'(.+?)(?:\s*;|$)").unwrap()
});
static RE_CD_QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"filename\s*=\s*"([^"]+)""#).unwrap());
static RE_CD_UNQUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"filename\s*=\s*([^\s;]+)").unwrap());

const CHROME_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub async fn download(
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    config: &Config,
) {
    // Guard: user bisa cancel/pause di jeda sebelum proses aria2c lahir
    // (pid belum ada → kill_child_pid no-op). Tanpa guard, status ditimpa
    // Resolving dan download yang "dibatalkan" jalan terus.
    {
        let i = info.lock().await;
        if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return;
        }
    }

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

    // Spawn aria2c — v2.3.1 (M1): async penuh, tanpa spawn_blocking
    run_aria2c(cmd, info.clone(), tx.clone()).await;

    // Cleanup input file
    if let Some(path) = input_file {
        let _ = std::fs::remove_file(path);
    }
}

fn build_aria2_cmd(info: &DownloadInfo, config: &Config) -> (Vec<String>, Option<String>) {
    // Write URL to input file (handles long URLs)
    // v2.3.0 (K3): direktori privat (XDG_RUNTIME_DIR/config dir, 0700) +
    // file 0600 — URL bisa mengandung token; jangan lagi di /tmp publik.
    let input_dir = Config::aria2_input_dir();
    let input_path = input_dir.join(format!("aria2-{}.txt", info.id));

    // Hanya URL di input-file (untuk menangani URL panjang). Semua opsi lain
    // dikirim sebagai argumen CLI: nilai dengan spasi (path folder, nama file,
    // header) tidak salah di-parse oleh format input-file aria2.
    let _ = std::fs::write(&input_path, format!("{}\n", info.url));
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&input_path, std::fs::Permissions::from_mode(0o600));
    }

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
        // Limit user adalah TOTAL aplikasi, tapi tiap download = proses aria2c
        // sendiri. Engine sudah membaginya menurut jumlah unduhan hidup SAAT
        // proses ini start (v2.3.0 M3, lihat DownloadEngine::spawn_supervised)
        // — di sini tinggal pakai. "0" = tanpa batas.
        format!("--max-overall-download-limit={}", config.max_overall_speed),
        // check-integrity sengaja TIDAK dimatikan (default aria2 = true):
        // tanpa ini, resume setelah crash bisa menandai file korup sebagai
        // selesai.
        "--continue=true".into(),
        // Konfigurasi auto_file_renaming sebelumnya diabaikan (hardcoded
        // false) — file tabrakan SELALU ditimpa. Sekarang dihormati:
        // default true → tabrakan menjadi "file (1).ext". allow-overwrite
        // harus berlawanan: kalau overwrite=true, aria2 menimpa SEBELUM
        // sempat auto-rename.
        format!("--allow-overwrite={}", !config.auto_file_renaming).into(),
        format!("--auto-file-renaming={}", config.auto_file_renaming).into(),
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

    // Cookies dari browser extension (Netscape format, file per-domain,
    // termasuk domain induk) — aria2 otomatis hanya memakai cookie yang
    // cocok dengan domain target
    if let Ok(u) = url::Url::parse(&info.url) {
        if let Some(host) = u.host_str() {
            if let Some(cookies) = Config::find_cookies_file(host) {
                cmd.push(format!("--load-cookies={}", cookies.display()));
            }
        }
    }

    (cmd, Some(input_path.to_string_lossy().to_string()))
}

/// v2.3.1 (M1): tokio::process penuh — pola lama (std::process di dalam
/// `spawn_blocking` + `Handle::current().block_on` per baris output) rapuh dan
/// punya tiga bug nyata:
/// 1. pause/cancel hanya dicek saat baris output baru tiba — aria2c yang stall
///    (tanpa output) membuat tombol user tidak berdampak sampai ada baris lagi;
/// 2. `child.wait()` tanpa batas — child yang tidak merespons SIGTERM (pause)
///     membekukan thread blocking selamanya;
/// 3. thread khusus stderr yang bisa bocor saat panic.
/// Sekarang: `ChildLines` (baca cancellation-safe), ticker 500ms untuk cek
/// status walau child diam, wait PAUSA terbatas 30 dtk dengan eskalasi SIGKILL,
/// dan `kill_on_drop` sebagai jaring pengaman bila future seluruhnya di-drop.
async fn run_aria2c(
    cmd: Vec<String>,
    info: Arc<Mutex<DownloadInfo>>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
) {
    // process_group(0): child jadi leader group → SIGTERM/SIGKILL via killpg
    // menjangkau seluruh keturunannya (K4).
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
                "aria2c tidak terinstall — jalankan: sudo apt install aria2".to_string()
            } else {
                format!("aria2c: {}", e)
            };
            let mut i = info.lock().await;
            i.status = DownloadStatus::Error;
            i.error_msg = msg;
            let _ = tx.send(DownloadEvent::Error(i.clone()));
            return;
        }
    };

    // Simpan PID supaya bisa di-kill saat app ditutup (anti orphan).
    // tokio: id() -> Option, None bila proses sudah selesai. Semua pemakaian
    // pid di bawah dijaga Option — killpg(0) akan mengenai GRUP FAST-DM SENDIRI.
    let pid = child.id();
    info.lock().await.pid = pid;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // stderr disedot task terpisah — kalau tidak, buffer pipe (64KB) bisa
    // penuh oleh log error/warning dan aria2c berhenti menulis → deadlock.
    // Task mengembalikan seluruh buffer; tidak perlu Mutex karena pemakainya
    // hanya jalur ini (join sebelum membaca isinya).
    let stderr_task = tokio::spawn(async move {
        let mut lines = super::ChildLines::new(stderr);
        let mut buf = String::new();
        while let Some(line) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
            // Batasi 16 KB (pertahankan yang terbaru) — log error bisa
            // sangat panjang untuk download yang bermasalah
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
    // v2.3.1 (M1): cek status juga saat child tidak mengeluarkan output.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut aborted = None;

    loop {
        let line = tokio::select! {
            biased;
            l = lines.next_line() => match l {
                Some(s) => s,
                None => break, // stdout tertutup — aria2c akan selesai
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
                let mut i = info.lock().await;
                i.downloaded = downloaded;
                i.total_size = total;
                i.progress = progress;
                i.speed = speed;
                i.connections = connections;
                i.eta = eta;
                // Bersihkan error lama saat download berjalan lagi (mis. setelah Retry)
                i.error_msg.clear();
                i.status_detail.clear();
                let _ = tx.send(DownloadEvent::Progress(i.clone()));
                drop(i);
                last_update = Instant::now();
            }
        }
    }

    if let Some(status) = aborted {
        // B8: Paused → SIGTERM sudah dikirim pause_download(); TUNGGU aria2c
        // menulis control file .aria2 (supaya bisa di-resume), jangan SIGKILL —
        // tapi sekarang TERBATAS 30 dtk (M1), lalu eskalasi SIGKILL ke group
        // bila aria2c macet. Cancelled → langsung SIGKILL group (K4) + kill()
        // langsung sebagai fallback dan reap anti-zombie.
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
        let _ = child.kill().await; // SIGKILL child langsung + reap (no-op jika sudah mati)
        info.lock().await.pid = None;
        stderr_task.abort();
        return;
    }

    // stdout EOF → proses akan selesai. Tunggu stderr selesai (pipe tertutup),
    // lalu exit code. (kill_on_drop tetap jadi jaring pengaman jalur panic.)
    let err_detail = stderr_task.await.unwrap_or_default();
    let exit_code = child
        .wait()
        .await
        .map(|s| s.code().unwrap_or(-1))
        .unwrap_or(-1);

    let mut i = info.lock().await;
    i.pid = None;

    if matches!(i.status, DownloadStatus::Cancelled | DownloadStatus::Paused) {
        return;
    }

    if exit_code == 0 {
        i.status = DownloadStatus::Completed;
        i.progress = 100.0;
        i.speed = 0;
        i.status_detail.clear();
        let _ = tx.send(DownloadEvent::Completed(i.clone()));
    } else {
        let detail = if err_detail.trim().is_empty() {
            String::new()
        } else {
            format!("\n{}", err_detail.trim())
        };
        i.status = DownloadStatus::Error;
        i.error_msg = format!("aria2c exit code: {}{}", exit_code, detail);
        i.status_detail.clear();
        i.speed = 0;
        let _ = tx.send(DownloadEvent::Error(i.clone()));
    }
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

/// Limit total user → batas per-proses aria2c. v2.3.0 (M3): pembaginya
/// adalah jumlah unduhan HIDUP (aktif+antri) saat proses ini start — dihitung
/// engine — bukan `max_concurrent` statis, sehingga unduhan tunggal kini
/// memakai limit penuh. "0" (maupun "0K") = tanpa batas → "0".
pub(crate) fn resolve_speed_limit(total: &str, live_share: usize) -> String {
    let total_bytes = parse_speed_setting(total);
    if total_bytes == 0 {
        return "0".into();
    }
    let per = total_bytes / (live_share.max(1) as u64);
    if per < 1024 {
        return "1K".into();
    }
    format!("{:.0}K", per as f64 / 1024.0)
}

/// "0" | "512K" | "2M" | "10G" → byte/detik (konvensi aria2: K = 1024)
fn parse_speed_setting(s: &str) -> u64 {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let num: f64 = num.parse().unwrap_or(0.0);
    let mult: f64 = match unit.trim().to_ascii_uppercase().as_str() {
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (num * mult) as u64
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

/// HTTP client resolve — DI-BAGIKAN (dibuat sekali, bukan per-download).
/// Membuat client baru tiap unduhan berarti setup TLS + koneksi ulang yang
/// tidak perlu. Return Option untuk mempertahankan semantics lama: kalau
/// client gagal dibangun, resolve dilewati (bukan panic → abort).
fn resolve_client(verify_tls: bool) -> Option<&'static reqwest::Client> {
    static VERIFY: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    static NO_VERIFY: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let slot = if verify_tls { &VERIFY } else { &NO_VERIFY };
    if slot.get().is_none() {
        let client = reqwest::Client::builder()
            .user_agent(CHROME_UA)
            .danger_accept_invalid_certs(!verify_tls)
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(10))
            .build();
        if let Ok(c) = client {
            let _ = slot.set(c);
        }
    }
    slot.get()
}

async fn resolve_filename(info: &Arc<Mutex<DownloadInfo>>, verify_tls: bool) -> Result<(), String> {
    let (url, headers) = {
        let i = info.lock().await;
        (i.url.clone(), i.headers.clone())
    };

    let Some(client) = resolve_client(verify_tls) else {
        // Client tidak bisa dibangun → lewati resolve (sebutir nama dari URL
        // tetap dipakai) — perilaku sama seperti versi sebelumnya.
        return Ok(());
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
    let is_html = ct == "text/html" || ct == "application/xhtml+xml" || ct.contains("text/html");
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
        let fake_ext = [
            ".php", ".asp", ".aspx", ".jsp", ".do", ".action", ".html", ".htm",
        ]
        .iter()
        .any(|e| lower.ends_with(e));
        if fake_ext && !i.filename.is_empty() {
            let stem = i
                .filename
                .rsplit_once('.')
                .map(|(s, _)| s.to_string())
                .unwrap_or_else(|| i.filename.clone());
            tracing::info!(
                "Ganti ekstensi {} → {} (content-type: {})",
                i.filename,
                ext,
                ct
            );
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
    let host = url::Url::parse(url)
        .ok()?
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    // File per-domain dulu (termasuk domain induk — file video sering ada di
    // subdomain CDN, sedangkan cookies disimpan dengan host halaman);
    // fallback ke cookies.txt lama (versi sebelumnya)
    let path = Config::find_cookies_file(&host)
        .unwrap_or_else(|| Config::config_dir().join("cookies.txt"));
    let text = std::fs::read_to_string(&path).ok()?;
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
        "download", "index", "file", "get", "fetch", "stream", "media", "content", "data",
        "output", "video", "audio", "default", "main",
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
        "video/mp4" => Some(".mp4"),
        "video/webm" => Some(".webm"),
        "video/x-matroska" => Some(".mkv"),
        "video/quicktime" => Some(".mov"),
        "video/x-msvideo" => Some(".avi"),
        "video/x-flv" => Some(".flv"),
        "video/3gpp" => Some(".3gp"),
        "video/mp2t" => Some(".ts"),
        "audio/mpeg" => Some(".mp3"),
        "audio/mp4" => Some(".m4a"),
        "audio/ogg" => Some(".ogg"),
        "audio/wav" => Some(".wav"),
        "audio/flac" => Some(".flac"),
        "application/pdf" => Some(".pdf"),
        "application/zip" => Some(".zip"),
        "application/gzip" => Some(".gz"),
        "application/x-rar-compressed" => Some(".rar"),
        "application/x-7z-compressed" => Some(".7z"),
        "application/x-tar" => Some(".tar"),
        "application/x-iso9660-image" => Some(".iso"),
        "image/jpeg" => Some(".jpg"),
        "image/png" => Some(".png"),
        "image/gif" => Some(".gif"),
        "image/webp" => Some(".webp"),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_aria2_size ──

    #[test]
    fn parse_aria2_size_raw_bytes() {
        // Mode non-human: angka mentah
        assert_eq!(parse_aria2_size("1048576"), 1048576);
        assert_eq!(parse_aria2_size("0"), 0);
    }

    #[test]
    fn parse_aria2_size_human_kib() {
        // KiB = 1024, MiB = 1024², dst
        assert_eq!(parse_aria2_size("1KiB"), 1024);
        assert_eq!(parse_aria2_size("1.5KiB"), 1536);
        assert_eq!(parse_aria2_size("400.0KiB"), 409_600);
    }

    #[test]
    fn parse_aria2_size_human_mib_gib() {
        // 33.2 × 1024 × 1024 ≈ 34_812_723
        assert_eq!(parse_aria2_size("33.2MiB"), 34_812_723);
        // 1.4 × 1024 × 1024 ≈ 1_468_006
        assert_eq!(parse_aria2_size("1.4MiB"), 1_468_006);
        assert_eq!(parse_aria2_size("2GiB"), 2_147_483_648);
    }

    #[test]
    fn parse_aria2_size_short_unit() {
        // aria2 kadang output "300K", "1M" (tanpa "iB")
        assert_eq!(parse_aria2_size("512K"), 512 * 1024);
        assert_eq!(parse_aria2_size("2M"), 2 * 1024 * 1024);
    }

    #[test]
    fn parse_aria2_size_with_trailing_slash() {
        // Kecepatan ditulis "DL:300KiB/s" → parser hanya ambil bagian numeric
        // (regex DL: dipisah). Test parser saja:
        assert_eq!(parse_aria2_size("300KiB"), 307_200);
    }

    #[test]
    fn parse_aria2_size_invalid() {
        // String kosong / tanpa angka → 0 (bukan panic)
        assert_eq!(parse_aria2_size(""), 0);
        assert_eq!(parse_aria2_size("abc"), 0);
        assert_eq!(parse_aria2_size("."), 0);
    }

    // ── parse_eta ──

    #[test]
    fn parse_eta_seconds() {
        assert_eq!(parse_eta("30s"), 30);
        assert_eq!(parse_eta("5s"), 5);
    }

    #[test]
    fn parse_eta_minutes_seconds() {
        assert_eq!(parse_eta("4m51s"), 4 * 60 + 51);
        assert_eq!(parse_eta("10m"), 600);
    }

    #[test]
    fn parse_eta_hours_minutes_seconds() {
        assert_eq!(parse_eta("1h2m3s"), 3600 + 120 + 3);
        assert_eq!(parse_eta("2h"), 7200);
    }

    #[test]
    fn parse_eta_empty_or_zero() {
        assert_eq!(parse_eta("0s"), 0);
        assert_eq!(parse_eta(""), 0);
    }

    // ── parse_speed_setting ──

    #[test]
    fn parse_speed_setting_zero() {
        assert_eq!(parse_speed_setting("0"), 0);
        assert_eq!(parse_speed_setting("  0  "), 0); // trim
    }

    #[test]
    fn parse_speed_setting_units() {
        // K = 1024 (konvensi aria2)
        assert_eq!(parse_speed_setting("512K"), 512 * 1024);
        assert_eq!(parse_speed_setting("2M"), 2 * 1024 * 1024);
        assert_eq!(parse_speed_setting("10G"), 10 * 1024u64.pow(3));
    }

    #[test]
    fn parse_speed_setting_case_insensitive() {
        assert_eq!(parse_speed_setting("2k"), 2 * 1024);
        assert_eq!(parse_speed_setting("2m"), 2 * 1024 * 1024);
    }

    // ── resolve_speed_limit (M3) ──

    #[test]
    fn resolve_speed_limit_unlimited() {
        // "0" = tanpa batas → return "0" berapapun pembaginya
        assert_eq!(resolve_speed_limit("0", 3), "0");
        assert_eq!(resolve_speed_limit("0K", 1), "0");
        assert_eq!(resolve_speed_limit("", 2), "0"); // kosong = tanpa batas
    }

    #[test]
    fn resolve_speed_limit_single_download_gets_full_budget() {
        // M3: dulu ini "341K" (selalu /max_concurrent=3); kini penuh
        assert_eq!(resolve_speed_limit("1M", 1), "1024K");
    }

    #[test]
    fn resolve_speed_limit_divided_by_live() {
        // 1M / 2 unduhan hidup = 512K per proses
        assert_eq!(resolve_speed_limit("1M", 2), "512K");
        assert_eq!(resolve_speed_limit("2M", 4), "512K");
    }

    #[test]
    fn resolve_speed_limit_floor_at_1k() {
        // Limit kecil / banyak unduhan → minimum 1K
        assert_eq!(resolve_speed_limit("1K", 10), "1K");
        assert_eq!(resolve_speed_limit("512", 1), "1K"); // 512 B/s → floor
    }

    #[test]
    fn resolve_speed_limit_zero_share_treated_as_one() {
        // live_share=0 tidak mungkin (engine selalu ≥1) tapi jangan divide-by-zero
        assert_eq!(resolve_speed_limit("1M", 0), "1024K");
    }

    // ── is_generic_filename ──

    #[test]
    fn is_generic_filename_true_cases() {
        let generic = [
            "",
            "download",
            "index.html",
            "file.zip",
            "video.mp4",
            "get.bin",
            "default.jpg",
            "123.mp4",
            "download_12345.mp4",
            "main.bin",
        ];
        for name in generic {
            assert!(is_generic_filename(name), "{} harus dianggap generic", name);
        }
    }

    #[test]
    fn is_generic_filename_false_cases() {
        let specific = [
            "my-video.mp4",
            "linuxmint-21-cinnamon-64bit.iso",
            "github-cli_2.40.0_linux_amd64.deb",
            "report-2024.pdf",
        ];
        for name in specific {
            assert!(
                !is_generic_filename(name),
                "{} harus dianggap specific",
                name
            );
        }
    }

    // ── content_type_to_ext ──

    #[test]
    fn content_type_to_ext_video() {
        assert_eq!(content_type_to_ext("video/mp4"), Some(".mp4"));
        assert_eq!(content_type_to_ext("video/webm"), Some(".webm"));
        assert_eq!(content_type_to_ext("video/quicktime"), Some(".mov"));
    }

    #[test]
    fn content_type_to_ext_audio() {
        assert_eq!(content_type_to_ext("audio/mpeg"), Some(".mp3"));
        assert_eq!(content_type_to_ext("audio/mp4"), Some(".m4a"));
    }

    #[test]
    fn content_type_to_ext_archive() {
        assert_eq!(content_type_to_ext("application/zip"), Some(".zip"));
        assert_eq!(
            content_type_to_ext("application/x-rar-compressed"),
            Some(".rar")
        );
        assert_eq!(
            content_type_to_ext("application/x-7z-compressed"),
            Some(".7z")
        );
    }

    #[test]
    fn content_type_to_ext_image() {
        assert_eq!(content_type_to_ext("image/jpeg"), Some(".jpg"));
        assert_eq!(content_type_to_ext("image/png"), Some(".png"));
    }

    #[test]
    fn content_type_to_ext_strips_charset() {
        // Content-Type bisa ada charset: "text/html; charset=utf-8"
        // (walau text/html harus ditolak duluan, parser-nya strip ";")
        assert_eq!(
            content_type_to_ext("video/mp4; charset=binary"),
            Some(".mp4")
        );
    }

    #[test]
    fn content_type_to_ext_unknown() {
        assert_eq!(content_type_to_ext("application/octet-stream"), None);
        assert_eq!(content_type_to_ext("text/html"), None);
    }

    // ── parse_content_disposition ──

    #[test]
    fn parse_content_disposition_rfc5987() {
        // RFC 5987: filename*=UTF-8''<urlencoded>
        let cd = "attachment; filename=\"fallback.zip\"; filename*=UTF-8''nama%20file.zip";
        assert_eq!(
            parse_content_disposition(cd),
            Some("nama file.zip".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_quoted() {
        let cd = "attachment; filename=\"my report.pdf\"";
        assert_eq!(
            parse_content_disposition(cd),
            Some("my report.pdf".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_unquoted() {
        let cd = "attachment; filename=file.zip";
        assert_eq!(parse_content_disposition(cd), Some("file.zip".to_string()));
    }

    #[test]
    fn parse_content_disposition_none() {
        // Tanpa filename sama sekali
        assert_eq!(parse_content_disposition("attachment"), None);
        assert_eq!(parse_content_disposition(""), None);
    }

    #[test]
    fn parse_content_disposition_rfc5987_lowercase() {
        // utf-8 (lowercase) juga harus cocok
        let cd = "filename*=utf-8''my%20video.mp4";
        assert_eq!(
            parse_content_disposition(cd),
            Some("my video.mp4".to_string())
        );
    }

    // ── Regex RE_PROGRESS (regresi: format dual) ──

    #[test]
    fn re_progress_matches_human_readable() {
        // Format: [#2089b0 400.0KiB/33.2MiB(1%) CN:1 DL:115.7KiB ETA:4m51s]
        let line = "[#2089b0 400.0KiB/33.2MiB(1%) CN:1 DL:115.7KiB ETA:4m51s]";
        let m = RE_PROGRESS
            .captures(line)
            .expect("harus match human format");
        assert_eq!(&m[1], "400.0KiB");
        assert_eq!(&m[2], "33.2MiB");
        assert_eq!(&m[3], "1");
    }

    #[test]
    fn re_progress_matches_raw_bytes() {
        // Format: [#2089b0 1048576/34896138(3%) CN:1 DL:524288 ETA:0s]
        let line = "[#2089b0 1048576/34896138(3%) CN:1 DL:524288 ETA:0s]";
        let m = RE_PROGRESS.captures(line).expect("harus match raw format");
        assert_eq!(&m[1], "1048576");
        assert_eq!(&m[2], "34896138");
        assert_eq!(&m[3], "3");
    }
}
