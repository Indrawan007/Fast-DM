use crate::downloader::DownloadEngine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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

pub async fn start_server(engine: Arc<DownloadEngine>) -> Result<(), Box<dyn std::error::Error>> {
    let uid = nix::unistd::getuid();
    let socket_path = format!("/tmp/fast-dm-{}.sock", uid);

    // Cleanup old socket
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;

    // Set permissions 0600
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

    tracing::info!("IPC listening on {}", socket_path);

    loop {
        let (stream, _) = listener.accept().await?;
        let engine = engine.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            match buf_reader.read_line(&mut line).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }

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
                _ => return IpcResponse {
                    success: false, id: None,
                    error: Some("No URL".into()), message: None,
                },
            };

            let id = engine
                .add_download(&url, msg.filename.as_deref(), None, true)
                .await;

            IpcResponse {
                success: true,
                id: Some(id),
                error: None,
                message: None,
            }
        }

        "ping" => IpcResponse {
            success: true, id: None, error: None,
            message: Some("running".into()),
        },

        "pause" => {
            if let Some(id) = msg.id {
                engine.pause_download(&id).await;
            }
            IpcResponse { success: true, id: None, error: None, message: None }
        }

        "resume" => {
            if let Some(id) = msg.id {
                engine.resume_download(&id).await;
            }
            IpcResponse { success: true, id: None, error: None, message: None }
        }

        "cancel" => {
            if let Some(id) = msg.id {
                engine.cancel_download(&id).await;
            }
            IpcResponse { success: true, id: None, error: None, message: None }
        }

        "list" => {
            let downloads = engine.get_all_downloads().await;
            let list: Vec<serde_json::Value> = downloads.iter().map(|d| {
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
            }).collect();

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
                        success: true, id: None, error: None,
                        message: Some(format!("Registered {} manifests", count)),
                    },
                    Err(e) => IpcResponse {
                        success: false, id: None,
                        error: Some(e.to_string()), message: None,
                    },
                }
            } else {
                IpcResponse {
                    success: false, id: None,
                    error: Some("No extension_id".into()), message: None,
                }
            }
        }

        _ => IpcResponse {
            success: false, id: None,
            error: Some(format!("Unknown action: {}", msg.action)),
            message: None,
        },
    }
}
