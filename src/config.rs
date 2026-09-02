use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub download_dir: String,
    pub max_connections: u8,
    pub max_concurrent: u8,
    pub max_overall_speed: String,
    pub retry_count: u8,
    pub retry_wait: u8,
    pub timeout: u16,
    pub disk_cache_size: String,
    pub file_allocation: String,
    pub auto_file_renaming: bool,
    pub verify_tls: bool,
}

impl Default for Config {
    fn default() -> Self {
        // Fallback bertingkat — JANGAN unwrap() langsung: home_dir() bisa
        // None (mis. HOME tidak diset) dan panic akan mematikan app saat start.
        let download_dir = dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
            .unwrap_or_else(|| PathBuf::from("Downloads"))
            .to_string_lossy()
            .to_string();

        Self {
            download_dir,
            max_connections: 16,
            max_concurrent: 3,
            max_overall_speed: "0".into(),
            retry_count: 5,
            retry_wait: 3,
            timeout: 30,
            disk_cache_size: "64M".into(),
            file_allocation: "falloc".into(),
            auto_file_renaming: true,
            verify_tls: true,
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("fast-dm")
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.json")
    }
    /// B7: file cookie per-domain (cookies_<host>.txt). Per-domain supaya dua
    /// download bersamaan dari situs berbeda tidak saling menimpa cookies.
    pub fn cookies_file_for(domain: &str) -> PathBuf {
Self::cookies_file_for_host(&Self::normalize_host(domain))
    }

    /// Cari file cookie untuk host — coba host persis dulu, lalu naik ke
    /// domain induk (sub.example.com → example.com). Extension menyimpan
    /// cookies memakai host HALAMAN, sedangkan file media sering berada di
    /// subdomain CDN yang berbeda; tanpa pencarian induk, login-protected
    /// download dari subdomain kehilangan cookies-nya.
    pub fn find_cookies_file(host: &str) -> Option<PathBuf> {
        let mut h = Self::normalize_host(host);
        while !h.is_empty() {
            let p = Self::cookies_file_for_host(&h);
            if p.exists() {
                return Some(p);
            }
            // Buang label kiri: "a.b.c" → "b.c"; berhenti di "c"
            h = match h.find('.') {
                Some(i) => h[i + 1..].to_string(),
                None => String::new(),
            };
        }
        None
    }

    /// "WWW.Example.COM" → "example.com" (lowercase + strip "www." case-insensitive)
    fn normalize_host(domain: &str) -> String {
        // Lowercase dulu, BARU strip "www." — kalau tidak, trim_start_matches
        // case-sensitive akan skip "WWW." (Bug B1, ditemukan saat menambah
        // unit test — sebelumnya integration test tidak cover uppercase).
        let s = domain.trim().to_ascii_lowercase();
        s.trim_start_matches("www.").to_string()
    }

    fn cookies_file_for_host(host: &str) -> PathBuf {
        let safe: String = host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self::config_dir().join(format!("cookies_{safe}.txt"))
    }

    pub fn load() -> &'static Config {
        CONFIG.get_or_init(|| {
            let path = Self::config_file();
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str(&content) {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            // Jangan diam-diam reset config user — log dan lanjut default
                            tracing::warn!("Config rusak/tidak cocok ({e}), pakai default");
                            Config::default()
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Config tidak bisa dibaca ({e}), pakai default");
                        Config::default()
                    }
                }
            } else {
                let cfg = Config::default();
                let _ = cfg.save();
                cfg
            }
        })
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        // Tulis atomik (tmp + rename) supaya config tidak korup kalau crash
        let path = Self::config_file();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_host (tested via public cookies_file_for) ──

    #[test]
    fn normalize_strips_www_and_lowercases() {
        let p = Config::cookies_file_for("WWW.Example.COM");
        let name = p.file_name().unwrap().to_str().unwrap();
        // Bentuk: "cookies_example.com.txt" (bukan WWW.example.com)
        assert_eq!(name, "cookies_example.com.txt");
    }

    #[test]
    fn normalize_trims_whitespace() {
        let p = Config::cookies_file_for("  example.com  ");
        assert!(p.to_str().unwrap().ends_with("cookies_example.com.txt"));
    }

    // ── cookies_file_for sanitization ──

    #[test]
    fn cookies_file_sanitizes_unsafe_chars() {
        // Karakter non-alfanumerik (selain . -) diganti underscore
        let p = Config::cookies_file_for("evil host/name.txt");
        let name = p.file_name().unwrap().to_str().unwrap();
        // "evil host/name.txt" → "evil_host_name.txt"
        // (chars ' ', '/' → '_', '.txt' tetap)
        assert!(
            name.starts_with("cookies_evil_host_name.txt"),
            "got: {:?}",
            name
        );
        // Tidak boleh ada karakter terlarang
        assert!(!name.contains(' '));
        assert!(!name.contains('/'));
    }

    // ── find_cookies_file: parent-domain lookup ──
    //
    // CATATAN: Config::config_dir() baca `dirs::config_dir()` (XDG_CONFIG_HOME
    // atau ~/.config) yang statis per-proses. Unit test di sini tidak
    // menyentuh filesystem (akan mengotori config user). Test riil ada di
    // integration test `tests/find_cookies.rs` yang override XDG_CONFIG_HOME
    // + pakai std::env::temp_dir() (stdlib-only, tanpa dependency).

    #[test]
    fn find_cookies_file_none_when_not_found() {
        // Domain yang PASTI tidak punya file cookie di config_dir manapun
        // (suffix unik agar tidak bentrok dengan test/installation lain).
        let found = Config::find_cookies_file("nonexistent-domain-zzzz-unique-12345.test");
        assert!(found.is_none(), "domain fiktif harus return None");
    }

    /// Smoke test: walk-up logic seharusnya berhenti di single-label host
    /// (mis. "localhost" tidak punya parent, harus return None tanpa loop).
    #[test]
    fn find_cookies_file_single_label_returns_none() {
        let found = Config::find_cookies_file("localhost");
        // Single-label tidak punya '.' → loop keluar → None
        assert!(found.is_none());
    }

    // ── Config default ──

    #[test]
    fn config_default_has_safe_values() {
        let c = Config::default();
        // Jangan panic di env tanpa HOME
        assert!(!c.download_dir.is_empty());
        assert!(c.max_connections > 0 && c.max_connections <= 32);
        assert!(c.max_concurrent > 0 && c.max_concurrent <= 10);
        assert!(c.timeout > 0);
        assert!(c.verify_tls); // default aman
    }

    #[test]
    fn config_serde_roundtrip() {
        let original = Config::default();
        let json = serde_json::to_string(&original).unwrap();
        let restored: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.download_dir, original.download_dir);
        assert_eq!(restored.max_connections, original.max_connections);
        assert_eq!(restored.verify_tls, original.verify_tls);
    }

    #[test]
    fn config_serde_with_missing_fields_uses_default() {
        // JSON kosong (atau hanya sebagian field) harus fallback ke default
        // via #[serde(default)] di struct Config
        let partial = r#"{"download_dir": "/custom"}"#;
        let c: Config = serde_json::from_str(partial)
            .expect("field opsional harus fallback ke default");
        assert_eq!(c.download_dir, "/custom");
        // Field lain dari default impl
        assert!(c.max_connections > 0);
    }
}

