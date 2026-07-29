use glob::glob;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const HOST_NAME: &str = "com.fastdm.native";
const NATIVE_PATH: &str = "/opt/fast-dm/fast-dm-native";

pub fn check_and_setup() -> Result<usize, Box<dyn std::error::Error>> {
    let native_path = if Path::new(NATIVE_PATH).exists() {
        NATIVE_PATH.to_string()
    } else {
        let exe = std::env::current_exe()?;
        exe.parent()
            .unwrap_or(Path::new("/opt/fast-dm"))
            .join("fast-dm-native")
            .to_string_lossy()
            .to_string()
    };

    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": native_path,
        "type": "stdio",
        "allowed_origins": ["chrome-extension://*/*"]
    });

    let dirs = get_all_nmh_dirs();
    let mut created = 0;

    for dir in &dirs {
        let manifest = dir.join(format!("{}.json", HOST_NAME));
        let need_update = if manifest.exists() {
            match fs::read_to_string(&manifest) {
                Ok(content) => {
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(existing) => existing["path"] != native_path,
                        Err(_) => true,
                    }
                }
                Err(_) => true,
            }
        } else {
            true
        };

        if need_update {
            if let Ok(()) = fs::create_dir_all(dir) {
                let json_str = serde_json::to_string_pretty(&host_json)?;
                if fs::write(&manifest, &json_str).is_ok() {
                    created += 1;
                }
            }
        }
    }

    if created > 0 {
        tracing::info!("Setup: {} browser manifest(s) updated", created);
    }

    Ok(created)
}

pub fn register_extension_id(ext_id: &str) -> Result<usize, Box<dyn std::error::Error>> {
    if ext_id.len() < 20 || !ext_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid extension ID".into());
    }

    let native_path = if Path::new(NATIVE_PATH).exists() {
        NATIVE_PATH
    } else {
        "/opt/fast-dm/fast-dm-native"
    };

    let origin = format!("chrome-extension://{}/", ext_id);
    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": native_path,
        "type": "stdio",
        "allowed_origins": [origin]
    });

    let dirs = get_all_nmh_dirs();
    let mut updated = 0;

    for dir in &dirs {
        let manifest = dir.join(format!("{}.json", HOST_NAME));
        if let Ok(()) = fs::create_dir_all(dir) {
            let json_str = serde_json::to_string_pretty(&host_json)?;
            if fs::write(&manifest, &json_str).is_ok() {
                updated += 1;
            }
        }
    }

    tracing::info!("Extension ID registered: {} ({} manifests)", ext_id, updated);
    Ok(updated)
}

fn get_all_nmh_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return dirs,
    };

    let config = home.join(".config");

    // Standard browsers
    let browsers = [
        "google-chrome", "chromium", "thorium",
        "BraveSoftware/Brave-Browser", "vivaldi",
        "opera", "com.operasoftware.Opera",
        "microsoft-edge", "ungoogled-chromium",
        "yandex-browser", "sidekick", "helium",
    ];

    for browser in &browsers {
        dirs.push(config.join(browser).join("NativeMessagingHosts"));
    }

    // Ice / Helium profiles
    let ice_bases = [
        home.join(".local/share/ice/profiles"),
        home.join(".local/share/helium/profiles"),
    ];

    for base in &ice_bases {
        if base.is_dir() {
            if let Ok(entries) = fs::read_dir(base) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        dirs.push(entry.path().join("NativeMessagingHosts"));
                    }
                }
            }
        }
    }

    // Scan existing NMH dirs
    let patterns = [
        format!("{}/**/NativeMessagingHosts", config.display()),
    ];

    for pattern in &patterns {
        if let Ok(paths) = glob(pattern) {
            for path in paths.flatten() {
                if path.is_dir() && !dirs.contains(&path) {
                    dirs.push(path);
                }
            }
        }
    }

    dirs
}
