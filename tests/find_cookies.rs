//! Integration test untuk `Config::find_cookies_file` — pakai tempdir
//! + override `XDG_CONFIG_HOME` agar test tidak touch `~/.config` user.
//!
//! **Concurrency note**: `XDG_CONFIG_HOME` adalah process-global env var.
//! Test dalam file ini **harus serial** (satu per satu). Kita pakai
//! `Mutex` global + `lock()` di awal setiap test untuk memaksa urutan.
//! Tanpa ini, test paralel bisa overwrite `XDG_CONFIG_HOME` satu sama
//! lain dan `Config::config_dir()` (yang baca env per-call) bisa return
//! path yang salah.
//!
//! Sesuai `Config::config_dir()`:
//!   Linux: `XDG_CONFIG_HOME` or `~/.config` → join("fast-dm")
//!   macOS: `~/Library/Application Support` → join("fast-dm")
//!   Windows: `%APPDATA%` → join("fast-dm")
//!
//! Kita hanya set `XDG_CONFIG_HOME` — di macOS/Windows `dirs` crate pakai
//! env lain, jadi test ini di-skip di platform tersebut via #[cfg].
//!
//! **Tidak butuh crate eksternal** — pakai `std::env::temp_dir()` saja.

use fast_dm::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Global mutex untuk serialize test yang override `XDG_CONFIG_HOME`.
/// Setiap test WAJIB acquire lock ini di awal. Test yang lupa lock
/// akan kena race condition — sengaja di-design ketat.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Counter global untuk nama tempdir unik.
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "linux")]
fn redirect_config_to(tmp: &Path) {
    std::env::set_var("XDG_CONFIG_HOME", tmp);
    // dirs::config_dir() di Linux cek $XDG_CONFIG_HOME duluan
}

#[cfg(not(target_os = "linux"))]
fn redirect_config_to(_tmp: &Path) {
    // Skip di platform non-Linux.
}

/// Bikin tempdir unik. Caller WAJIB panggil `cleanup_tempdir` di akhir test.
fn make_tempdir() -> PathBuf {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nano = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir()
        .join(format!("fast-dm-test-{}-{}-{}", pid, counter, nano));
    fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Hapus tempdir + semua isinya. Best-effort.
fn cleanup_tempdir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Helper: hitung path cookie file via public API.
/// **Hanya boleh dipanggil SETELAH lock + redirect_config_to**, karena
/// path bergantung pada XDG_CONFIG_HOME saat ini.
fn cookie_path(host: &str) -> PathBuf {
    Config::cookies_file_for(host)
}

/// Helper: tulis file cookie kosong.
fn write_cookie_file(path: &Path) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(path, "# Netscape HTTP Cookie File\n").unwrap();
}

// ────────────────────────────────────────────────────────────────
// Test body helper — wrap pattern: lock → set env → run test → cleanup
// ────────────────────────────────────────────────────────────────

/// RAII guard: restore `XDG_CONFIG_HOME` ke nilai sebelumnya saat drop.
///
/// **Field order matters**: di Rust, field di-drop dalam reverse order
/// dari declaration. Kita declare `_lock` **terakhir** agar dia di-drop
/// **pertama** (= mutex di-release). Lalu `Drop` impl jalan (env
/// restored). Urutan: restore env → release lock.
struct EnvGuard {
    /// Env var lama — di-restore di Drop impl.
    previous: Option<std::ffi::OsString>,
    /// Lock guard — HARUS declare terakhir agar di-drop pertama.
    /// Saat di-drop, mutex di-release SEBELUM `Drop` impl jalan,
    /// jadi test berikutnya tidak akan mulai sampai env var selesai
    /// di-restore.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn new(tmp: &Path) -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner()); // recover dari poison
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        redirect_config_to(tmp);
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // Restore env var ke nilai sebelumnya. Lock akan di-release
        // otomatis setelah Drop selesai (karena _lock field di-declare
        // terakhir → di-drop terakhir? Lihat comment di struct).
        //
        // Tunggu — Rust drop order REVERSE dari declaration:
        //   struct { previous, _lock }
        //   → _lock di-drop DULU (declare terakhir → drop pertama)
        //   → previous di-drop KEMUDIAN
        //
        // Itu BERARTI mutex di-release SEBELUM kita restore env.
        // Test berikutnya bisa masuk critical section dan baca env
        // yang belum di-restore. Untuk itu kita restore env SECARA
        // MANUAL di sini (tidak rely on field drop), SEBELUM lock
        // dilepas oleh _lock field.
        //
        // Setelah baris ini, env var kembali ke nilai original.
        // Lalu _lock field di-drop → mutex released → test lain jalan
        // dengan env yang sudah benar.
        match &self.previous {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        // _lock drop secara otomatis setelah return dari Drop::drop,
        // melepas mutex. Env var SUDAH di-restore di atas.
    }
}

// ────────────────────────────────────────────────────────────────
// Tests — setiap #[test] HARUS mulai dengan `let _env = EnvGuard::new(&tmp);`
// ────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn walks_up_to_parent_domain_when_child_missing() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp); // serial + set XDG_CONFIG_HOME

    let parent = cookie_path("example.com");
    let child = cookie_path("cdn.example.com");

    assert!(!parent.exists());
    assert!(!child.exists());

    write_cookie_file(&parent);
    assert!(parent.exists());

    let found = Config::find_cookies_file("cdn.example.com");
    assert_eq!(
        found,
        Some(parent.clone()),
        "harus naik ke parent domain (sub.example.com → example.com)"
    );

    cleanup_tempdir(&tmp);
    // _env di-drop di sini → restore XDG_CONFIG_HOME + release lock
}

#[cfg(target_os = "linux")]
#[test]
fn exact_match_takes_priority_over_parent() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    let exact = cookie_path("api.example.com");
    let parent = cookie_path("example.com");

    write_cookie_file(&exact);
    write_cookie_file(&parent);

    // Lookup 1: exact match
    let found = Config::find_cookies_file("api.example.com");
    assert_eq!(
        found,
        Some(exact.clone()),
        "exact match harus diprioritaskan, parent sebagai fallback"
    );

    // Lookup 2: subdomain berbeda → harus naik ke parent
    let found = Config::find_cookies_file("other.example.com");
    assert_eq!(
        found,
        Some(parent.clone()),
        "host dengan subdomain berbeda harus pakai parent domain"
    );

    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn walks_up_multiple_levels() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    // a.b.example.com → b.example.com → example.com
    let top = cookie_path("example.com");
    let sub = cookie_path("b.example.com");
    let deep = cookie_path("a.b.example.com");

    write_cookie_file(&top);
    assert!(!sub.exists());
    assert!(!deep.exists());

    let found = Config::find_cookies_file("a.b.example.com");
    assert_eq!(
        found,
        Some(top.clone()),
        "harus naik 2 level parent (a.b.example.com → b.example.com → example.com)"
    );

    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn returns_none_when_no_cookie_anywhere() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    let found = Config::find_cookies_file(
        "totally-nonexistent-domain-xxx-unique-9999.invalid",
    );
    assert!(found.is_none());

    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn single_label_host_returns_none_without_loop() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    // "localhost" tidak punya '.' → loop walk-up langsung exit
    let found = Config::find_cookies_file("localhost");
    assert!(found.is_none());

    let found = Config::find_cookies_file("intranet");
    assert!(found.is_none());

    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn www_prefix_normalized() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    let no_www = cookie_path("example.com");
    write_cookie_file(&no_www);

    let found = Config::find_cookies_file("www.example.com");
    assert_eq!(
        found,
        Some(no_www.clone()),
        "'www.' prefix harus di-strip sebelum lookup"
    );

    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn case_insensitive_normalization() {
    let tmp = make_tempdir();
    let _env = EnvGuard::new(&tmp);

    let lower = cookie_path("example.com");
    write_cookie_file(&lower);

    let found = Config::find_cookies_file("EXAMPLE.COM");
    assert_eq!(
        found,
        Some(lower.clone()),
        "lookup harus case-insensitive"
    );

    cleanup_tempdir(&tmp);
}

// Di platform non-Linux, sediakan stub test.
#[cfg(not(target_os = "linux"))]
#[test]
fn find_cookies_skipped_on_non_linux() {
    eprintln!("find_cookies_file integration test di-skip di platform ini");
}
