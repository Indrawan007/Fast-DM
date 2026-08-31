use crate::config::Config;
use glob::glob;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const HOST_NAME: &str = "com.fastdm.native";
const NATIVE_PATH: &str = "/opt/fast-dm/fast-dm-native";
const EXT_ID: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/EXT_ID"));
const REGISTRY_FILE: &str = "extension_ids.json";

/// Baca daftar extension ID yang pernah di-register (persisten di config dir).
/// Dipakai supaya manifest TIDAK ditimpa ke EXT_ID lagi saat aplikasi restart
/// (bug: extension unpacked putus native messaging setelah restart).
fn load_registered_ids() -> Vec<String> {
    let path = Config::config_dir().join(REGISTRY_FILE);
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<Vec<String>>(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_registered_ids(ids: &[String]) {
    let path = Config::config_dir().join(REGISTRY_FILE);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(ids) {
        let _ = fs::write(&path, json);
    }
}

/// Semua origin yang boleh memanggil native host: ID packed + ID yang pernah
/// di-register (unpacked/dev extension).
fn make_origins(registered: &[String]) -> Vec<String> {
    let mut origins = vec![format!("chrome-extension://{}/", EXT_ID.trim())];
    for id in registered {
        let origin = format!("chrome-extension://{}/", id);
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

/// Tulis manifest ke semua lokasi browser, kembalikan jumlah yang ditulis.
fn write_manifests(json_str: &str) -> usize {
    let mut written = 0;
    for dir in get_all_nmh_dirs() {
        let manifest = dir.join(format!("{}.json", HOST_NAME));
        if fs::create_dir_all(dir).is_ok() {
            if fs::write(&manifest, json_str).is_ok() {
                written += 1;
                tracing::debug!("Manifest: {}", manifest.display());
            }
        }
    }
    written
}

pub fn check_and_setup() -> Result<usize, Box<dyn std::error::Error>> {
    let registered = load_registered_ids();

    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": resolve_native_path(),
        "type": "stdio",
        "allowed_origins": make_origins(&registered)
    });

    let json_str = serde_json::to_string_pretty(&host_json)?;
    let dirs = get_all_nmh_dirs();
    let mut created = 0;

    tracing::debug!("Checking {} browser locations", dirs.len());

    for dir in &dirs {
        let manifest = dir.join(format!("{}.json", HOST_NAME));

        // Cek apakah perlu update (bandingkan konten penuh, bukan hanya path).
        // Karena origin register ikut disertakan, manifest tidak lagi ditimpa
        // secara tidak sengaja oleh check_and_setup.
        let need_update = if manifest.exists() {
            match fs::read_to_string(&manifest) {
                Ok(content) => content.trim() != json_str.trim(),
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

    // Simpan ID secara persisten + gabung dengan yang sudah ada
    let mut registered = load_registered_ids();
    if !registered.iter().any(|id| id == ext_id) {
        registered.push(ext_id.to_string());
        save_registered_ids(&registered);
    }

    let host_json = json!({
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": resolve_native_path(),
        "type": "stdio",
        "allowed_origins": make_origins(&registered)
    });

    let json_str = serde_json::to_string_pretty(&host_json)?;
    let updated = write_manifests(&json_str);

    tracing::info!("Extension ID registered: {} ({} manifests)", ext_id, updated);
    Ok(updated)
}

pub fn resolve_native_path() -> String {
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
            // B16: Development (cargo run/build) — exe ada di target/<profile>/.
            // Buat wrapper kecil di situ (manifest NMH tidak bisa membawa
            // argumen --native), sehingga native messaging bisa diuji tanpa
            // install .deb.
            let in_target = parent
                .file_name()
                .map_or(false, |p| p == "debug" || p == "release")
                && parent
                    .parent()
                    .map_or(false, |p| p.file_name() == Some(std::ffi::OsStr::new("target")));
            if in_target {
                let _ = fs::write(
                    &candidate,
                    format!("#!/bin/sh\nexec \"{}\" --native \"$@\"\n", exe.display()),
                );
                let _ = fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755));
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
