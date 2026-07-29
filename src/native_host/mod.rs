pub mod setup;

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

#[derive(Deserialize)]
struct NativeMessage {
    action: String,
    url: Option<String>,
    filename: Option<String>,
    quality: Option<String>,
    extension_id: Option<String>,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
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

    loop {
        // Read 4-byte length
        let mut len_buf = [0u8; 4];
        if stdin.read_exact(&mut len_buf).is_err() {
            break;
        }
        let msg_len = u32::from_ne_bytes(len_buf) as usize;
        if msg_len > 1024 * 1024 {
            continue;
        }

        // Read message
        let mut msg_buf = vec![0u8; msg_len];
        if stdin.read_exact(&mut msg_buf).is_err() {
            break;
        }

        // Parse
        let response = match serde_json::from_slice::<NativeMessage>(&msg_buf) {
            Ok(msg) => handle_native_message(msg),
            Err(e) => NativeResponse {
                success: false,
                message: None,
                error: Some(e.to_string()),
            },
        };

        // Send response
        let resp_bytes = serde_json::to_vec(&response).unwrap_or_default();
        let len = (resp_bytes.len() as u32).to_ne_bytes();
        let _ = stdout.write_all(&len);
        let _ = stdout.write_all(&resp_bytes);
        let _ = stdout.flush();
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

        _ => {
            // Forward to GUI via Unix socket
            let uid = nix::unistd::getuid();
            let socket_path = format!("/tmp/fast-dm-{}.sock", uid);

            match forward_to_gui(&socket_path, &msg) {
                Ok(resp) => resp,
                Err(e) => {
                    // Try launching GUI
                    let _ = std::process::Command::new("/opt/fast-dm/fast-dm")
                        .spawn();
                    std::thread::sleep(std::time::Duration::from_secs(2));

                    forward_to_gui(&socket_path, &msg).unwrap_or(NativeResponse {
                        success: false, message: None,
                        error: Some(format!("Cannot reach GUI: {}", e)),
                    })
                }
            }
        }
    }
}

fn forward_to_gui(socket_path: &str, msg: &NativeMessage) -> Result<NativeResponse, String> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| e.to_string())?;

    let json = serde_json::to_string(&serde_json::json!({
        "action": msg.action,
        "url": msg.url,
        "filename": msg.filename,
        "quality": msg.quality,
        "headers": msg.headers,
    })).unwrap();

    stream.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    Ok(NativeResponse {
        success: true, message: None, error: None,
    })
}
