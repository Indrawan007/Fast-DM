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
    /// v2.3.0 (M10): info status non-error (mis. "Merging video + audio…").
    /// Dulu info seperti ini ditulis ke error_msg → UI menampilkan baris
    /// merah untuk proses yang sebenarnya normal.
    #[serde(default)]
    pub status_detail: String,
    pub connections: u8,
    pub retry_count: u8,
    pub is_youtube: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    /// epoch MILLISECONDS (v2.3.0, L4 — dulu detik sehingga antrian FIFO tidak
    /// deterministik bila dua download dibuat pada detik yang sama)

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
            status_detail: String::new(),
            connections: 0, retry_count: 0,
            is_youtube: false,
            headers, quality,
            pid: None,
            created: chrono::Utc::now().timestamp_millis(),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── DownloadStatus Display ──

    #[test]
    fn status_display_all_variants() {
        // Setiap varian harus punya Display yang non-kosong & stabil
        // (dipakai untuk serialisasi IPC dan badge label)
        let cases = [
            (DownloadStatus::Queued, "queued"),
            (DownloadStatus::Resolving, "resolving"),
            (DownloadStatus::Downloading, "downloading"),
            (DownloadStatus::Paused, "paused"),
            (DownloadStatus::Completed, "completed"),
            (DownloadStatus::Error, "error"),
            (DownloadStatus::Cancelled, "cancelled"),
        ];
        for (status, expected) in cases {
            assert_eq!(status.to_string(), expected, "status {:?} display salah", status);
        }
    }

    #[test]
    fn status_roundtrip_via_serde() {
        // IPC serialize/deserialize harus roundtrip identik
        for status in [
            DownloadStatus::Queued,
            DownloadStatus::Resolving,
            DownloadStatus::Downloading,
            DownloadStatus::Paused,
            DownloadStatus::Completed,
            DownloadStatus::Error,
            DownloadStatus::Cancelled,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: DownloadStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status, "roundtrip serde gagal untuk {:?}", status);
        }
    }

    // ── format_size ──

    #[test]
    fn format_size_zero() {
        assert_eq!(format_size(0), "0 B");
    }

    #[test]
    fn format_size_bytes() {
        // format_size pakai "{:.1} {unit}" untuk konsistensi visual.
        // Byte di bawah 1KB ditampilkan sebagai "512.0 B" (bukan "512 B")
        // — dipilih seragam dengan KB/MB/GB yang memang butuh desimal.
        assert_eq!(format_size(512), "512.0 B");
        assert_eq!(format_size(1023), "1023.0 B");
    }

    #[test]
    fn format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn format_size_mb_gb_tb() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(1024_u64.pow(4)), "1.0 TB");
    }

    #[test]
    fn format_size_overflow_safety() {
        // u64::MAX tidak boleh panic
        let _ = format_size(u64::MAX);
    }

    // ── format_eta ──

    #[test]
    fn format_eta_zero() {
        assert_eq!(format_eta(0), "--");
    }

    #[test]
    fn format_eta_seconds_only() {
        assert_eq!(format_eta(45), "45s");
    }

    #[test]
    fn format_eta_minutes_seconds() {
        assert_eq!(format_eta(125), "2m 5s");
        assert_eq!(format_eta(59), "59s"); // < 60 tetap detik saja
    }

    #[test]
    fn format_eta_hours_minutes() {
        assert_eq!(format_eta(3725), "1h 2m"); // 1*3600 + 2*60 + 5
        assert_eq!(format_eta(3600), "1h 0m");
    }

    // ── DownloadInfo::new default state ──

    #[test]
    fn download_info_new_defaults() {
        let info = DownloadInfo::new(
            "dl_test".into(),
            "https://example.com/file.zip".into(),
            "file.zip".into(),
            "/tmp".into(),
            Default::default(),
            None,
        );
        assert_eq!(info.id, "dl_test");
        assert_eq!(info.status, DownloadStatus::Queued);
        assert_eq!(info.total_size, 0);
        assert_eq!(info.downloaded, 0);
        assert_eq!(info.progress, 0.0);
        assert!(info.error_msg.is_empty());
        assert!(info.pid.is_none());
        assert!(info.status_detail.is_empty());
        assert!(info.pid.is_none());
        // created sekarang milliseconds — harus jauh > 0
        assert!(info.created > 1_700_000_000_000);
    }

    #[test]
    fn download_info_formatters() {
        let mut info = DownloadInfo::new(
            "x".into(), "u".into(), "f".into(), "/tmp".into(),
            Default::default(), None,
        );
        info.total_size = 2048;
        info.downloaded = 1024;
        info.speed = 512;
        info.eta = 30;
        assert_eq!(info.total_size_fmt(), "2.0 KB");
        assert_eq!(info.downloaded_fmt(), "1.0 KB");
        // speed_fmt pakai format_size → "512.0 B/s" (bukan "512 B/s")
        // untuk konsistensi dengan format_size_bytes test di atas.
        assert_eq!(info.speed_fmt(), "512.0 B/s");
        assert_eq!(info.eta_fmt(), "30s");
    }

    #[test]
    fn download_info_speed_zero() {
        let info = DownloadInfo::new(
            "x".into(), "u".into(), "f".into(), "/tmp".into(),
            Default::default(), None,
        );
        // speed=0 → "0 B/s" (special case di speed_fmt, BUKAN format_size
        // yang return "0.0 B" — lihat types.rs)
        assert_eq!(info.speed_fmt(), "0 B/s");
    }

    // ── DownloadInfo serde backward-compat ──

    #[test]
    fn download_info_serde_with_missing_fields() {
        // Field baru dengan #[serde(default)] harus bisa deserialize
        // payload lama (mis. session.json dari versi sebelumnya).
        let old_json = r#"{
            "id": "dl_abc",
            "url": "https://x.com/y.zip",
            "filename": "y.zip",
            "save_dir": "/tmp",
            "status": "completed",
            "total_size": 100,
            "downloaded": 100,
            "speed": 0,
            "eta": 0,
            "progress": 100.0,
            "error_msg": "",
            "connections": 0,
            "retry_count": 0,
            "is_youtube": false
        }"#;
        let info: DownloadInfo = serde_json::from_str(old_json)
            .expect("field optional (headers/quality/pid/created) harus backward-compat");
        assert_eq!(info.id, "dl_abc");
        assert_eq!(info.status, DownloadStatus::Completed);
        assert!(info.headers.is_empty());
        assert!(info.quality.is_none());
        assert!(info.pid.is_none());
        assert!(info.status_detail.is_empty()); // field baru → default "" (M10)
    }
}
