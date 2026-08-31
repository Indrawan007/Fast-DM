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
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join("Downloads"))
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
        let host = domain.trim().trim_start_matches("www.").to_ascii_lowercase();
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
