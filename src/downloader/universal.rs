use super::types::*;
use crate::config::Config;
use crate::downloader::aria2::conn_per_server;
use crate::downloader::youtube::{
    cookie_args, merge_output_format, network_args, output_template, quality_args,
    run_ytdlp_with_stdin, ytdlp_proxy_args, PrivateFileGuard, YTDLP_BATCH_FILE_STDIN,
};
use std::path::Path;
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
/// Pesan install mengikuti target Arch Linux (`pacman`), sama seperti
/// `youtube.rs` dan `aria2.rs` — satu sumber di `crate::pkg`.
fn mark_missing_tool(info: &mut DownloadInfo, binary: &str, pkg: &str) {
    info.status = DownloadStatus::Error;
    info.error_msg = crate::pkg::missing_tool_msg(binary, pkg);
    info.speed = 0;
}

/// File parsial yt-dlp (`<nama>.part`, `<nama>.ytdl`) di samping nama sekarang
/// → unduhan ini LANJUTAN, bukan unduhan baru.
///
/// Padanan `aria2::partial_control_exists` (yang mencari `<nama>.aria2`) di
/// jalur aria2: nama placeholder hasil sesi lama harus dipertahankan supaya
/// `--continue` melanjutkan, bukan memulai ulang dari nol dengan nama lain dan
/// meninggalkan parsial yatim.
fn ytdlp_partial_exists(save_dir: &str, filename: &str) -> bool {
    [".part", ".ytdl"].iter().any(|ext| {
        Path::new(save_dir)
            .join(format!("{}{}", filename, ext))
            .exists()
    })
}

/// v4.2.0: target impersonasi TLS untuk tahap 2 tangga eskalasi.
///
/// `--impersonate` hanya ada bila yt-dlp dibangun dengan curl_cffi (opsional
/// di Arch: `python-curl_cffi`). Probe gagal/daftar kosong → `None` dan
/// unduhan jalan dengan yt-dlp polos, jadi tidak ada jalur yang mati karena
/// fitur opsional ini absen.
async fn impersonation_target() -> Option<String> {
    static TARGET: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if let Some(cached) = TARGET.get() {
        return cached.clone();
    }
    let found = tokio::task::spawn_blocking(|| {
        let out = Command::new("yt-dlp")
            .arg("--list-impersonate-targets")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_impersonation_target(&String::from_utf8_lossy(&out.stdout))
    })
    .await
    .unwrap_or(None);
    let _ = TARGET.set(found.clone());
    found
}

/// Inti murni `impersonation_target`: kolom pertama tabel
/// `--list-impersonate-targets` adalah keluarga client (`chrome`,
/// `chrome_mobile`, `safari`, …). Keluarga chrome desktop diprioritaskan;
/// keluarga chrome lain (mobile) tetap dipakai sebagai fallback karena
/// `--impersonate chrome` diterima yt-dlp sebagai "chrome versi apa pun".
pub(crate) fn parse_impersonation_target(list: &str) -> Option<String> {
    let mut chrome_family = false;
    for line in list.lines() {
        let Some(client) = line.split_whitespace().next() else {
            continue;
        };
        if client == "chrome" {
            return Some("chrome".to_string());
        }
        if client.starts_with("chrome") {
            chrome_family = true;
        }
    }
    if chrome_family {
        Some("chrome".to_string())
    } else {
        None
    }
}

/// v3.3.4: tanyakan nama asli ke server lalu adopsi ke item unduhan — hanya
/// bila nama sekarang benar-benar belum diketahui.
///
/// Syarat (semuanya harus benar, lihat `should_probe_filename`):
/// * nama sekarang placeholder Fast-DM (`download_<millis>_<hex>`) ATAU tanpa
///   ekstensi sama sekali (`unknown_video` dari path CDN bertanda tangan);
/// * nama itu BUKAN pilihan user (dialog "Simpan Sebagai...");
/// * nama itu bukan manifest streaming — `.m3u8`/`.mpd` harus dibiarkan ke
///   yt-dlp untuk di-merge, bukan dijadikan nama output;
/// * belum ada file parsial di samping nama itu (`.part`/`.ytdl`) — kalau ada,
///   ini unduhan lanjutan dari sesi sebelumnya, bukan unduhan baru.
///
/// Adopsi nama juga meng-update `i.filename` + mengirim event Progress supaya
/// kartu GUI berubah saat unduhan berjalan — persis seperti yang dilakukan
/// jalur aria2 lewat `resolve_filename`.
async fn adopt_real_filename(
    info: &Arc<Mutex<DownloadInfo>>,
    tx: &mpsc::UnboundedSender<DownloadEvent>,
    url: &str,
    headers: &std::collections::HashMap<String, String>,
    config: &Config,
) -> Option<String> {
    {
        let i = info.lock().await;
        if i.user_named || !super::aria2::should_probe_filename(&i.filename) {
            return None;
        }
        if super::aria2::is_manifest_name(&i.filename) {
            return None;
        }
        if ytdlp_partial_exists(&i.save_dir, &i.filename) {
            return None;
        }
    }

    let name = super::aria2::probe_real_filename(url, headers, config).await?;

    let mut i = info.lock().await;
    // Re-check di bawah lock: user bisa saja membatalkan/pause saat probe
    // berjalan, dan nama yang baru ditemukan harus tetap masuk ke kartu.
    if i.stop_requested() {
        return None;
    }
    tracing::info!("Nama asli dari server (jalur yt-dlp): {}", name);
    i.filename = name;
    // Tabrakan nama WAJIB diselesaikan di sini, sebelum yt-dlp jalan: Fast-DM
    // mengirim `--no-overwrites`, jadi kalau file dengan nama itu sudah ada
    // yt-dlp GAGAL ("File already exists"), bukan menamai ulang seperti
    // browser. Skema `name (1).ext` yang sama sudah dipakai jalur aria2
    // (v3.2.9), jadi file yang sama yang diunduh lewat dua jalur berakhir
    // sama-sama. Retry setelah gagal juga aman: file final belum ada (hanya
    // `.part`), jadi nama tidak berubah dan yt-dlp melanjutkan `--continue`.
    super::aria2::apply_unique_filename(&mut i, config.auto_file_renaming);
    let name = i.filename.clone();
    // Kartu di GUI menampilkan `filename`; tanpa event ini kartu masih
    // menampilkan nama placeholder sampai progress berikutnya datang.
    let _ = tx.send(DownloadEvent::Progress(i.clone()));
    Some(name)
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
    let (url, save_dir, headers, quality, filename, impersonate) = {
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
            i.impersonate,
        )
    };

    // v3.3.4: nama asli dari server, SEBELUM yt-dlp menentukan nama file.
    //
    // Akar bug: `resolve_filename` (satu-satunya pembaca `Content-Disposition`)
    // hanya dipanggil dari 2 jalur aria2. Tautan bertanda tangan tanpa
    // ekstensi — PikPak `.../unknown_video?sign=...` — tidak punya ekstensi
    // media, jadi `is_direct_file_url` FALSE dan unduhan masuk ke resolver
    // universal (yt-dlp) TANPA pernah menanyakan nama ke server. Hasilnya
    // `output_template()` memakai `%(title)s.%(ext)s`, nama file ditentukan
    // extractor yt-dlp dari path bertanda tangan, dan file di disk maupun
    // kartu GUI sama-sama bernama `download.unknown_video`.
    //
    // Di sini kita menanyakan nama yang sama seperti jalur aria2, lalu
    // memakai hasilnya di `output_template` bawah sehingga nama di disk dan
    // nama di kartu identik dengan nama asli.
    let filename = match adopt_real_filename(&info, &tx, &url, &headers, config).await {
        Some(name) => name,
        None => filename,
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
        // Petunjuk install paket mengikuti target Arch — lihat `mark_missing_tool`.
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
    // v4.2.0 (tahap 2 tangga eskalasi): server yang menolak karena sidik jari
    // TLS non-browser (WAF/anti-bot) tidak bisa diyakinkan lewat header apa
    // pun. yt-dlp + curl_cffi mampu meniru handshake Chrome (`--impersonate`);
    // target dipilih dari `--list-impersonate-targets` dan TANPA target yang
    // tersedia argumen ini dilewati (yt-dlp polos tetap jalur fallback).
    if impersonate {
        if let Some(target) = impersonation_target().await {
            cmd.push("--impersonate".into());
            cmd.push(target);
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

    // ── v3.3.4: nama asli dari server, tanpa menimpa pilihan user ──

    /// Engine unduhan mini + item, supaya test tidak spawning proses.
    fn probe_fixture(
        filename: &str,
    ) -> (
        Arc<Mutex<DownloadInfo>>,
        mpsc::UnboundedSender<DownloadEvent>,
    ) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let info = Arc::new(Mutex::new(DownloadInfo::new(
            "dl_x".into(),
            "https://dl-pikpak.test/down/abc/unknown_video?sign=xyz".into(),
            filename.into(),
            "/tmp".into(),
            Default::default(),
            None,
        )));
        (info, tx)
    }

    /// Nama pilihan user (dialog "Simpan Sebagai...") TIDAK boleh ditimpa
    /// probe, walau tanpa ekstensi. Kedua guard ini jalan sebelum ada request
    /// apa pun, jadi test ini tidak menyentuh jaringan.
    #[tokio::test]
    async fn adopt_real_filename_never_overrides_a_user_chosen_name() {
        let (info, tx) = probe_fixture("myvideo");
        info.lock().await.user_named = true;
        let headers = std::collections::HashMap::new();

        let out = adopt_real_filename(
            &info,
            &tx,
            "https://dl-pikpak.test/down/abc/unknown_video?sign=xyz",
            &headers,
            &Config::default(),
        )
        .await;

        assert!(out.is_none(), "nama pilihan user tidak boleh ditimpa");
        assert_eq!(info.lock().await.filename, "myvideo");
    }

    /// Nama ber-ekstensi sudah final → probe dilewati tanpa request.
    #[tokio::test]
    async fn adopt_real_filename_skips_a_name_that_already_has_an_extension() {
        let name = "bangbrosclips.26.09.29.rika.fane.and.dalila.lapiedra.mp4";
        let (info, tx) = probe_fixture(name);
        let headers = std::collections::HashMap::new();

        let out = adopt_real_filename(
            &info,
            &tx,
            "https://dl-pikpak.test/down/abc/unknown_video?sign=xyz",
            &headers,
            &Config::default(),
        )
        .await;

        assert!(out.is_none());
        assert_eq!(info.lock().await.filename, name);
    }

    /// `.part`/`.ytdl` di samping nama = unduhan lanjutan, bukan unduhan baru.
    /// Nama placeholder dari sesi lama harus dipertahankan agar `--continue`
    /// melanjutkan, bukan memulai ulang dari nol.
    #[test]
    fn ytdlp_partial_marker_is_detected_next_to_the_name() {
        let dir = std::env::temp_dir().join("fast-dm-partial-test");
        std::fs::create_dir_all(&dir).expect("buat tempdir");
        let name = "download_1790300206135_cd6d.mp4";
        let path = |suffix: &str| dir.join(format!("{}{}", name, suffix));
        let dir_s = dir.to_string_lossy().to_string();

        assert!(!ytdlp_partial_exists(&dir_s, name), "belum ada parsial");
        std::fs::write(path(".part"), b"x").expect("tulis .part");
        assert!(ytdlp_partial_exists(&dir_s, name), ".part = target resume");
        std::fs::remove_file(path(".part")).expect("hapus .part");
        std::fs::write(path(".ytdl"), b"{}").expect("tulis .ytdl");
        assert!(ytdlp_partial_exists(&dir_s, name), ".ytdl = target resume");

        let _ = std::fs::remove_file(path(".ytdl"));
        let _ = std::fs::remove_dir(&dir);
    }

    // ── v4.2.0: pemilihan target impersonasi TLS ──

    /// Tabel nyata yt-dlp (kolom Client/OS/Version) dengan chrome desktop →
    /// target generik "chrome" (diterima sebagai "versi apa pun").
    #[test]
    fn impersonation_target_prefers_desktop_chrome() {
        let table = "Client OS Version\nchrome windows 131\nchrome_mobile android 131\n";
        assert_eq!(
            parse_impersonation_target(table).as_deref(),
            Some("chrome")
        );
    }

    /// Hanya keluarga chrome lain (mobile) → tetap "chrome", bukan None:
    /// lebih baik sidik jari chrome mobile daripada tanpa impersonasi.
    #[test]
    fn impersonation_target_falls_back_to_chrome_family() {
        let table = "Client OS Version\nchrome_mobile android 120\n";
        assert_eq!(
            parse_impersonation_target(table).as_deref(),
            Some("chrome")
        );
    }

    /// Tanpa keluarga chrome (atau daftar kosong/keluarannya error) → None:
    /// argumen `--impersonate` dilewati, yt-dlp polos yang jalan.
    #[test]
    fn impersonation_target_absent_without_chrome() {
        assert_eq!(
            parse_impersonation_target("Client OS Version\nsafari macos 18\n"),
            None
        );
        assert_eq!(parse_impersonation_target(""), None);
    }
}
