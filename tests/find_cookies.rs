//! Integration test untuk `Config::find_cookies_file` — pakai tempdir
//! + override `XDG_CONFIG_HOME` agar test tidak touch `~/.config` user.
//!
//! Test ini **TIDAK** bisa jalan paralel dengan test lain yang baca
//! `XDG_CONFIG_HOME` (vars_env adalah process-global). Solusi:
//! - `cargo test -- --test-threads=1` (sequential)
//! - atau pastikan test ini satu-satunya yang override env (saat ini ya).
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
//! Keuntungan: build lebih cepat, tidak ada risiko dependency issue.

use fast_dm::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
fn redirect_config_to(tmp: &Path) {
    // SAFETY: Set_var aman di sini karena test runner single-threaded
    // untuk test ini. Kalau ada race dengan test lain, pindahkan
    // dependency ke `serial_test` crate.
    std::env::set_var("XDG_CONFIG_HOME", tmp);
    // dirs::config_dir() di Linux cek $XDG_CONFIG_HOME duluan
}

#[cfg(not(target_os = "linux"))]
fn redirect_config_to(_tmp: &Path) {
    // Skip di platform non-Linux — dirs::config_dir() pakai env berbeda
    // yang tidak di-override di sini.
}

/// Counter global untuk nama tempdir unik antar test.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bikin tempdir unik + return path. Caller WAJIB panggil `cleanup_tempdir`
/// di akhir test (tidak ada Drop otomatis — pure stdlib).
///
/// Pattern: `{temp_dir}/fast-dm-test-{pid}-{counter}-{nano_ts}`
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

/// Hapus tempdir + semua isinya. Best-effort: error diabaikan.
fn cleanup_tempdir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Helper: hitung path cookie file via public API.
fn cookie_path(host: &str) -> PathBuf {
    Config::cookies_file_for(host)
}

/// Helper: tulis file kosong (header Netscape) di path yang diberikan.
/// Auto-create parent dir kalau belum ada.
fn write_cookie_file(path: &Path) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(path, "# Netscape HTTP Cookie File\n").unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn walks_up_to_parent_domain_when_child_missing() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    let parent = cookie_path("example.com");
    let child = cookie_path("cdn.example.com");

    // Sanity: file belum ada
    assert!(!parent.exists());
    assert!(!child.exists());

    // Setup: tulis hanya file parent
    write_cookie_file(&parent);
    assert!(parent.exists());

    // Lookup "cdn.example.com" → harus naik ke "example.com"
    let found = Config::find_cookies_file("cdn.example.com");
    assert_eq!(
        found,
        Some(parent.clone()),
        "harus naik ke parent domain (sub.example.com → example.com)"
    );

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn exact_match_takes_priority_over_parent() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    let exact = cookie_path("api.example.com");
    let parent = cookie_path("example.com");

    // Tulis KEDUA file
    write_cookie_file(&exact);
    write_cookie_file(&parent);

    // Lookup "api.example.com" → harus pilih exact, BUKAN parent
    let found = Config::find_cookies_file("api.example.com");
    assert_eq!(
        found,
        Some(exact.clone()),
        "exact match harus diprioritaskan, parent sebagai fallback"
    );

    // Lookup "other.example.com" → exact tidak ada, harus parent
    let found = Config::find_cookies_file("other.example.com");
    assert_eq!(
        found,
        Some(parent.clone()),
        "host dengan subdomain berbeda harus pakai parent domain"
    );

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn walks_up_multiple_levels() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    // a.b.example.com → b.example.com → example.com
    let top = cookie_path("example.com");
    let sub = cookie_path("b.example.com");
    let deep = cookie_path("a.b.example.com");

    // Hanya tulis top-level
    write_cookie_file(&top);
    assert!(!sub.exists());
    assert!(!deep.exists());

    // Lookup deep → harus naik 2 level
    let found = Config::find_cookies_file("a.b.example.com");
    assert_eq!(
        found,
        Some(top.clone()),
        "harus naik 2 level parent (a.b.example.com → b.example.com → example.com)"
    );

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn returns_none_when_no_cookie_anywhere() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    // Domain fiktif yang PASTI tidak punya file cookie di tempdir ini
    let found = Config::find_cookies_file(
        "totally-nonexistent-domain-xxx-unique-9999.invalid",
    );
    assert!(found.is_none());

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn single_label_host_returns_none_without_loop() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    // "localhost" tidak punya '.' → loop walk-up langsung exit
    // PENTING: test ini memastikan tidak ada infinite loop
    let found = Config::find_cookies_file("localhost");
    assert!(found.is_none());

    // "intranet" juga single-label
    let found = Config::find_cookies_file("intranet");
    assert!(found.is_none());

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn www_prefix_normalized() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    // File ditulis untuk "example.com" (no www)
    let no_www = cookie_path("example.com");
    write_cookie_file(&no_www);

    // Lookup "www.example.com" → harus normalize ke "example.com"
    let found = Config::find_cookies_file("www.example.com");
    assert_eq!(
        found,
        Some(no_www.clone()),
        "'www.' prefix harus di-strip sebelum lookup"
    );

    // Cleanup
    cleanup_tempdir(&tmp);
}

#[cfg(target_os = "linux")]
#[test]
fn case_insensitive_normalization() {
    let tmp = make_tempdir();
    redirect_config_to(&tmp);

    // File ditulis lowercase
    let lower = cookie_path("example.com");
    write_cookie_file(&lower);

    // Lookup UPPERCASE → harus normalize
    let found = Config::find_cookies_file("EXAMPLE.COM");
    assert_eq!(
        found,
        Some(lower.clone()),
        "lookup harus case-insensitive"
    );

    // Cleanup
    cleanup_tempdir(&tmp);
}

// Di platform non-Linux, sediakan stub test agar test count tidak kosong
#[cfg(not(target_os = "linux"))]
#[test]
fn find_cookies_skipped_on_non_linux() {
    // Test ini placeholder — logika find_cookies_file adalah cross-platform
    // (tidak ada syscall spesifik Linux), tapi override XDG_CONFIG_HOME
    // hanya relevan di Linux. Test riil di unit test (config.rs) sudah
    // cover pure-function path generation.
    eprintln!("find_cookies_file integration test di-skip di platform ini");
}
