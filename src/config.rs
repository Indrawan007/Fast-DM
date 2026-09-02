use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
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
    /// v2.3.0 (K5): unduhan tertunda hasil restore sesi dilanjutkan otomatis
    /// saat aplikasi dibuka. Matikan bila lebih suka memutuskan manual.
    #[serde(default = "default_true")]
    pub auto_resume: bool,
}

fn default_true() -> bool {
    true
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
            auto_resume: true,
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("fast-dm")
    }

    /// XDG_RUNTIME_DIR hanya diterima bila benar-benar aman: absolute,
    /// milik UID kita, dan tidak bisa diakses group/other (0700 ala systemd).
    /// Kalau tidak valid → None (pemanggil fallback ke config dir).
    fn validated_runtime_dir() -> Option<PathBuf> {
        use std::os::unix::fs::MetadataExt;
        let raw = std::env::var_os("XDG_RUNTIME_DIR")?;
        let p = PathBuf::from(&raw);
        if !p.is_absolute() {
            return None;
        }
        let md = fs::metadata(&p).ok()?;
        if md.uid() != nix::unistd::geteuid().as_raw() {
            return None;
        }
        if md.mode() & 0o077 != 0 {
            return None;
        }
        Some(p)
    }

    /// Buat (atau perketat) direktori privat per-user. Best-effort chmod 0700
    /// — bila direktori sudah ada milik orang lain, set_permissions gagal dan
    /// konten tetap dilindungi 0600 per file.
    fn ensure_private_dir(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::create_dir_all(dir);
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        dir.to_path_buf()
    }

    /// v2.3.0 (K1): basis untuk socket IPC & file sementara. Dulu langsung di
    /// `/tmp/fast-dm-<uid>.sock` — user lain bisa pre-create socket di path
    /// publik yang predictable dan menerima kirian URL + cookies. Sekarang
    /// `XDG_RUNTIME_DIR/fast-dm` (0700, per-user, tmpfs), fallback ke
    /// `~/.config/fast-dm/run`.
    pub fn work_dir() -> PathBuf {
        match Self::validated_runtime_dir() {
            Some(rt) => Self::ensure_private_dir(&rt.join("fast-dm")),
            None => Self::ensure_private_dir(&Self::config_dir().join("run")),
        }
    }

    /// Path socket IPC — dipakai GUI (bind) dan native host (connect).
    pub fn ipc_socket_path() -> PathBuf {
        Self::work_dir().join("fast-dm.sock")
    }

    /// Direktori input-file aria2 (berisi URL yang mungkin bertoken) — 0600/0700 (K3).
    pub fn aria2_input_dir() -> PathBuf {
        Self::ensure_private_dir(&Self::work_dir().join("aria2-in"))
    }

    /// Hapus file cookie `cookies_*.txt` yang lebih tua dari `max_age`.
    /// Cookie yang ditulis extension punya TTL 24 jam — yang menua tak akan
    /// pernah dipakai lagi (yt-dlp menolak > 2 jam), jadi buang saja.
    pub fn gc_stale_cookies() -> usize {
        Self::gc_cookie_files_in(&Self::config_dir(), std::time::Duration::from_secs(7 * 24 * 3600))
    }

    fn gc_cookie_files_in(dir: &Path, max_age: std::time::Duration) -> usize {
        let mut removed = 0;
        let Ok(rd) = fs::read_dir(dir) else {
            return 0;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let is_cookie = path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("cookies_") && n.ends_with(".txt"));
            if !is_cookie {
                continue;
            }
            let stale = path
                .metadata()
                .ok()
                .and_then(|md| md.modified().ok())
                .and_then(|m| m.elapsed().ok())
                .map_or(false, |age| age > max_age);
            if stale && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
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
        assert!(c.auto_resume); // K5: default lanjutkan restore otomatis
    }

    // ── v2.3.0: path privat (K1/K3) ──

    #[test]
    fn ipc_socket_path_is_named_and_absolute() {
        let p = Config::ipc_socket_path();
        assert!(p.is_absolute(), "socket path harus absolute: {p:?}");
        assert!(p.ends_with("fast-dm.sock"));
        // work_dir = runtime dir (0700) atau config dir — bukan /tmp telanjang
        // yang bisa di-preempt user lain (K1).
        assert_ne!(p.parent().unwrap(), Path::new("/tmp"));
    }

    #[test]
    fn aria2_input_dir_is_private_subdir() {
        let d = Config::aria2_input_dir();
        assert!(d.exists());
        assert!(d.ends_with("aria2-in"));
    }

    #[test]
    fn gc_cookie_files_removes_only_old_cookie_files() {
        use std::io::Write;
        use std::time::{Duration, UNIX_EPOCH};

        let tmp = std::env::temp_dir().join(format!("fast-dm-gc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let fresh = tmp.join("cookies_fresh.com.txt");
        let old = tmp.join("cookies_old.com.txt");
        let keep = tmp.join("config.json");
        for p in [&fresh, &old, &keep] {
            let mut f = fs::File::create(p).unwrap();
            f.write_all(b"# Netscape HTTP Cookie File\n").unwrap();
        }
        // Mundur mtime file "old" ke jaman UNIX_EPOCH+1000s (pasti > 1 jam)
        let ft = std::fs::FileTimes::new()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_000));
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(ft)
            .unwrap();

        let removed = Config::gc_cookie_files_in(&tmp, Duration::from_secs(3600));
        assert_eq!(removed, 1, "hanya cookie lama yang dibuang");
        assert!(fresh.exists(), "cookie fresh harus tetap ada");
        assert!(!old.exists(), "cookie basi harus dibuang");
        assert!(keep.exists(), "bukan file cookie tidak boleh disentuh");

        let _ = fs::remove_dir_all(&tmp);
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

