use glob::glob;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const HOST_NAME: &str = "com.fastdm.native";
const NATIVE_PATH: &str = "/opt/fast-dm/fast-dm-native";

pub fn check_and_setup() -> Result<usize, Box<dyn std::error::Error>> {
    let native_path = resolve_native_path();

    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": native_path,
        "type": "stdio",
        "allowed_origins": ["chrome-extension://*/*"]
    });

    let json_str = serde_json::to_string_pretty(&host_json)?;
    let dirs = get_all_nmh_dirs();
    let mut created = 0;

    tracing::debug!("Checking {} browser locations", dirs.len());

    for dir in &dirs {
        let manifest = dir.join(format!("{}.json", HOST_NAME));

        // Cek apakah perlu update
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
            if fs::create_dir_all(dir).is_ok() {
                if fs::write(&manifest, &json_str).is_ok() {
                    created += 1;
                    tracing::debug!("Setup: {}", manifest.display());
                }
            }
        }
    }

    if created > 0 {
        tracing::info!("Setup: {} browser manifest(s) created/updated", created);
    }

    Ok(created)
}

pub fn register_extension_id(ext_id: &str) -> Result<usize, Box<dyn std::error::Error>> {
    // Validasi ID
    if ext_id.len() < 20 || !ext_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid extension ID".into());
    }

    let native_path = resolve_native_path();
    let origin = format!("chrome-extension://{}/", ext_id);

    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": native_path,
        "type": "stdio",
        "allowed_origins": [origin]
    });

    let json_str = serde_json::to_string_pretty(&host_json)?;
    let dirs = get_all_nmh_dirs();
    let mut updated = 0;

    for dir in &dirs {
        let manifest = dir.join(format!("{}.json", HOST_NAME));
        if fs::create_dir_all(dir).is_ok() {
            if fs::write(&manifest, &json_str).is_ok() {
                updated += 1;
            }
        }
    }

    tracing::info!("Extension ID registered: {} ({} manifests)", ext_id, updated);
    Ok(updated)
}

fn resolve_native_path() -> String {
    // Prioritas: /opt/fast-dm/fast-dm-native (dari .deb install)
    if Path::new(NATIVE_PATH).exists() {
        return NATIVE_PATH.to_string();
    }

    // Fallback: cari di lokasi executable saat ini
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Cek native-host wrapper di folder yang sama
            let candidate = parent.join("fast-dm-native");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
            // Cek di root project (development)
            let project_root = parent.parent().and_then(|p| p.parent());
            if let Some(root) = project_root {
                let candidate = root.join("fast-dm-native");
                if candidate.exists() {
                    return candidate.to_string_lossy().to_string();
                }
            }
        }
    }

    // Default ke /opt bahkan jika tidak ada (postinst akan buat)
    NATIVE_PATH.to_string()
}

fn get_all_nmh_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return dirs,
    };

    let config = home.join(".config");
    let local_share = home.join(".local").join("share");

    // ── 1. Standard Chromium-based browsers ──
    let browsers = [
        "google-chrome",
        "chromium",
        "thorium",
        "BraveSoftware/Brave-Browser",
        "vivaldi",
        "opera",
        "com.operasoftware.Opera",
        "microsoft-edge",
        "ungoogled-chromium",
        "yandex-browser",
        "sidekick",
        "helium",
        "net.imput.helium",  // Helium Flatpak
    ];

    for browser in &browsers {
        dirs.push(config.join(browser).join("NativeMessagingHosts"));
    }

    // ── 2. Ice / Helium / WebApp profiles ──
    let profile_bases = [
        local_share.join("ice/profiles"),
        local_share.join("helium/profiles"),
    ];

    for base in &profile_bases {
        if base.is_dir() {
            if let Ok(entries) = fs::read_dir(base) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Profile-level
                        dirs.push(path.join("NativeMessagingHosts"));
                        // Default profile subdirectory (kadang di sini)
                        dirs.push(path.join("Default").join("NativeMessagingHosts"));
                    }
                }
            }
        }
    }

    // ── 3. Scan folder yang punya subfolder "Default" (Chromium profile) ──
    let scan_pattern = format!("{}/*/Default", config.display());
    if let Ok(paths) = glob(&scan_pattern) {
        for path in paths.flatten() {
            if path.is_dir() {
                if let Some(parent) = path.parent() {
                    let nmh = parent.join("NativeMessagingHosts");
                    if !dirs.contains(&nmh) {
                        dirs.push(nmh);
                    }
                }
            }
        }
    }

    // ── 4. Flatpak sandbox — biarkan, tidak perlu setup ──

    dirs
}
