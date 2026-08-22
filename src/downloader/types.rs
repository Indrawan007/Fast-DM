use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Queued,
    Resolving,
    Downloading,
    Paused,
    Completed,
    Error,
    Cancelled,
}

impl std::fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Resolving => write!(f, "resolving"),
            Self::Downloading => write!(f, "downloading"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Error => write!(f, "error"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_dir: String,
    pub status: DownloadStatus,
    pub total_size: u64,
    pub downloaded: u64,
    pub speed: u64,
    pub eta: u64,
    pub progress: f64,
    pub error_msg: String,
    pub connections: u8,
    pub retry_count: u8,
    pub is_youtube: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub created: i64,
}

impl DownloadInfo {
    pub fn new(
        id: String,
        url: String,
        filename: String,
        save_dir: String,
        headers: HashMap<String, String>,
        quality: Option<String>,
    ) -> Self {
        Self {
            id, url, filename, save_dir,
            status: DownloadStatus::Queued,
            total_size: 0, downloaded: 0,
            speed: 0, eta: 0, progress: 0.0,
            error_msg: String::new(),
            connections: 0, retry_count: 0,
            is_youtube: false,
            headers, quality,
            pid: None,
            created: chrono::Utc::now().timestamp(),
        }
    }

    pub fn total_size_fmt(&self) -> String { format_size(self.total_size) }
    pub fn downloaded_fmt(&self) -> String { format_size(self.downloaded) }
    pub fn speed_fmt(&self) -> String {
        if self.speed == 0 { "0 B/s".into() }
        else { format!("{}/s", format_size(self.speed)) }
    }
    pub fn eta_fmt(&self) -> String { format_eta(self.eta) }
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 { return "0 B".into(); }
    let mut size = bytes as f64;
    let mut i = 0;
    while size >= 1024.0 && i < UNITS.len() - 1 {
        size /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", size, UNITS[i])
}

pub fn format_eta(seconds: u64) -> String {
    if seconds == 0 { return "--".into(); }
    if seconds < 60 { return format!("{}s", seconds); }
    if seconds < 3600 {
        return format!("{}m {}s", seconds / 60, seconds % 60);
    }
    format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress(DownloadInfo),
    Completed(DownloadInfo),
    Error(DownloadInfo),
}
