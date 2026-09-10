use crate::config::Config;
use crate::downloader::DownloadEngine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[derive(Deserialize)]
#[allow(dead_code)]
struct IpcMessage {
    action: String,
    url: Option<String>,
    filename: Option<String>,
    quality: Option<String>,
    id: Option<String>,
    extension_id: Option<String>,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    cookies: Option<String>,
    domain: Option<String>,
}

#[derive(Serialize)]
struct IpcResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

const MAX_REQUEST_LINE: usize = 1024 * 1024;

/// v2.9.4 (C3): allow-list header yang boleh diteruskan extension ke CLI
/// downloader (aria2 `--header=`, yt-dlp `--add-header`).
///
/// Sebelumnya `headers` dari IPC diterima apa adanya — proses lokal mana pun
/// dengan UID sama bisa menyuntikkan header arbitrer ke permintaan unduhan
/// (`Host`, `Content-Length`, `Transfer-Encoding`, `Cookie` sesuka hati).
/// Allow-list (bukan deny-list) dipakai karena extension hanya benar-benar
/// mengirim `Referer` (lihat background.js / content.js / popup.js), sehingga
/// daftar ini menutup celah tanpa mengubah perilaku yang dipakai.
///
/// Dicocokkan case-insensitive — nama header HTTP memang case-insensitive.
///
/// v2.10.0 (B1/B4): `Cookie`, `Authorization`, dan `Proxy-Authorization`
/// DIHAPUS dari daftar. Alasannya dua:
/// 1. **Tidak ada pemakainya.** Extension hanya pernah mengirim `Referer`
///    (`background.js`, `content.js`, `popup.js`). Cookie sudah punya jalur
///    sendiri yang lebih aman: field `cookies`+`domain` → file Netscape
///    per-domain 0600 (`write_cookies_txt`) → `--load-cookies`/`--cookies`/
///    opsi per-URI `cookie`. Lewat header, cookie justru berakhir di argv
///    proses (terbaca di `/proc/<pid>/cmdline`) dan — sebelum v2.10.0 — ikut
///    tertulis ke `session.json`.
/// 2. **Kredensial tidak perlu menempuh jalur ini sama sekali**, jadi
///    permukaan injeksi untuk proses lokal se-UID menyempit tanpa mengubah
///    satu pun perilaku yang dipakai.
///
/// Lapisan kedua tetap ada: `downloader::redact_for_persist` membuang header
/// sensitif dari snapshot session (membersihkan `session.json` warisan
/// ≤2.9.4), dan tiap runner men-strip `\r\n` sebelum membentuk argumen CLI.
///
/// v2.10.0 (B1/B4): `Cookie`, `Authorization`, dan `Proxy-Authorization`
/// DIHAPUS dari daftar. Alasannya dua:
/// 1. **Tidak ada pemakainya.** Extension hanya pernah mengirim `Referer`
///    (`background.js`, `content.js`, `popup.js`). Cookie sudah punya jalur
///    sendiri yang lebih aman: field `cookies`+`domain` → file Netscape
///    per-domain 0600 (`write_cookies_txt`) → `--load-cookies`/`--cookies`/
///    opsi per-URI `cookie`. Lewat header, cookie justru berakhir di argv
///    proses (terbaca di `/proc/<pid>/cmdline`) dan — sebelum v2.10.0 — ikut
///    tertulis ke `session.json`.
/// 2. **Kredensial tidak perlu menempuh jalur ini sama sekali**, jadi
///    permukaan injeksi untuk proses lokal se-UID menyempit tanpa mengubah
///    satu pun perilaku yang dipakai.
///
/// Lapisan kedua tetap ada: `downloader::redact_for_persist` membuang header
/// sensitif dari snapshot session (membersihkan `session.json` warisan
/// ≤2.9.4), dan tiap runner men-strip `\r\n` sebelum membentuk argumen CLI.
pub(crate) const HEADER_ALLOWLIST: &[&str] =
    &["referer", "origin", "accept-language", "user-agent"];

/// Batas jumlah header per permintaan. Dengan allow-list 4 nama saat ini batas
/// ini praktis tak terjangkau (kunci HashMap unik) — dipasang sebagai penjaga
/// bila daftar diperluas, supaya argumen CLI tidak bisa tumbuh tanpa batas.
pub(crate) const MAX_HEADERS: usize = 16;

/// Batas panjang nilai satu header (byte). `User-Agent` dan `Referer` bisa
/// panjang (referer sering membawa query string); 8 KB lebih dari cukup dan
/// menjaga argumen CLI tetap waras.
pub(crate) const MAX_HEADER_VALUE_LEN: usize = 8 * 1024;

/// Bersihkan header masuk sebelum dipakai engine/CLI. Aturan:
/// - nama harus ada di `HEADER_ALLOWLIST` (case-insensitive) — casing asli
///   DIPERTAHANKAN di keluaran,
/// - nama/nilai kosong (atau hanya whitespace) dibuang,
/// - `\r`, `\n`, `\0` dan control char lain dihapus dari nama & nilai
///   (anti header injection — lapisan kedua; downloader juga strip `\r\n`),
/// - nilai dipotong di char boundary bila melebihi `MAX_HEADER_VALUE_LEN`,
/// - jumlah dibatasi `MAX_HEADERS` dengan urutan DETERMINISTIK (nama kunci
///   diurut case-insensitive) supaya hasil tidak bergantung acakan HashMap.
pub(crate) fn sanitize_headers(
    raw: std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut entries: Vec<(String, String)> = raw.into_iter().collect();
    // Urutkan agar pemotongan MAX_HEADERS deterministik & bisa diuji.
    entries.sort_by(|a, b| {
        let ka = a.0.to_ascii_lowercase();
        let kb = b.0.to_ascii_lowercase();
        ka.cmp(&kb).then_with(|| a.0.cmp(&b.0))
    });

    let mut out = std::collections::HashMap::new();
    for (key, value) in entries {
        if out.len() >= MAX_HEADERS {
            tracing::debug!("Header dibuang (melebihi {} entri): {}", MAX_HEADERS, key);
            continue;
        }
        // Buang control char (\r\n = pemisah header) lalu OWS di ujung —
        // whitespace di sekitar nama/nilai header tidak signifikan menurut HTTP.
        let stripped_key = strip_control(&key);
        let stripped_value = strip_control(&value);
        let clean_key = stripped_key.trim();
        let clean_value = stripped_value.trim();
        if clean_key.is_empty() || clean_value.is_empty() {
            continue;
        }
        if !HEADER_ALLOWLIST.contains(&clean_key.to_ascii_lowercase().as_str()) {
            tracing::debug!("Header di luar allow-list dibuang: {}", clean_key);
            continue;
        }
        let clean_value = truncate_chars(clean_value, MAX_HEADER_VALUE_LEN);
        // Kunci casing asli yang dikirim ke CLI (`strip_control`/`trim` tidak
        // mengubah huruf besar/kecil; pencocokan allow-list yang lowercase).
        out.insert(clean_key.to_string(), clean_value);
    }
    out
}

/// Buang karakter control (0x00–0x1f & 0x7f) — termasuk `\r\n` pemisah header.
fn strip_control(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Potong ke maksimal `max` BYTE tanpa memotong char UTF-8 di tengah
/// (slice byte mentah akan panic pada karakter multi-byte).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum RequestLine {
    Eof,
    Value(String),
    TooLarge,
}

/// Baca maksimal `limit + 1` byte. Berbeda dari `read_line` langsung, batas
/// ini diterapkan SAAT membaca sehingga peer tidak dapat memaksa Vec tumbuh
/// tanpa batas sebelum pemeriksaan ukuran dijalankan.
async fn read_request_line<R>(reader: R, limit: usize) -> std::io::Result<RequestLine>
where
    R: AsyncRead + Unpin,
{
    let mut limited = BufReader::new(reader).take(limit.saturating_add(1) as u64);
    let mut line = String::new();
    let read = limited.read_line(&mut line).await?;
    if read == 0 {
        Ok(RequestLine::Eof)
    } else if line.len() > limit {
        Ok(RequestLine::TooLarge)
    } else {
        Ok(RequestLine::Value(line))
    }
}

/// v2.3.0 (K1): socket pindah dari /tmp publik ke direktori privat
/// (`XDG_RUNTIME_DIR/fast-dm` bila valid, fallback `~/.config/fast-dm/run`).
/// Path lama yang ditinggalkan versi ≤2.2.5 dibersihkan saat start, bila
/// memang socket milik user kita sendiri (jangan sentuh symlink/sock orang lain).
fn cleanup_legacy_socket() {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let old = PathBuf::from(format!(
        "/tmp/fast-dm-{}.sock",
        nix::unistd::getuid().as_raw()
    ));
    if let Ok(md) = std::fs::symlink_metadata(&old) {
        if md.file_type().is_socket() && md.uid() == nix::unistd::getuid().as_raw() {
            let _ = std::fs::remove_file(&old);
        }
    }
}

/// Defense-in-depth: terima koneksi HANYA dari proses dengan UID yang sama.
/// Permission socket 0600 sudah membatasi, tapi bila parent dir pernah salah
/// mode (mis. hasil versi lama / home di-share), peer-cred tetap menutup celah.
///
/// v2.10.0 (A5): memakai `geteuid()` — identitas EFEKTIF, sama seperti
/// `config::validated_runtime_dir()`. Sebelumnya fungsi ini membandingkan
/// dengan `getuid()` sementara pemeriksaan path privat memakai `geteuid()`;
/// keduanya identik untuk proses non-setuid, tapi dua keputusan keamanan yang
/// berbeda tidak boleh memakai identitas yang berbeda.
///
/// (Catatan: `cleanup_legacy_socket` di atas sengaja TETAP memakai `getuid()`
/// — ia merekonstruksi path warisan ≤2.2.5 yang dulu memang dibentuk dari
/// `getuid()`, jadi menggantinya justru membuat socket lama tidak ditemukan.)
fn peer_uid_ok(stream: &tokio::net::UnixStream) -> bool {
    match nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials) {
        // Ucred::uid() mengembalikan uid_t (u32), bukan Uid — bandingkan raw.
        Ok(cred) => cred.uid() == nix::unistd::geteuid().as_raw(),
        Err(_) => false,
    }
}

/// Backoff untuk `accept()` yang gagal beruntun: 50 ms, 100, 200, 400, 800,
/// 1600, lalu di-cap 2000 ms. Murni (tanpa I/O) supaya bisa di-unit test.
///
/// Cap 2 dtk dipilih agar server tetap terasa "hidup" untuk browser (native
/// host punya timeout 5 dtk di `forward_to_gui`) sementara loop tidak sibuk
/// berputar saat penyebabnya permanen.
pub(crate) fn accept_backoff(consecutive_failures: u32) -> std::time::Duration {
    const BASE_MS: u64 = 50;
    const MAX_MS: u64 = 2_000;
    // exponent dibatasi 6 (BASE_MS << 6 = 3200) lalu di-cap MAX_MS —
    // saturating di dua tempat agar u32 besar tidak overflow shift.
    let exp = consecutive_failures.saturating_sub(1).min(6);
    let ms = BASE_MS.saturating_mul(1u64 << exp).min(MAX_MS);
    std::time::Duration::from_millis(ms)
}

pub async fn start_server(engine: Arc<DownloadEngine>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = Config::ipc_socket_path();
    cleanup_legacy_socket();

    // Single-instance: kalau ada instance lain yang masih melayani socket ini,
    // jangan ambil alih. Kalau tidak, instance kedua menghapus socket instance
    // pertama dan semua koneksi browser masuk ke instance yang salah.
    if std::os::unix::net::UnixStream::connect(&socket_path).is_ok() {
        tracing::info!(
            "IPC already active on {} — skip server",
            socket_path.display()
        );
        return Ok(());
    }

    // Socket basi (instance lama sudah mati) → bersihkan lalu bind
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;

    // Set permissions 0600
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

    tracing::info!("IPC listening on {}", socket_path.display());

    // v2.9.4: error `accept()` TIDAK boleh mematikan server. Dulu `?` langsung
    // mempropagasi keluar dari `start_server`, dan pemanggilnya hanya menulis
    // log — akibatnya satu error transien (fd habis = EMFILE/ENFILE, ENOBUFS,
    // ECONNABORTED) mematikan IPC untuk SELURUH sesi aplikasi: extension tidak
    // akan pernah bisa mencapai GUI lagi sampai app di-restart. Sekarang setiap
    // error dicatat lalu dicoba lagi dengan backoff eksponensial terbatas.
    let mut accept_failures = 0u32;
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => {
                accept_failures = 0;
                pair
            }
            Err(e) => {
                accept_failures += 1;
                let delay = accept_backoff(accept_failures);
                tracing::warn!(
                    "IPC accept gagal ({e}) — mencoba lagi dalam {} ms (kegagalan beruntun #{})",
                    delay.as_millis(),
                    accept_failures
                );
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        if !peer_uid_ok(&stream) {
            tracing::warn!("IPC: koneksi ditolak (peer UID != uid kita) — ditutup");
            drop(stream); // close
            continue;
        }

        let engine = engine.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();

            let line = match read_request_line(reader, MAX_REQUEST_LINE).await {
                Ok(RequestLine::Value(line)) => line,
                Ok(RequestLine::Eof) | Err(_) => return,
                Ok(RequestLine::TooLarge) => {
                    let response = IpcResponse {
                        success: false,
                        id: None,
                        error: Some("Request terlalu besar".into()),
                        message: None,
                    };
                    let json = serde_json::to_string(&response).unwrap_or_default();
                    let _ = writer.write_all(json.as_bytes()).await;
                    let _ = writer.write_all(b"\n").await;
                    return;
                }
            };

            let response = match serde_json::from_str::<IpcMessage>(&line) {
                Ok(msg) => handle_message(msg, &engine).await,
                Err(e) => IpcResponse {
                    success: false,
                    id: None,
                    error: Some(e.to_string()),
                    message: None,
                },
            };

            let json = serde_json::to_string(&response).unwrap_or_default();
            let _ = writer.write_all(json.as_bytes()).await;
            let _ = writer.write_all(b"\n").await;
        });
    }
}

async fn handle_message(msg: IpcMessage, engine: &DownloadEngine) -> IpcResponse {
    match msg.action.as_str() {
        "download" => {
            let url = match msg.url {
                Some(u) if !u.is_empty() => u,
                _ => {
                    return IpcResponse {
                        success: false,
                        id: None,
                        error: Some("No URL".into()),
                        message: None,
                    }
                }
            };

            // v2.10.0 (A2): tolak skema yang tidak bisa diunduh SEBELUM
            // membuat item. Dulu `add_download` tetap mengembalikan id (dengan
            // status Error di dalamnya) dan handler ini selalu membalas
            // `success: true` — extension lalu menampilkan badge ⬇ "sukses"
            // untuk `blob:`/`data:`/`file:` yang tidak akan pernah terunduh.
            // Guard di dalam engine tetap dipertahankan (dipakai jalur GUI).
            if !crate::downloader::is_supported_scheme(&url) {
                const REJECTED: &str = "Skema URL tidak didukung — http, https, ftp, atau magnet.";
                return IpcResponse {
                    success: false,
                    id: None,
                    error: Some(REJECTED.into()),
                    message: None,
                };
            }

            // Tulis cookies.txt SEBELUM download start (yt-dlp membacanya saat spawn)
            if let (Some(c), Some(d)) = (msg.cookies.as_deref(), msg.domain.as_deref()) {
                if let Err(e) = write_cookies_txt(c, d) {
                    tracing::warn!("set cookies: {}", e);
                }
            }

            // v2.9.4 (C3): header disaring di boundary ini — sebelum mencapai
            // engine dan argumen CLI aria2/yt-dlp. Lihat `sanitize_headers`.
            let headers = sanitize_headers(msg.headers);

            let id = engine
                .add_download(
                    &url,
                    msg.filename.as_deref(),
                    None,
                    true,
                    headers,
                    msg.quality,
                )
                .await;

            IpcResponse {
                success: true,
                id: Some(id),
                error: None,
                message: None,
            }
        }

        "ping" => IpcResponse {
            success: true,
            id: None,
            error: None,
            message: Some("running".into()),
        },

        "pause" => {
            if let Some(id) = msg.id {
                engine.pause_download(&id).await;
            }
            IpcResponse {
                success: true,
                id: None,
                error: None,
                message: None,
            }
        }

        "resume" => {
            if let Some(id) = msg.id {
                engine.resume_download(&id).await;
            }
            IpcResponse {
                success: true,
                id: None,
                error: None,
                message: None,
            }
        }

        "cancel" => {
            if let Some(id) = msg.id {
                engine.cancel_download(&id).await;
            }
            IpcResponse {
                success: true,
                id: None,
                error: None,
                message: None,
            }
        }

        "list" => {
            let downloads = engine.get_all_downloads().await;
            let list: Vec<serde_json::Value> = downloads
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "url": d.url,
                        "filename": d.filename,
                        "status": d.status.to_string(),
                        "progress": d.progress,
                        "speed": d.speed,
                        "total_size": d.total_size,
                        "downloaded": d.downloaded,
                        "error_msg": d.error_msg,
                    })
                })
                .collect();

            IpcResponse {
                success: true,
                id: None,
                error: None,
                message: Some(serde_json::to_string(&list).unwrap_or_default()),
            }
        }

        "register" => {
            if let Some(ext_id) = msg.extension_id {
                match crate::native_host::setup::register_extension_id(&ext_id) {
                    Ok(count) => IpcResponse {
                        success: true,
                        id: None,
                        error: None,
                        message: Some(format!("Registered {} manifests", count)),
                    },
                    Err(e) => IpcResponse {
                        success: false,
                        id: None,
                        error: Some(e.to_string()),
                        message: None,
                    },
                }
            } else {
                IpcResponse {
                    success: false,
                    id: None,
                    error: Some("No extension_id".into()),
                    message: None,
                }
            }
        }

        _ => IpcResponse {
            success: false,
            id: None,
            error: Some(format!("Unknown action: {}", msg.action)),
            message: None,
        },
    }
}

/// Konversi cookie string browser ("k=v; k=v") → file Netscape per-domain
/// untuk yt-dlp/aria2 (B7: per-domain agar tidak saling menimpa)
fn write_cookies_txt(cookie_header: &str, domain: &str) -> Result<(), String> {
    if cookie_header.len() > 256 * 1024 {
        return Err("cookies too large".into());
    }
    let host = domain.trim().trim_start_matches("www.");
    if host.is_empty() || host.chars().any(|c| c.is_whitespace()) {
        return Err("invalid domain".into());
    }

    // v2.3.0 (M7): TTL 24 jam — dulu 1 tahun (!) padahal ini salinan sesi
    // browser; GC engine (7 hari) + kedaluwarsa mandiri menjamin tidak ada
    // kredensial basi menumpuk di disk. Cookie session browser memang pendek
    // umurnya, 24 jam lebih dari cukup untuk menyelesaikan unduhan.
    let expires = chrono::Utc::now().timestamp() + 24 * 3600;
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    let mut count = 0;

    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = name.trim().replace(['\t', '\r', '\n'], "");
        let value = value.trim().replace(['\t', '\r', '\n'], "");
        if name.is_empty() {
            continue;
        }
        out.push_str(&format!(
            ".{}\tTRUE\t/\tFALSE\t{}\t{}\t{}\n",
            host, expires, name, value
        ));
        count += 1;
    }

    if count == 0 {
        return Err("no cookies".into());
    }

    let path = Config::cookies_file_for(host);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;

    // Cookies = rahasia → 0600
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    #[tokio::test]
    async fn request_line_accepts_value_within_limit() {
        let input = std::io::Cursor::new(b"{\"action\":\"ping\"}\n".to_vec());
        let got = read_request_line(input, 64).await.unwrap();
        assert_eq!(got, RequestLine::Value("{\"action\":\"ping\"}\n".into()));
    }

    #[tokio::test]
    async fn request_line_rejects_before_unbounded_growth() {
        let input = std::io::Cursor::new(vec![b'x'; 65]);
        let got = read_request_line(input, 64).await.unwrap();
        assert_eq!(got, RequestLine::TooLarge);
    }

    #[tokio::test]
    async fn request_line_reports_clean_eof() {
        let input = std::io::Cursor::new(Vec::<u8>::new());
        let got = read_request_line(input, 64).await.unwrap();
        assert_eq!(got, RequestLine::Eof);
    }

    // ── v2.9.4: accept_backoff — IPC server tidak boleh mati saat accept error ──

    #[test]
    fn accept_backoff_grows_exponentially() {
        assert_eq!(accept_backoff(1), Duration::from_millis(50));
        assert_eq!(accept_backoff(2), Duration::from_millis(100));
        assert_eq!(accept_backoff(3), Duration::from_millis(200));
        assert_eq!(accept_backoff(4), Duration::from_millis(400));
        assert_eq!(accept_backoff(5), Duration::from_millis(800));
        assert_eq!(accept_backoff(6), Duration::from_millis(1600));
    }

    #[test]
    fn accept_backoff_caps_and_never_overflows() {
        // 50 << 6 = 3200 → di-cap 2000. Counter u32 besar tidak boleh panic
        // (shift overflow) karena kegagalan bisa menumpuk lama.
        assert_eq!(accept_backoff(7), Duration::from_millis(2000));
        assert_eq!(accept_backoff(100), Duration::from_millis(2000));
        assert_eq!(accept_backoff(u32::MAX), Duration::from_millis(2000));
    }

    #[test]
    fn accept_backoff_zero_is_not_busy_loop() {
        // Pemanggil menaikkan counter sebelum memanggil, jadi 0 tidak terjadi —
        // tetap harus > 0 agar tidak pernah jadi busy-loop.
        assert!(accept_backoff(0) > Duration::ZERO);
    }

    #[test]
    fn accept_backoff_never_decreases() {
        let mut prev = Duration::ZERO;
        for n in 1..=20u32 {
            let d = accept_backoff(n);
            assert!(d >= prev, "backoff menurun pada n={n}: {d:?} < {prev:?}");
            prev = d;
        }
    }

    // ── v2.9.4 (C3): sanitize_headers — allow-list di boundary IPC ──

    fn headers_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .copied()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn every_allowlist_entry_is_accepted() {
        // Penjaga typo: setiap nama di HEADER_ALLOWLIST harus benar-benar lolos
        // filter (sudah lowercase, tanpa salah eja) — kalau tidak, header sah
        // ikut terbuang diam-diam.
        let raw: HashMap<String, String> = HEADER_ALLOWLIST
            .iter()
            .map(|n| ((*n).to_string(), "v".to_string()))
            .collect();
        let got = sanitize_headers(raw);
        assert_eq!(got.len(), HEADER_ALLOWLIST.len(), "got {got:?}");
    }

    #[test]
    fn sanitize_keeps_allowlisted_headers_case_insensitively() {
        let raw = [
            ("Referer", "https://site.test/page"),
            ("ORIGIN", "https://site.test"),
            ("accept-language", "id-ID"),
            ("User-Agent", "Mozilla/5.0"),
        ];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got.len(), 4, "got {got:?}");
        assert_eq!(got["Referer"], "https://site.test/page");
        // Casing asli DIPERTAHANKAN — pencocokan allow-list yang lowercase,
        // bukan namanya yang dinormalkan.
        assert_eq!(got["ORIGIN"], "https://site.test");
        assert_eq!(got["accept-language"], "id-ID");
    }

    #[test]
    fn sanitize_drops_headers_outside_allowlist() {
        let raw = [
            ("Host", "evil.test"),
            ("Content-Length", "0"),
            ("Connection", "keep-alive"),
            ("Transfer-Encoding", "chunked"),
            ("X-Injected", "yes"),
            ("Cookie2", "nope"),
            ("Referer", "https://ok.test/"),
        ];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got.len(), 1, "hanya Referer yang boleh lolos: {got:?}");
        assert_eq!(got["Referer"], "https://ok.test/");
    }

    #[test]
    fn sanitize_strips_header_injection_from_value() {
        let raw = [("Referer", "https://ok.test/\r\nX-Injected: yes")];
        let got = sanitize_headers(headers_of(&raw));
        let v = &got["Referer"];
        assert!(!v.contains('\r') && !v.contains('\n'), "got {v:?}");
        assert_eq!(v, "https://ok.test/X-Injected: yes");
    }

    #[test]
    fn sanitize_strips_control_chars_from_key() {
        // "Ref\r\nerer" → "Referer" setelah control char dibuang → lolos.
        let raw = [("Ref\r\nerer", "x")];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got.get("Referer").map(String::as_str), Some("x"));
    }

    #[test]
    fn sanitize_drops_empty_key_or_value() {
        let raw = [
            ("", "value"),
            ("Referer", ""),
            ("Origin", "   "),
            ("Accept-Language", "id"),
        ];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got.len(), 1, "got {got:?}");
        assert!(got.contains_key("Accept-Language"));
    }

    #[test]
    fn sanitize_truncates_overlong_value() {
        let long = "a".repeat(MAX_HEADER_VALUE_LEN + 500);
        let raw = [("Referer", long.as_str())];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got["Referer"].len(), MAX_HEADER_VALUE_LEN);
    }

    #[test]
    fn sanitize_truncates_on_utf8_boundary() {
        // 'é' = 2 byte. Nilai melewati batas byte tidak boleh terpotong di
        // tengah char (slice byte mentah akan panic).
        let long = "é".repeat(MAX_HEADER_VALUE_LEN);
        let raw = [("Referer", long.as_str())];
        let got = sanitize_headers(headers_of(&raw));
        let v = &got["Referer"];
        assert!(v.len() <= MAX_HEADER_VALUE_LEN, "len {}", v.len());
        assert_eq!(v.len() % 2, 0, "harus berhenti di batas char");
        assert!(v.chars().all(|c| c == 'é'));
    }

    #[test]
    fn truncate_chars_short_input_untouched() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("", 10), "");
        assert_eq!(truncate_chars("abcdef", 3), "abc");
    }

    // ── v2.10.0 (B1/B4): kredensial tidak lagi lewat jalur header ──

    #[test]
    fn sanitize_drops_credential_headers() {
        // Cookie punya jalur sendiri yang lebih aman (field `cookies`+`domain`
        // → file Netscape 0600 → --load-cookies). Lewat header ia berakhir di
        // argv proses (terbaca di /proc/<pid>/cmdline) dan — sebelum v2.10.0 —
        // ikut tertulis ke session.json.
        let raw = [
            ("Cookie", "SID=super-rahasia; HSID=x"),
            ("Authorization", "Bearer token-rahasia"),
            ("Proxy-Authorization", "Basic abc"),
            ("Referer", "https://ok.test/page"),
        ];
        let got = sanitize_headers(headers_of(&raw));
        assert_eq!(got.len(), 1, "got {got:?}");
        assert_eq!(got["Referer"], "https://ok.test/page");
        // Pastikan tidak lolos dalam casing apa pun.
        let raw2 = headers_of(&[("COOKIE", "a=b"), ("authorization", "x")]);
        assert!(sanitize_headers(raw2).is_empty());
    }

    #[test]
    fn allowlist_has_no_credential_entries() {
        // Penjaga arah: jangan diam-diam menambahkan kredensial kembali.
        for banned in ["cookie", "authorization", "proxy-authorization"] {
            assert!(
                !HEADER_ALLOWLIST.contains(&banned),
                "{banned} masuk allow-list lagi — lihat komentar HEADER_ALLOWLIST"
            );
        }
        // Yang memang dibutuhkan extension tetap ada.
        assert!(HEADER_ALLOWLIST.contains(&"referer"));
    }

    #[test]
    fn truncate_chars_backs_off_to_boundary() {
        // 'é' = 2 byte: batas 3 mundur ke 2, bukan memotong di tengah.
        assert_eq!(truncate_chars("aébc", 3), "aé");
        // '🦀' = 4 byte: batas 6 mundur ke 4.
        assert_eq!(truncate_chars("🦀🦀", 6), "🦀");
        assert_eq!(truncate_chars("🦀🦀", 4), "🦀");
    }

    #[test]
    fn strip_control_removes_cr_lf_nul_tab_del() {
        assert_eq!(strip_control("a\r\nb"), "ab");
        assert_eq!(strip_control("a\0b"), "ab");
        assert_eq!(strip_control("a\tb"), "ab");
        assert_eq!(strip_control("a\x7fb"), "ab");
        // spasi BUKAN control char — harus tetap ada
        assert_eq!(strip_control("normal text 123"), "normal text 123");
    }
}
