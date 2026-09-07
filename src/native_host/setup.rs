use crate::config::Config;
use glob::glob;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt; // from_mode()
use std::path::{Path, PathBuf};

const HOST_NAME: &str = "com.fastdm.native";
const NATIVE_PATH: &str = "/opt/fast-dm/fast-dm-native";
const EXT_ID: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/EXT_ID"));
const REGISTRY_FILE: &str = "extension_ids.json";

/// Panjang ID extension Chrome (dipin lewat `key` manifest maupun unpacked).
const EXT_ID_LEN: usize = 32;

/// v2.9.4 (C2): batas entri `extension_ids.json`. Setiap ID terdaftar menjadi
/// satu origin di `allowed_origins` manifest NMH, jadi tanpa batas proses lokal
/// mana pun dengan UID sama bisa memperbesar daftar origin yang diizinkan
/// memanggil native host — dan manifest-nya membengkak tanpa batas.
pub(crate) const MAX_REGISTERED_IDS: usize = 8;

/// v2.9.4 (C2): ID extension Chrome SELALU 32 karakter dari himpunan `a`–`p`
/// (hash kunci publik yang di-encode dengan digit dibatasi a-p).
///
/// Validasi sebelumnya hanya "≥20 karakter alfanumerik apa pun", yang
/// membiarkan string arbitrer masuk `allowed_origins`. Bentuk ketat ini juga
/// menyelaraskan sisi Rust dengan `setup-browser.sh`, yang sudah lama memakai
/// `^[a-p]{32}$` — keduanya HARUS setuju, sebab keduanya menulis manifest yang
/// sama (lihat komentar di kepala script itu).
pub(crate) fn is_valid_extension_id(id: &str) -> bool {
    let id = id.trim();
    id.len() == EXT_ID_LEN && id.chars().all(|c| matches!(c, 'a'..='p'))
}

/// Masukkan ID ke registry: dedup (ID yang sudah ada digeser ke paling baru,
/// pola LRU) lalu evict entri TERLAMA bila melewati `MAX_REGISTERED_IDS`.
/// Urutan vektor = kronologis pendaftaran, paling lama di depan.
///
/// v2.9.4 (C2): mengembalikan apakah ID ini BARU bagi registry. ID yang sudah
/// terdaftar hanya di-refresh posisi LRU-nya — aksesnya tidak bertambah, jadi
/// pemanggil tidak boleh mengumumkan-nya sebagai origin baru.
pub(crate) fn push_registered_id(ids: &mut Vec<String>, id: &str) -> bool {
    let existing = ids.iter().position(|known| known.as_str() == id);
    if let Some(pos) = existing {
        ids.remove(pos);
    }
    ids.push(id.to_string());
    while ids.len() > MAX_REGISTERED_IDS {
        let dropped = ids.remove(0);
        tracing::warn!(
            "Registry extension ID penuh (maks {}) — {} dikeluarkan dari allowed_origins",
            MAX_REGISTERED_IDS,
            dropped
        );
    }
    existing.is_none()
}

/// Baca daftar extension ID yang pernah di-register (persisten di config dir).
/// Dipakai supaya manifest TIDAK ditimpa ke EXT_ID lagi saat aplikasi restart
/// (bug: extension unpacked putus native messaging setelah restart).
///
/// v2.9.4 (C2): entri yang tidak lolos `is_valid_extension_id` dibuang saat
/// baca — file ini bisa berisi ID warisan validasi lama yang longgar, dan entri
/// seperti itu tidak boleh ikut masuk `allowed_origins`.
fn load_registered_ids() -> Vec<String> {
    let path = Config::config_dir().join(REGISTRY_FILE);
    let ids: Vec<String> = match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    // partition mempertahankan urutan relatif di tiap kelompok.
    let (valid, dropped): (Vec<String>, Vec<String>) =
        ids.into_iter().partition(|id| is_valid_extension_id(id));
    if !dropped.is_empty() {
        tracing::warn!(
            "{} entri extension_ids.json tidak valid — dibuang (ID Chrome = {} karakter a-p)",
            dropped.len(),
            EXT_ID_LEN
        );
    }
    valid
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

/// v2.3.0 (M8): tulis manifest HANYA untuk browser yang profilnya benar-benar
/// ada. NativeMessagingHosts selalu = <profil browser>/NativeMessagingHosts,
/// jadi cukup cek parent-nya. Dulu `create_dir_all` buta → ±13 folder sampah
/// di ~/.config walau user cuma punya 1 browser.
fn browser_profile_exists(nmh_dir: &Path) -> bool {
    nmh_dir.parent().is_some_and(|p| p.is_dir())
}

/// Tulis manifest ke semua lokasi browser, kembalikan jumlah yang ditulis.
fn write_manifests(json_str: &str) -> usize {
    let mut written = 0;
    for dir in get_all_nmh_dirs() {
        if !browser_profile_exists(&dir) {
            continue;
        }
        let manifest = dir.join(format!("{}.json", HOST_NAME));
        if fs::create_dir_all(dir).is_ok() && fs::write(&manifest, json_str).is_ok() {
            written += 1;
            tracing::debug!("Manifest: {}", manifest.display());
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

        // M8: lewati browser yang tidak ter-install (profil tidak ada)
        if !browser_profile_exists(dir) {
            continue;
        }

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

        if need_update && fs::create_dir_all(dir).is_ok() && fs::write(&manifest, &json_str).is_ok()
        {
            created += 1;
            tracing::debug!("Setup: {}", manifest.display());
        }
    }

    if created > 0 {
        tracing::info!("Setup: {} browser manifest(s) created/updated", created);
    }

    Ok(created)
}

/// v2.9.4 (C2): apakah ID ini memberi origin BARU di `allowed_origins`?
///
/// ID bawaan dikecualikan. `make_origins` selalu memasang `EXT_ID` di posisi
/// pertama apa pun isi registry, jadi register atas ID itu tidak memberi akses
/// baru kepada siapa pun — mengumumkan-nya hanya menjadi noise pada pemasangan
/// normal (user memasang extension, extension mendaftar, tidak ada yang aneh).
/// ID lain = satu origin yang benar-benar bertambah.
fn is_newly_granted_origin(ext_id: &str) -> bool {
    ext_id.trim() != EXT_ID.trim()
}

/// Teks (ringkasan, isi) notifikasi "ekstensi baru terhubung". Dipisah dari
/// `notify_new_extension_id` supaya isinya bisa diuji tanpa men-spawn proses.
fn new_origin_notice(ext_id: &str) -> (String, String) {
    let registry = Config::config_dir().join(REGISTRY_FILE);
    let summary = "Fast DM: ekstensi browser baru diizinkan".to_string();
    let body = format!("ID {ext_id} kini boleh memanggil native host. Cabut: {registry}");
    (summary, body)
}

/// v2.9.4 (C2): beri tahu user bahwa sebuah ID extension baru saja masuk
/// `allowed_origins` manifest Native Messaging.
///
/// Mengapa perlu: aksi `register` bisa datang dari socket IPC (proses lokal
/// mana pun dengan UID sama, setelah cek `SO_PEERCRED`) maupun dari native
/// host, dan ID yang bentuknya valid akan diterima. `is_valid_extension_id`
/// dan cap LRU membatasi seberapa jauh itu bisa pergi, tetapi tidak satu pun
/// bisa membedakan extension sah dari proses lokal yang sedang memasang
/// persistensi untuk dirinya sendiri. Hanya user yang bisa — jadi user harus
/// diberi tahu.
///
/// Memakai `notify-send` (libnotify), bukan notifikasi GTK: `register`
/// diproses di process native host yang tidak punya koneksi display/GTK
/// sama sekali, dan objek GTK tidak `Send` sehingga tidak bisa diserahkan ke
/// task IPC. Best-effort murni — tidak adanya daemon notifikasi bukan alasan
/// menolak register, dan jejaknya tetap tertinggal di log.
///
/// `std::process`, BUKAN `tokio::process`: fungsi ini juga dipanggil dari
/// `native_host::run()`, loop stdio sinkron yang tidak punya runtime tokio
/// sama sekali — `tokio::process` di sana akan panic. Satu-satunya pemanggil
/// yang async adalah `ipc::handle_message`, dan `std::process::Command::spawn`
/// (fork+exec, orde milidetik) sah dipanggil dari dalam task tokio.
fn notify_new_extension_id(ext_id: &str) {
    use std::os::unix::process::CommandExt; // process_group()

    let registry = Config::config_dir().join(REGISTRY_FILE);
    tracing::warn!("Extension ID BARU di allowed_origins: {ext_id} (cabut: {registry})");

    let (summary, body) = new_origin_notice(ext_id);
    let spawned = std::process::Command::new("notify-send")
        .arg("--app-name=Fast Download Manager")
        .arg("--icon=io.github.fastdm.FastDownloadManager")
        .arg("--urgency=normal")
        .arg("--expire-time=10000")
        .arg(&summary)
        .arg(&body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0) // jangan ikut mati bila process group penampung ditutup
        .spawn();

    match spawned {
        // notify-send biasanya selesai dalam milidetik, tetapi bisa menggantung
        // bila daemon notifikasi macet — jadi jangan ditunggu di sini (ini jalur
        // loop pesan native host / accept IPC). Reap di thread terpisah: pemanggil
        // tidak pernah terblokir, dan anak tidak menjadi zombie karena `Child`
        // yang di-drop TIDAK di-reap oleh Rust.
        Ok(child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::debug!("notify-send tidak tersedia ({e}) — notifikasi dilewati"),
    }
}

pub fn register_extension_id(ext_id: &str) -> Result<usize, Box<dyn std::error::Error>> {
    // v2.9.4 (C2): validasi bentuk ID Chrome yang sebenarnya — lihat
    // `is_valid_extension_id`. Pesan error menyertakan nilai yang ditolak agar
    // mudah didiagnosis dari log browser.
    let ext_id = ext_id.trim();
    if !is_valid_extension_id(ext_id) {
        return Err(format!("Invalid extension ID ({EXT_ID_LEN} char a-p): {ext_id:?}").into());
    }

    // Simpan ID secara persisten + gabung dengan yang sudah ada. Selalu ditulis
    // ulang (bukan hanya saat ID baru) supaya registry di disk ikut konvergen
    // setelah entri tak valid dibuang / urutan LRU berubah. Register dipanggil
    // jarang (background.js men-dedup lewat storage), jadi biayanya sepele.
    let mut registered = load_registered_ids();
    let newly_added = push_registered_id(&mut registered, ext_id);
    save_registered_ids(&registered);

    // C2: umumkan origin baru. Dilakukan tepat setelah registry ditulis — bukan
    // setelah manifest — karena registry itulah sumber `allowed_origins` dan
    // ikut dipakai `check_and_setup`, sehingga izinnya bertahan walaupun tulis
    // manifest kali ini gagal. Lihat `is_newly_granted_origin` untuk alasan ID
    // bawaan dilewati.
    if newly_added && is_newly_granted_origin(ext_id) {
        notify_new_extension_id(ext_id);
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

    tracing::info!(
        "Extension ID registered: {} ({} manifests)",
        ext_id,
        updated
    );
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
                .is_some_and(|p| p == "debug" || p == "release")
                && parent
                    .parent()
                    .is_some_and(|p| p.file_name() == Some(std::ffi::OsStr::new("target")));
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
        "net.imput.helium", // Helium Flatpak
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ID sintetis berbentuk sah (32 karakter dari satu huruf a-p).
    fn fake_id(fill: char) -> String {
        std::iter::repeat(fill).take(EXT_ID_LEN).collect()
    }

    fn ids(n: usize, prefix: &str) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i}")).collect()
    }

    // ── v2.9.4 (C2): is_valid_extension_id ──

    #[test]
    fn packed_ext_id_is_valid_shape() {
        // EXT_ID di-pin lewat `key` manifest dan juga dibaca setup-browser.sh.
        // Bila suatu hari diganti dengan bentuk yang tidak valid, native
        // messaging mati total tanpa pesan — test ini penjaganya.
        assert!(
            is_valid_extension_id(EXT_ID.trim()),
            "EXT_ID harus {} karakter a-p, didapat {:?}",
            EXT_ID_LEN,
            EXT_ID.trim()
        );
    }

    #[test]
    fn accepts_chrome_shaped_ids() {
        assert!(is_valid_extension_id("gocipbdlikcnfjpkdcmgoneapnbdodbh"));
        assert!(is_valid_extension_id(&fake_id('a')));
        assert!(is_valid_extension_id(&fake_id('p')));
        // Whitespace di ujung ditoleransi (nilai bisa datang dari file/log).
        let padded = format!("  {}  ", fake_id('b'));
        assert!(is_valid_extension_id(&padded));
    }

    #[test]
    fn rejects_ids_outside_alphabet() {
        // q-z, digit, dan uppercase bukan bagian alphabet ID Chrome.
        assert!(!is_valid_extension_id(&fake_id('q')));
        assert!(!is_valid_extension_id(&fake_id('z')));
        assert!(!is_valid_extension_id(&fake_id('A')));
        assert!(!is_valid_extension_id("12345678901234567890123456789012"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!is_valid_extension_id(""));
        assert!(!is_valid_extension_id("abc"));
        let short: String = std::iter::repeat('a').take(EXT_ID_LEN - 1).collect();
        let long: String = std::iter::repeat('a').take(EXT_ID_LEN + 1).collect();
        assert!(!is_valid_extension_id(&short));
        assert!(!is_valid_extension_id(&long));
    }

    #[test]
    fn rejects_shapes_the_old_loose_check_allowed() {
        // Regresi C2: semua ini LOLOS validasi lama ("≥20 alfanumerik") padahal
        // bukan ID Chrome, sehingga bisa menyusup ke allowed_origins.
        assert!(!is_valid_extension_id("ABCDEFGHIJKLMNOPQRST")); // 20 alfanumerik
        assert!(!is_valid_extension_id("aaaaaaaaaaaaaaaaaaaa")); // 20 huruf a
        // wildcard / URL bukan ID — build.sh lama pernah memasang wildcard (K2)
        assert!(!is_valid_extension_id("chrome-extension://*/"));
    }

    // ── v2.9.4 (C2): push_registered_id — dedup, LRU, cap ──

    #[test]
    fn push_appends_and_dedups() {
        let mut got: Vec<String> = Vec::new();
        assert!(push_registered_id(&mut got, "aaa"), "entri pertama = baru");
        assert!(push_registered_id(&mut got, "bbb"), "entri kedua = baru");
        // Duplikat tidak boleh menumpuk, dan BUKAN penambahan baru — inilah
        // yang membuat register ulang (background.js retry, browser restart)
        // tidak memunculkan notifikasi "ekstensi baru" berulang kali (C2).
        assert!(!push_registered_id(&mut got, "aaa"));
        assert_eq!(got, vec!["bbb".to_string(), "aaa".to_string()]);
    }

    #[test]
    fn push_moves_existing_to_newest() {
        // LRU: ID yang di-register ulang digeser ke paling baru supaya tidak
        // ter-evict lebih dulu padahal masih aktif dipakai.
        let mut got = ids(MAX_REGISTERED_IDS, "id");
        assert!(!push_registered_id(&mut got, "id0"), "refresh LRU bukan ID baru");
        assert_eq!(got.len(), MAX_REGISTERED_IDS, "tidak boleh tumbuh");
        assert_eq!(got[0], "id1");
        assert_eq!(got[MAX_REGISTERED_IDS - 1], "id0");
    }

    #[test]
    fn push_evicts_oldest_beyond_cap() {
        let total = MAX_REGISTERED_IDS + 3;
        let mut got: Vec<String> = Vec::new();
        for i in 0..total {
            let id = format!("id{i}");
            assert!(push_registered_id(&mut got, &id), "{id} harus dilaporkan baru");
        }
        assert_eq!(got.len(), MAX_REGISTERED_IDS, "cap wajib ditegakkan");
        // Tiga entri TERLAMA terbuang; sisanya tetap urutan kronologis.
        assert_eq!(got.first().map(String::as_str), Some("id3"));
        let newest = format!("id{}", total - 1);
        assert_eq!(got.last().map(String::as_str), Some(newest.as_str()));
    }

    #[test]
    fn push_cap_survives_many_registrations() {
        // Simulasi proses lokal mendaftarkan banyak ID (serangan persistensi
        // lokal yang disebut CODE-REVIEW §C2) — registry harus tetap terbatas.
        let mut got: Vec<String> = Vec::new();
        for i in 0..(MAX_REGISTERED_IDS * 10) {
            push_registered_id(&mut got, &format!("id{i}"));
        }
        assert_eq!(got.len(), MAX_REGISTERED_IDS);
    }

    #[test]
    fn push_reports_evicted_id_as_new_again() {
        // C2: ID yang ter-evict sudah kehilangan akses, jadi bila ia mendaftar
        // lagi itu BENAR-BENAR origin baru dan layak diumumkan ke user.
        let mut got: Vec<String> = Vec::new();
        assert!(push_registered_id(&mut got, "id0"));
        for i in 1..=MAX_REGISTERED_IDS {
            push_registered_id(&mut got, &format!("id{i}"));
        }
        assert!(!got.iter().any(|x| x == "id0"), "id0 harus ter-evict");
        assert!(push_registered_id(&mut got, "id0"), "kembali = baru lagi");
    }

    // ── v2.9.4 (C2): pengumuman origin baru ──

    #[test]
    fn packed_ext_id_is_not_a_new_origin() {
        // `make_origins` SELALU memasang EXT_ID di posisi pertama apa pun isi
        // registry, jadi register atas ID bawaan tidak memberi akses baru kepada
        // siapa pun dan tidak perlu diumumkan (noise pada pemasangan normal).
        assert!(!is_newly_granted_origin(EXT_ID.trim()));
        // EXT_ID berasal dari include_str! sehingga membawa newline di ujung.
        assert!(!is_newly_granted_origin(EXT_ID));
        assert!(!is_newly_granted_origin(&format!(" {} ", EXT_ID.trim())));
    }

    #[test]
    fn any_other_valid_id_is_a_new_origin() {
        let other = "abcdefghijklmnopabcdefghijklmnop";
        assert!(is_valid_extension_id(other), "prasyarat: bentuk ID valid");
        assert_ne!(other, EXT_ID.trim(), "prasyarat: bukan ID bawaan");
        assert!(is_newly_granted_origin(other));
    }

    #[test]
    fn notice_names_the_id_and_how_to_revoke() {
        let id = "abcdefghijklmnopabcdefghijklmnop";
        let (summary, body) = new_origin_notice(id);
        assert!(!summary.is_empty(), "notifikasi butuh ringkasan");
        assert!(body.contains(id), "body harus menyebut ID: {body}");
        assert!(body.contains(REGISTRY_FILE), "body harus menyebut cara cabut: {body}");
    }

    // ── make_origins ──

    #[test]
    fn make_origins_always_includes_packed_id_first() {
        let got = make_origins(&[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], format!("chrome-extension://{}/", EXT_ID.trim()));
    }

    #[test]
    fn make_origins_appends_registered_and_dedups_packed() {
        let packed = EXT_ID.trim().to_string();
        let registered = vec!["aaa".to_string(), packed.clone(), "bbb".to_string()];
        let want = vec![
            format!("chrome-extension://{packed}/"),
            "chrome-extension://aaa/".to_string(),
            "chrome-extension://bbb/".to_string(),
        ];
        assert_eq!(make_origins(&registered), want);
    }

    #[test]
    fn make_origins_never_emits_wildcard() {
        // K2: build.sh lama memasang "chrome-extension://*/*" sehingga extension
        // APA PUN di browser user bisa memanggil native host. Jangan pernah lagi.
        for origin in make_origins(&["aaa".to_string(), "bbb".to_string()]) {
            assert!(!origin.contains('*'), "origin wildcard: {origin}");
            assert!(origin.starts_with("chrome-extension://"));
            assert!(origin.ends_with('/'));
        }
    }
}
