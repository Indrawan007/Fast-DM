use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub fn load() -> &'static Config {
        CONFIG.get_or_init(|| {
            let path = Self::config_file();
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        serde_json::from_str(&content).unwrap_or_default()
                    }
                    Err(_) => Config::default(),
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
        fs::write(Self::config_file(), json)?;
        Ok(())
    }
}
