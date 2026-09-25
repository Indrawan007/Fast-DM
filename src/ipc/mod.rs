use crate::config::Config;
use crate::downloader::DownloadEngine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn default_cookie_path() -> String {
    // Missing path must not widen a cookie to every URL on the host.
    String::new()
}

fn default_cookie_secure() -> bool {
    // Missing Secure metadata fails closed; an explicit `false` is preserved.
    true
}

fn default_cookie_host_only() -> bool {
    // Omit metadata must fail closed: a cookie tanpa hostOnly tidak boleh
    // berubah menjadi domain-wide hanya karena client mengirim field parsial.
    true
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BrowserCookie {
    pub(crate) name: String,
    pub(crate) value: String,
    #[serde(default)]
    pub(crate) domain: String,
    #[serde(default = "default_cookie_path")]
    pub(crate) path: String,
    #[serde(default = "default_cookie_secure")]
    pub(crate) secure: bool,
    #[serde(rename = "hostOnly", default = "default_cookie_host_only")]
    pub(crate) host_only: bool,
    #[serde(rename = "expirationDate", default)]
    pub(crate) expiration_date: Option<f64>,
}

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
    cookies: Option<Vec<BrowserCookie>>,
    // Dipertahankan untuk kompatibilitas payload, tetapi domain dari client
    // tidak dipercaya; writer memvalidasi metadata terhadap URL request.
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
///    proses aria2 terpisah. Lewat header, cookie justru berakhir di argv
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

/// Batas panjang nilai `quality`. Preset terpanjang kita `"audio_best"` = 10
/// char; id format yt-dlp dibatasi 32 char oleh `looks_like_format_id`. 64
/// memberi ruang longgar sekaligus menutup nilai tak terbatas.
pub(crate) const MAX_QUALITY_LEN: usize = 64;

/// v3.2.3 (A5): saring field `quality` yang datang dari IPC.
///
/// Ini **lapisan kedua**, bukan satu-satunya: pemetaan sesungguhnya ada di
/// `youtube::quality_args`, dan nilai yang tidak dikenal sudah dipetakan ke
/// selector default yang aman. Tetapi `quality` adalah satu-satunya string bebas
/// dari extension yang sampai ke boundary ini tanpa penyaring sama sekali
/// (bandingkan `HEADER_ALLOWLIST` untuk header), dan ia berakhir sebagai nilai
/// `--format` yt-dlp. Whitelist `looks_like_format_id` sendiri masih menerima
/// `/ * [ ] ( ) > < ^ & | , = !`, jadi penjaga murah di sini mempersempit
/// permukaan tanpa mengubah satu pun nilai yang dipakai extension
/// (`best_mp4`, `2160p`…`360p`, `audio_best`, `audio_mp3`, atau id format nyata).
///
/// Aturan: ada isi setelah trim, ≤ `MAX_QUALITY_LEN` byte, tanpa whitespace,
/// control char, kutip, backslash, atau newline. Nilai yang ditolak
/// dikembalikan sebagai `None` sehingga unduhan TETAP jalan dengan kualitas
/// default — menolak seluruh unduhan akan lebih buruk daripada mengabaikan
/// preferensi kualitas yang cacat.
pub(crate) fn sanitize_quality(raw: Option<String>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_QUALITY_LEN {
        return None;
    }
    let clean: String = trimmed
        .chars()
        .filter(|c| {
            !c.is_control()
                && !c.is_whitespace()
                && !matches!(c, '"' | '\'' | '`' | '\\' | '\n' | '\r')
        })
        .collect();
    if clean.is_empty() || clean != trimmed {
        return None;
    }
    Some(clean)
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

fn control_response(action: &str, id: &str, accepted: bool) -> IpcResponse {
    IpcResponse {
        success: accepted,
        id: Some(id.to_string()),
        error: (!accepted)
            .then(|| format!("Download tidak ditemukan atau tidak dapat di-{action}: {id}")),
        message: None,
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
                // v3.0.0: magnet dihapus. Pesan TIDAK lagi disalin di sini —
                // pakai konstanta yang sama dengan engine
                // (`downloader::UNSUPPORTED_SCHEME_MSG`) supaya keduanya tidak
                // bisa melenceng lagi seperti sebelumnya.
                return IpcResponse {
                    success: false,
                    id: None,
                    error: Some(crate::downloader::UNSUPPORTED_SCHEME_MSG.into()),
                    message: None,
                };
            }

            // Tulis cookies.txt SEBELUM download start (yt-dlp membacanya saat
            // spawn). Metadata browser divalidasi ulang terhadap URL; field
            // `domain` dari client tidak dipakai sebagai sumber kebenaran.
            if let Some(cookies) = msg.cookies.as_deref() {
                if let Err(e) = write_cookies_txt(cookies, &url) {
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
                    // v3.2.3 (A5): disaring di boundary — lihat `sanitize_quality`.
                    sanitize_quality(msg.quality),
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

        "pause" | "resume" | "cancel" => {
            let Some(id) = msg.id.as_deref() else {
                return IpcResponse {
                    success: false,
                    id: None,
                    error: Some(format!("No ID untuk aksi {}", msg.action)),
                    message: None,
                };
            };
            let accepted = match msg.action.as_str() {
                "pause" => engine.pause_download(id).await,
                "resume" => engine.resume_download(id).await,
                "cancel" => engine.cancel_download(id).await,
                _ => false,
            };
            control_response(&msg.action, id, accepted)
        }

        // v3.3.2: extension bertanya apakah unduhan yang ia cegat harus
        // dikembalikan ke browser (server menolak Fast-DM). `message` =
        // "pending" | "handback" | "done" — lihat `HandbackState`.
        "handback" => {
            let Some(id) = msg.id.as_deref() else {
                return IpcResponse {
                    success: false,
                    id: None,
                    error: Some("No ID untuk aksi handback".into()),
                    message: None,
                };
            };
            let state = engine.poll_browser_handback(id).await;
            IpcResponse {
                success: true,
                id: Some(id.to_string()),
                error: None,
                message: Some(state.as_str().into()),
            }
        }

        "list" => {
            let downloads = engine.get_all_downloads().await;
            let list: Vec<serde_json::Value> = downloads
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        // Jangan kirim query/path URL mentah: signed URL sering
                        // memuat token akses dan respons `list` diteruskan ke
                        // extension/browser caller.
                        "url": redact_url_for_ipc(&d.url),
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

/// Tulis metadata cookie browser ke file Netscape per-domain.
///
/// Format lama menerima string `k=v; k=v` lalu memperlebar semua cookie ke
/// `.host`, `/`, dan `Secure=FALSE`. Itu tidak aman: host-only cookie dapat
/// bocor ke sibling subdomain dan cookie HTTPS dapat turun ke HTTP. Sekarang
/// metadata dari `chrome.cookies.getAll()` dipertahankan dan file ditulis
/// atomik agar downloader tidak pernah membaca file setengah jadi.
fn write_cookies_txt(cookies: &[BrowserCookie], request_url: &str) -> Result<(), String> {
    write_cookies_txt_in(cookies, request_url, &Config::config_dir())
}

/// v3.2.3 (A2): inti `write_cookies_txt` dengan direktori config
/// parameterisasi — sama seperti pola `config_dir_from`/`rpc_secret_in`, supaya
/// perilaku "tidak ada cookie yang cocok" bisa di-unit test tanpa menyentuh
/// `~/.config` user nyata.
fn write_cookies_txt_in(
    cookies: &[BrowserCookie],
    request_url: &str,
    config_dir: &std::path::Path,
) -> Result<(), String> {
    const MAX_COOKIES: usize = 512;
    const SESSION_COOKIE_TTL: i64 = 24 * 3600;

    if cookies.len() > MAX_COOKIES {
        return Err("too many cookies".into());
    }

    let parsed = url::Url::parse(request_url).map_err(|_| "invalid cookie URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("cookies require an HTTP(S) URL".into());
    }
    let request_host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "invalid cookie host".to_string())?;
    let file_host = request_host.trim_start_matches("www.");
    let now = chrono::Utc::now().timestamp();
    let request_path = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };

    let mut out = format!(
        "# Netscape HTTP Cookie File\n{}\n",
        crate::config::COOKIE_FILE_HEADER
    );
    let mut count = 0usize;
    let path = Config::cookies_file_in_host(config_dir, file_host);

    if cookies.is_empty() {
        clear_cookie_file(&path)?;
        return Ok(());
    }

    for cookie in cookies {
        let raw_domain = cookie.domain.trim().to_ascii_lowercase();
        let cookie_domain = raw_domain.trim_start_matches('.');
        if cookie_domain.is_empty()
            || !cookie_domain_matches_host(cookie_domain, &request_host)
            || !valid_cookie_field(cookie_domain, false)
        {
            continue;
        }

        let path = if cookie.path.starts_with('/') && valid_cookie_field(&cookie.path, true) {
            cookie.path.clone()
        } else {
            continue;
        };
        if !cookie_path_matches(request_path, &path) {
            continue;
        }
        if cookie.secure && parsed.scheme() != "https" {
            continue;
        }

        let expires = match cookie.expiration_date {
            Some(value) if value.is_finite() && value > 0.0 => {
                let value = value.floor();
                if value > i64::MAX as f64 {
                    i64::MAX
                } else {
                    value as i64
                }
            }
            Some(_) => continue,
            // Netscape's zero means a session cookie. Give the local copy a
            // bounded lifetime so it cannot survive indefinitely on disk.
            None => now.saturating_add(SESSION_COOKIE_TTL),
        };
        if expires > 0 && expires <= now {
            continue;
        }

        let name = strip_cookie_field(&cookie.name);
        let value = strip_cookie_field(&cookie.value);
        if !valid_cookie_field(&name, false) || !valid_cookie_field(&value, true) {
            continue;
        }

        // `hostOnly=true` means no leading dot and FALSE in the include-
        // subdomains column. Domain cookies retain their original scope.
        let include_subdomains = !cookie.host_only;
        let output_domain = if include_subdomains {
            format!(".{}", cookie_domain)
        } else {
            cookie_domain.to_string()
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            output_domain,
            if include_subdomains { "TRUE" } else { "FALSE" },
            path,
            if cookie.secure { "TRUE" } else { "FALSE" },
            expires,
            name,
            value
        ));
        count += 1;
    }

    if count == 0 {
        // v3.2.3 (A2): JANGAN hapus jar yang sudah ada. Dulu cabang ini
        // memanggil `clear_cookie_file(&path)` — padahal pemanggilnya
        // (`handle_message`) hanya mencatat `tracing::warn!` lalu TETAP
        // melanjutkan unduhan. Akibatnya satu unduhan yang cookienya tersaring
        // habis (host/path/Secure tidak cocok dengan URL request) menghapus
        // `cookies_<host>.txt` milik unduhan lain yang baru saja login, dan
        // unduhan berikutnya dari host yang sama kehilangan kredensial secara
        // diam-diam. Tidak menulis apa pun adalah hasil yang benar di sini:
        // jar lama dibiarkan sampai writer berikutnya menggantinya secara
        // atomik, atau `Config::gc_stale_cookies` membersihkannya (>7 hari).
        return Err("no cookies matching request URL".into());
    }

    let dir = path
        .parent()
        .ok_or_else(|| "cookie path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // Unique temporary file + rename: concurrent writers may replace one
    // another, but readers never observe a truncated cookie file.
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let temp = path.with_file_name(format!(".cookies-{}.tmp-{}", file_host, nonce));
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    if let Err(e) = file.write_all(out.as_bytes()).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.to_string());
    }
    drop(file);
    if let Err(e) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.to_string());
    }

    Ok(())
}

fn clear_cookie_file(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn strip_cookie_field(value: &str) -> String {
    value.trim().replace(['\t', '\r', '\n'], "")
}

fn valid_cookie_field(value: &str, is_value: bool) -> bool {
    (!value.is_empty() || is_value)
        && !value.chars().any(|ch| {
            ch.is_control() || (!is_value && (ch == '=' || ch == ';' || ch.is_whitespace()))
        })
        && (!is_value || !value.contains(';'))
}

fn cookie_domain_matches_host(cookie_domain: &str, host: &str) -> bool {
    host == cookie_domain || host.ends_with(&format!(".{}", cookie_domain))
}

fn cookie_path_matches(request_path: &str, cookie_path: &str) -> bool {
    if cookie_path == "/" || request_path == cookie_path {
        return true;
    }
    request_path.starts_with(cookie_path)
        && (cookie_path.ends_with('/')
            || request_path
                .as_bytes()
                .get(cookie_path.len())
                .is_some_and(|b| *b == b'/'))
}

/// URL yang keluar lewat IPC hanya perlu identitas host untuk status.
/// Query, fragment, userinfo, dan path sengaja dihapus karena URL download
/// dapat berisi signed token — termasuk token yang ditempatkan di path.
fn redact_url_for_ipc(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return "[URL disembunyikan]".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
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
    fn cookie_scope_helpers_fail_closed() {
        assert!(cookie_domain_matches_host("example.com", "www.example.com"));
        assert!(!cookie_domain_matches_host("example.com", "notexample.com"));
        assert!(cookie_path_matches("/private/file", "/private"));
        assert!(!cookie_path_matches("/private-file", "/private"));
        assert!(valid_cookie_field("sid", false));
        assert!(!valid_cookie_field("sid=other", false));
        assert!(!valid_cookie_field("a;b", true));
        assert!(!valid_cookie_field("a\0b", true));
    }

    #[test]
    fn redact_url_removes_credentials_and_tokens() {
        assert_eq!(
            redact_url_for_ipc(
                "https://user:password@example.com/private/token.mp4?sig=secret#fragment"
            ),
            "https://example.com/"
        );
        assert_eq!(redact_url_for_ipc("not a URL"), "[URL disembunyikan]");
    }

    #[test]
    fn control_response_reports_rejected_action() {
        let response = control_response("pause", "missing", false);
        assert!(!response.success);
        assert_eq!(response.id.as_deref(), Some("missing"));
        assert!(response
            .error
            .as_deref()
            .is_some_and(|message| message.contains("pause")));

        let response = control_response("cancel", "known", true);
        assert!(response.success);
        assert!(response.error.is_none());
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

    /// v3.2.3 (A5): nilai `quality` yang benar-benar dipakai extension harus
    /// lolos apa adanya — penjaga boundary tidak boleh mengubah perilaku.
    #[test]
    fn sanitize_quality_keeps_every_value_the_extension_sends() {
        for preset in [
            "best_mp4",
            "2160p",
            "1440p",
            "1080p",
            "720p",
            "480p",
            "360p",
            "audio_best",
            "audio_mp3",
        ] {
            assert_eq!(
                sanitize_quality(Some(preset.to_string())).as_deref(),
                Some(preset),
                "preset dialog/overlay harus lolos"
            );
        }
        // id format nyata dari `yt-dlp -J` (D6) — termasuk bentuk gabungan
        assert_eq!(
            sanitize_quality(Some("137+140".to_string())).as_deref(),
            Some("137+140")
        );
        assert_eq!(
            sanitize_quality(Some("bestvideo[height<=1080]".to_string())).as_deref(),
            Some("bestvideo[height<=1080]")
        );
    }

    /// v3.2.3 (A5): nilai cacat → `None` (unduhan tetap jalan dengan kualitas
    /// default), bukan diteruskan ke `--format`.
    #[test]
    fn sanitize_quality_rejects_malformed_values() {
        assert_eq!(sanitize_quality(None), None);
        assert_eq!(sanitize_quality(Some(String::new())), None);
        assert_eq!(sanitize_quality(Some("   ".to_string())), None);
        // whitespace di tengah = dua argumen berbeda bagi mata user
        assert_eq!(sanitize_quality(Some("best video".to_string())), None);
        // kutip / backslash / control char
        assert_eq!(sanitize_quality(Some("a\"b".to_string())), None);
        assert_eq!(sanitize_quality(Some("a'b".to_string())), None);
        assert_eq!(sanitize_quality(Some("a\\b".to_string())), None);
        assert_eq!(sanitize_quality(Some("a\r\nb".to_string())), None);
        // lebih dari MAX_QUALITY_LEN byte
        assert_eq!(
            sanitize_quality(Some("x".repeat(MAX_QUALITY_LEN + 1))),
            None
        );
        assert_eq!(
            sanitize_quality(Some("x".repeat(MAX_QUALITY_LEN))).as_deref(),
            Some("x".repeat(MAX_QUALITY_LEN).as_str()),
            "tepat di batas harus lolos"
        );
    }

    /// v3.2.3 (A2): cabang "tidak ada cookie yang cocok" TIDAK boleh lagi
    /// menghapus jar milik host itu. Test ini mengunci bahwa jar yang sudah
    /// ada tetap utuh; pencabutan kredensial hanya terjadi lewat jalur
    /// "client mengirim array kosong" (`cookies.is_empty()`), yang memang
    /// niat eksplisit extension.
    #[test]
    fn no_matching_cookie_does_not_wipe_the_jar() {
        // Simulasi: jar sudah ada untuk host lain; tulis untuk URL yang
        // cookienya tersaring habis tidak boleh menyentuhnya.
        let dir = std::env::temp_dir().join(format!("fastdm-a2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let jar = dir.join("cookies_example.com.txt");
        std::fs::write(&jar, "# Netscape HTTP Cookie File\n").unwrap();

        // Cookie untuk host yang TIDAK cocok dengan URL request → count == 0
        let cookies = vec![BrowserCookie {
            name: "sid".into(),
            value: "v".into(),
            domain: "lain.com".into(),
            path: "/".into(),
            secure: false,
            host_only: true,
            expiration_date: None,
        }];
        let res = write_cookies_txt_in(&cookies, "https://example.com/a.zip", &dir);
        assert!(res.is_err(), "harus melaporkan tidak ada cookie cocok");
        assert!(
            jar.exists(),
            "jar yang sudah ada TIDAK boleh dihapus (regresi A2)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
