pub mod setup;

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

#[derive(Deserialize)]
struct NativeMessage {
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
struct NativeResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn run() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();

    // Baca satu message, respond, exit
    // Chrome akan spawn ulang native host untuk message baru
    loop {
        // Read 4-byte length
        let mut len_buf = [0u8; 4];
        match stdin.read_exact(&mut len_buf) {
            Ok(_) => {},
            Err(_) => break,  // EOF atau error → exit
        }

        let msg_len = u32::from_le_bytes(len_buf) as usize;
        if msg_len == 0 || msg_len > 1024 * 1024 {
            break;
        }

        // Read message
        let mut msg_buf = vec![0u8; msg_len];
        if stdin.read_exact(&mut msg_buf).is_err() {
            break;
        }

        // Parse & handle
        let response = match serde_json::from_slice::<NativeMessage>(&msg_buf) {
            Ok(msg) => handle_native_message(msg),
            Err(e) => NativeResponse {
                success: false,
                message: None,
                error: Some(e.to_string()),
            },
        };

        // Send response
        let resp_bytes = match serde_json::to_vec(&response) {
            Ok(b) => b,
            Err(_) => break,
        };

        let len = (resp_bytes.len() as u32).to_le_bytes();
        if stdout.write_all(&len).is_err() { break; }
        if stdout.write_all(&resp_bytes).is_err() { break; }
        if stdout.flush().is_err() { break; }
    }
}

fn handle_native_message(msg: NativeMessage) -> NativeResponse {
    match msg.action.as_str() {
        "register" => {
            if let Some(ext_id) = msg.extension_id {
                match setup::register_extension_id(&ext_id) {
                    Ok(n) => NativeResponse {
                        success: true,
                        message: Some(format!("Registered {} manifests", n)),
                        error: None,
                    },
                    Err(e) => NativeResponse {
                        success: false, message: None,
                        error: Some(e.to_string()),
                    },
                }
            } else {
                NativeResponse {
                    success: false, message: None,
                    error: Some("No extension_id".into()),
                }
            }
        }

        _ => {            // Forward to GUI via Unix socket — path privat yang sama dengan
            // yang di-bind GUI (v2.3.0 K1: tidak lagi /tmp publik).
            let socket_path = crate::config::Config::ipc_socket_path();


            match forward_to_gui(&socket_path, &msg) {
                Ok(resp) => resp,
                Err(e) => {
                    // Launch GUI dengan setsid agar TIDAK jadi child dari browser.
                    // Jangan paksa GDK_BACKEND=x11 — pada sesi Wayland-only GUI tidak bisa start.
                    use std::os::unix::process::CommandExt;
                    let gui_path = resolve_gui_path();
                    // B4: stdio wajib diputus (null) — GUI TIDAK boleh mewarisi
                    // stdout/stdin native host (pipe length-prefixed milik
                    // browser); output GUI ke stdout akan merusak protokol
                    // native messaging.

                    let _ = std::process::Command::new(&gui_path)
                        .process_group(0)  // New process group
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();

                    // BUG FIX: cold start GUI (inisialisasi GTK) bisa > 2 dtk
                    // di mesin lambat — sleep tetap 2 dtk sering kalah cepat
                    // sehingga request pertama user gagal. Poll sampai socket
                    // merespons (maks ~15 dtk) lalu forward sekali lagi.
                    let mut ready = false;
                    for _ in 0..150 {
                        // &socket_path (bukan move): loop butuh path berulang
                        match std::os::unix::net::UnixStream::connect(&socket_path) {
                            Ok(_) => {
                                ready = true;
                                break;
                            }
                            Err(_) => {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                    }

                    if ready {
                        forward_to_gui(&socket_path, &msg).unwrap_or(NativeResponse {
                            success: false, message: None,
                            error: Some("Cannot reach GUI".into()),
                        })
                    } else {
                        NativeResponse {
                            success: false, message: None,
                            error: Some(format!(
                                "Cannot reach GUI: Fast DM tidak bisa dijalankan ({})",
                                e
                            )),
                        }
                    }
                }
            }
        }
    }
}

/// Resolve path to GUI binary (not the native host wrapper)
fn resolve_gui_path() -> String {
    // Installed path
    let installed = std::path::Path::new("/opt/fast-dm/fast-dm");
    if installed.exists() {
        return installed.to_string_lossy().to_string();
    }
    // Development: same directory as current exe, parent is `fast-dm` (not `fast-dm-native`)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("fast-dm");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "/opt/fast-dm/fast-dm".to_string()
}

fn forward_to_gui(socket_path: &std::path::Path, msg: &NativeMessage) -> Result<NativeResponse, String> {
    use std::io::BufRead;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| e.to_string())?;

    // Timeout supaya native host tidak hang selamanya jika GUI macet
    let timeout = Some(std::time::Duration::from_secs(5));
    stream.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    stream.set_write_timeout(timeout).map_err(|e| e.to_string())?;

    let json = serde_json::to_string(&serde_json::json!({
        "action": msg.action,
        "url": msg.url,
        "filename": msg.filename,
        "quality": msg.quality,
        "id": msg.id,
        "extension_id": msg.extension_id,
        "headers": msg.headers,
        "cookies": msg.cookies,
        "domain": msg.domain,
    })).unwrap();

    stream.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    // Read response from IPC server
    let mut reader = std::io::BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;

    if line.is_empty() {
        return Ok(NativeResponse { success: true, message: None, error: None });
    }

    let resp: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    let success = resp.get("success").and_then(|v| v.as_bool()).unwrap_or(true);

    Ok(NativeResponse {
        success,
        message: resp.get("message").and_then(|v| v.as_str().map(|s| s.to_string())),
        error: resp.get("error").and_then(|v| v.as_str().map(|s| s.to_string())),
    })
}
