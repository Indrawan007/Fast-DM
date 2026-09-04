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
fn peer_uid_ok(stream: &tokio::net::UnixStream) -> bool {
    match nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials) {
        // Ucred::uid() mengembalikan uid_t (u32), bukan Uid — bandingkan raw.
        Ok(cred) => cred.uid() == nix::unistd::getuid().as_raw(),
        Err(_) => false,
    }
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

    loop {
        let (stream, _) = listener.accept().await?;

        if !peer_uid_ok(&stream) {
            tracing::warn!("IPC: koneksi ditolak (peer UID != uid kita) — ditutup");
            drop(stream); // close
            continue;
        }

        let engine = engine.clone();

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

            // Tulis cookies.txt SEBELUM download start (yt-dlp membacanya saat spawn)
            if let (Some(c), Some(d)) = (msg.cookies.as_deref(), msg.domain.as_deref()) {
                if let Err(e) = write_cookies_txt(c, d) {
                    tracing::warn!("set cookies: {}", e);
                }
            }

            let id = engine
                .add_download(
                    &url,
                    msg.filename.as_deref(),
                    None,
                    true,
                    msg.headers,
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
}
